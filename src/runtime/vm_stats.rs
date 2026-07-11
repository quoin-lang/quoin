//! `VM` — runtime self-introspection: the AOT coverage counters the codegen
//! module keeps process-wide, plus the compute-offload pool's counters:
//!
//! - `VM.stats` -> a Map of sections (only `'aot'` today, shaped so `'gc'` /
//!   `'dispatch'` can join later without breaking callers):
//!   `#{ 'aot': #{ 'compiled': n 'refused': n 'skipped': n 'reasons': #{ kind: count } } }`
//! - `VM.aotRefusals` -> the drill-down: a List of
//!   `#{ 'selector': s 'kind': k 'reason': why }`, one per distinct refusal/skip.
//!
//! Semantics: `compiled` counts translation EVENTS (`codegen::compile_totals` —
//! speculative members recompile); `refused`/`skipped` count DISTINCT members
//! from the deduplicated refusal log (`codegen::refusal_snapshot`), split by
//! translator refusal vs candidacy pre-check. The log is bounded
//! (`REFUSAL_LOG_CAP`), so a very long session may undercount — counters, not
//! ledgers. Block-template skips appear under the pseudo-selector
//! `block@<template-id>` (a block literal has no user-facing name).

use indexmap::IndexMap;

use crate::codegen;
use crate::value::{NativeClassBuilder, Value};
use crate::vm::VmState;

/// The `'aot'` section of `VM.stats`.
fn aot_section<'gc>(vm: &VmState<'gc>, mc: &gc_arena::Mutation<'gc>) -> Value<'gc> {
    let (compiled, _refused_events) = codegen::compile_totals();
    let records = codegen::refusal_snapshot();

    let mut refused = 0i64;
    let mut skipped = 0i64;
    let mut reasons: IndexMap<String, i64> = IndexMap::new();
    for r in &records {
        if r.kind.is_precheck() {
            skipped += 1;
        } else {
            refused += 1;
        }
        *reasons.entry(r.kind.name().to_string()).or_insert(0) += 1;
    }

    let mut reasons_map = Vec::new();
    for (k, n) in reasons {
        reasons_map.push((k, vm.new_int(mc, n)));
    }

    let mut aot = Vec::new();
    aot.push((
        "enabled".to_string(),
        vm.new_bool(mc, crate::tuning::aot_enabled()),
    ));
    aot.push(("compiled".to_string(), vm.new_int(mc, compiled as i64)));
    aot.push((
        "entryBails".to_string(),
        vm.new_int(mc, codegen::entry_bails() as i64),
    ));
    aot.push((
        "retranslated".to_string(),
        vm.new_int(
            mc,
            codegen::TOTAL_RETRANSLATED.load(std::sync::atomic::Ordering::Relaxed) as i64,
        ),
    ));
    aot.push((
        "directSites".to_string(),
        vm.new_int(
            mc,
            codegen::TOTAL_DIRECT_SITES.load(std::sync::atomic::Ordering::Relaxed) as i64,
        ),
    ));
    aot.push((
        "retranslateMs".to_string(),
        vm.new_int(
            mc,
            (codegen::RETRANSLATE_NS.load(std::sync::atomic::Ordering::Relaxed) / 1_000_000) as i64,
        ),
    ));
    aot.push(("refused".to_string(), vm.new_int(mc, refused)));
    aot.push(("skipped".to_string(), vm.new_int(mc, skipped)));
    aot.push(("reasons".to_string(), vm.new_map(mc, reasons_map)));
    vm.new_map(mc, aot)
}

/// The `'compute'` section: the offload pool's counters (submitted /
/// completed jobs, plus gated ops that ran inline — below `QN_COMPUTE_MIN`
/// or with the pool disabled).
fn compute_section<'gc>(vm: &VmState<'gc>, mc: &gc_arena::Mutation<'gc>) -> Value<'gc> {
    let (submitted, completed, inline) = crate::compute::stats();
    let mut m = Vec::new();
    m.push(("submitted".to_string(), vm.new_int(mc, submitted as i64)));
    m.push(("completed".to_string(), vm.new_int(mc, completed as i64)));
    m.push(("inline".to_string(), vm.new_int(mc, inline as i64)));
    m.push((
        "threads".to_string(),
        vm.new_int(mc, crate::compute::threads() as i64),
    ));
    vm.new_map(mc, m)
}

/// The `'workers'` section: isolate counters (spawned / completed threads,
/// cross-worker messages copied).
fn workers_section<'gc>(vm: &VmState<'gc>, mc: &gc_arena::Mutation<'gc>) -> Value<'gc> {
    let (spawned, completed, messages) = crate::worker::stats();
    let mut m = Vec::new();
    m.push(("spawned".to_string(), vm.new_int(mc, spawned as i64)));
    m.push(("completed".to_string(), vm.new_int(mc, completed as i64)));
    m.push(("messages".to_string(), vm.new_int(mc, messages as i64)));
    vm.new_map(mc, m)
}

/// One task row of the `VM.ps` snapshot (shared by the guest method and the
/// REPL's `$ps` table).
pub(crate) struct PsTaskRow {
    pub id: usize,
    pub state: &'static str,
    pub fibers: usize,
    pub on: Option<String>,
    /// `(cap, buffered, recv_waiters, send_waiters)` of the channel this
    /// task is parked on, read LIVE through its park subject.
    pub channel: Option<(usize, usize, usize, usize)>,
    pub parent: Option<usize>,
    pub awaiting: Vec<usize>,
}

pub(crate) struct PsWorkerRow {
    pub id: usize,
    pub unit: String,
    pub label: String,
    pub backing: &'static str,
    pub pid: Option<u32>,
    pub running: bool,
    pub inbox: usize,
    pub outbox: usize,
}

pub(crate) struct PsData {
    pub is_worker: bool,
    pub tasks: Vec<PsTaskRow>,
    pub workers: Vec<PsWorkerRow>,
    pub io_in_flight: usize,
    pub compute_in_flight: usize,
}

/// Render `PsData` as plain wire data (the §13.3 control-lane reply and the
/// building block `VM.psTree` assembles). Worker rows carry an empty 'ps'
/// slot the collector patches with each child's subtree.
pub(crate) fn ps_to_wire(data: &PsData) -> quoin_ext_proto::DataValue {
    use quoin_ext_proto::DataValue as W;
    let tasks: Vec<W> = data
        .tasks
        .iter()
        .map(|t| {
            let mut m = vec![
                ("id".to_string(), W::Int(t.id as i64)),
                ("state".to_string(), W::Str(t.state.to_string())),
                ("fibers".to_string(), W::Int(t.fibers as i64)),
            ];
            if let Some(on) = &t.on {
                m.push(("on".to_string(), W::Str(on.clone())));
            }
            if let Some((cap, buffered, recv, send)) = t.channel {
                m.push((
                    "channel".to_string(),
                    W::Map(vec![
                        ("cap".to_string(), W::Int(cap as i64)),
                        ("buffered".to_string(), W::Int(buffered as i64)),
                        ("recvWaiters".to_string(), W::Int(recv as i64)),
                        ("sendWaiters".to_string(), W::Int(send as i64)),
                    ]),
                ));
            }
            if let Some(p) = t.parent {
                m.push(("parent".to_string(), W::Int(p as i64)));
            }
            if !t.awaiting.is_empty() {
                m.push((
                    "awaiting".to_string(),
                    W::List(t.awaiting.iter().map(|c| W::Int(*c as i64)).collect()),
                ));
            }
            W::Map(m)
        })
        .collect();
    let workers: Vec<W> = data
        .workers
        .iter()
        .map(|w| {
            W::Map(vec![
                ("id".to_string(), W::Int(w.id as i64)),
                ("unit".to_string(), W::Str(w.unit.clone())),
                ("label".to_string(), W::Str(w.label.clone())),
                ("backing".to_string(), W::Str(w.backing.to_string())),
                (
                    "pid".to_string(),
                    w.pid.map_or(W::Null, |p| W::Int(p as i64)),
                ),
                (
                    "state".to_string(),
                    W::Str(if w.running { "running" } else { "exited" }.to_string()),
                ),
                ("inbox".to_string(), W::Int(w.inbox as i64)),
                ("outbox".to_string(), W::Int(w.outbox as i64)),
            ])
        })
        .collect();
    W::Map(vec![
        ("worker?".to_string(), W::Bool(data.is_worker)),
        ("tasks".to_string(), W::List(tasks)),
        ("workers".to_string(), W::List(workers)),
        (
            "io".to_string(),
            W::Map(vec![(
                "inFlight".to_string(),
                W::Int(data.io_in_flight as i64),
            )]),
        ),
        (
            "compute".to_string(),
            W::Map(vec![(
                "inFlight".to_string(),
                W::Int(data.compute_in_flight as i64),
            )]),
        ),
    ])
}

/// Snapshot the scheduler/worker state as plain rows. Read-only.
/// `vm.sched.current_task` is trusted as the running task — callers that
/// know better (the DRIVER answering a control request between resumes,
/// when nothing is running) use [`ps_data_with_current`].
pub(crate) fn ps_data<'gc>(vm: &VmState<'gc>) -> PsData {
    ps_data_with_current(vm, Some(vm.sched.current_task))
}

pub(crate) fn ps_data_with_current<'gc>(
    vm: &VmState<'gc>,
    current: Option<crate::vm_scheduler::TaskId>,
) -> PsData {
    let mut tasks = Vec::new();
    let mut io_in_flight = 0;
    for (id, slot) in vm.sched.tasks.iter().enumerate() {
        let Some(t) = slot else { continue };
        let running = current.is_some_and(|c| id == c.0);
        let state = if running {
            "running"
        } else if vm.sched.ready.iter().any(|r| r.0 == id) {
            "ready"
        } else {
            "parked"
        };
        if t.abort_handle.is_some() {
            io_in_flight += 1;
        }
        // The RUNNING task's fiber chain lives in the scheduler slots; a
        // parked task carries its own.
        let fibers = if running {
            vm.sched.resume_stack.iter().flatten().count()
                + usize::from(vm.sched.current_fiber.is_some())
        } else {
            t.resume_stack.iter().flatten().count() + usize::from(t.current_fiber.is_some())
        };
        let on = if state == "parked" {
            t.park_label.clone()
        } else {
            None
        };
        let channel = if state == "parked" {
            t.park_subject.and_then(|subject| {
                subject
                    .with_native_state::<crate::runtime::channel::NativeChannelState, _, _>(|ch| {
                        (
                            ch.cap,
                            ch.buffer.len(),
                            ch.recv_waiters.len(),
                            ch.send_waiters.len(),
                        )
                    })
                    .ok()
            })
        } else {
            None
        };
        tasks.push(PsTaskRow {
            id,
            state,
            fibers,
            on,
            channel,
            parent: t.parent.map(|(p, _)| p.0),
            awaiting: Vec::new(),
        });
    }
    // A gather parent's children point AT it; invert that into `awaiting`.
    let child_parents: Vec<(usize, usize)> = vm
        .sched
        .tasks
        .iter()
        .enumerate()
        .filter_map(|(id, t)| Some((id, t.as_ref()?.parent?.0.0)))
        .collect();
    for row in tasks.iter_mut() {
        row.awaiting = child_parents
            .iter()
            .filter(|(_, p)| *p == row.id)
            .map(|(c, _)| *c)
            .collect();
    }
    let workers = vm
        .worker_registry
        .iter()
        .enumerate()
        .map(|(id, w)| PsWorkerRow {
            id,
            unit: w.unit.clone(),
            label: w.label.clone(),
            backing: w.backing,
            pid: w.pid,
            running: !w.outbox_rx.is_closed(),
            inbox: w.inbox_tx.len(),
            outbox: w.outbox_rx.len(),
        })
        .collect();
    PsData {
        is_worker: vm.worker_link.is_some(),
        tasks,
        workers,
        io_in_flight,
        compute_in_flight: crate::compute::in_flight(),
    }
}

pub fn build_vm_stats_class() -> NativeClassBuilder {
    NativeClassBuilder::new("VM", Some("Object"))
        .abstract_class()
        .class_doc(
            "The VM's self-introspection surface: `stats` (counters by section), \
             `aotRefusals` (which members stayed interpreted and why), and `ps` / `psTree` \
             (a live snapshot of tasks and workers as plain data).",
        )
        // `VM.stats` -> the section Map (see the module doc for the shape and
        // the events-vs-distinct-members counting semantics).
        // `VM.ps` — a live tree of the scheduler and workers as plain data
        // (Maps/Lists), so `.pp` renders it and programs can walk it. The
        // REPL's `$ps` shows the same snapshot as a table.
        .class_method("ps", |vm, mc, _receiver, _args| {
            let data = ps_data(vm);
            let mut root = Vec::new();
            root.push(("worker?".to_string(), vm.new_bool(mc, data.is_worker)));
            let tasks: Vec<Value> = data
                .tasks
                .iter()
                .map(|t| {
                    let mut m = Vec::new();
                    m.push(("id".to_string(), vm.new_int(mc, t.id as i64)));
                    m.push(("state".to_string(), vm.new_string(mc, t.state.to_string())));
                    m.push(("fibers".to_string(), vm.new_int(mc, t.fibers as i64)));
                    if let Some(on) = &t.on {
                        m.push(("on".to_string(), vm.new_string(mc, on.clone())));
                    }
                    if let Some((cap, buffered, recv, send)) = t.channel {
                        let mut c = Vec::new();
                        c.push(("cap".to_string(), vm.new_int(mc, cap as i64)));
                        c.push(("buffered".to_string(), vm.new_int(mc, buffered as i64)));
                        c.push(("recvWaiters".to_string(), vm.new_int(mc, recv as i64)));
                        c.push(("sendWaiters".to_string(), vm.new_int(mc, send as i64)));
                        m.push(("channel".to_string(), vm.new_map(mc, c)));
                    }
                    if let Some(p) = t.parent {
                        m.push(("parent".to_string(), vm.new_int(mc, p as i64)));
                    }
                    if !t.awaiting.is_empty() {
                        let ids: Vec<Value> = t
                            .awaiting
                            .iter()
                            .map(|c| vm.new_int(mc, *c as i64))
                            .collect();
                        m.push(("awaiting".to_string(), vm.new_list(mc, ids)));
                    }
                    vm.new_map(mc, m)
                })
                .collect();
            root.push(("tasks".to_string(), vm.new_list(mc, tasks)));
            let workers: Vec<Value> = data
                .workers
                .iter()
                .map(|w| {
                    let mut m = Vec::new();
                    m.push(("id".to_string(), vm.new_int(mc, w.id as i64)));
                    m.push(("unit".to_string(), vm.new_string(mc, w.unit.clone())));
                    m.push(("label".to_string(), vm.new_string(mc, w.label.clone())));
                    m.push((
                        "backing".to_string(),
                        vm.new_string(mc, w.backing.to_string()),
                    ));
                    if let Some(pid) = w.pid {
                        m.push(("pid".to_string(), vm.new_int(mc, pid as i64)));
                    }
                    m.push((
                        "state".to_string(),
                        vm.new_string(mc, if w.running { "running" } else { "exited" }.to_string()),
                    ));
                    m.push(("inbox".to_string(), vm.new_int(mc, w.inbox as i64)));
                    m.push(("outbox".to_string(), vm.new_int(mc, w.outbox as i64)));
                    vm.new_map(mc, m)
                })
                .collect();
            root.push(("workers".to_string(), vm.new_list(mc, workers)));
            let mut io = Vec::new();
            io.push((
                "inFlight".to_string(),
                vm.new_int(mc, data.io_in_flight as i64),
            ));
            root.push(("io".to_string(), vm.new_map(mc, io)));
            let mut comp = Vec::new();
            comp.push((
                "inFlight".to_string(),
                vm.new_int(mc, data.compute_in_flight as i64),
            ));
            root.push(("compute".to_string(), vm.new_map(mc, comp)));
            Ok(vm.new_map(mc, root))
        })
        .doc(
            "A live snapshot of the scheduler as plain data: a Map with 'worker?', 'tasks' \
             (id / state / fibers / park info per task, including the channel a parked task \
             waits on), 'workers' (one row per spawned worker), and 'io' / 'compute' \
             in-flight counts. Plain Maps and Lists, so `.pp` renders it and programs can \
             walk it; the REPL's `$ps` shows the same snapshot as a table.",
        )
        // `VM.psTree` — `VM.ps` plus each worker row's 'ps' filled with the
        // worker's OWN tree, recursively (docs/CONCURRENCY_ARCH.md §13.4):
        // one control request per worker (its driver answers between task
        // resumes), bounded deadline, 'unresponsive' for the silent.
        // Pull-based: the whole topology costs exactly one call.
        .class_method("psTree", |vm, mc, _receiver, _args| {
            let children: Vec<(
                usize,
                async_channel::Sender<crate::worker::ControlReq>,
                bool,
            )> = vm
                .worker_registry
                .iter()
                .enumerate()
                .map(|(i, w)| (i, w.control_tx.clone(), !w.outbox_rx.is_closed()))
                .collect();
            let mut subs: Vec<(usize, Option<quoin_ext_proto::DataValue>)> = Vec::new();
            for (idx, tx, running) in children {
                if !running {
                    subs.push((idx, None));
                    continue;
                }
                let (rtx, rrx) = async_channel::bounded(1);
                if tx
                    .try_send(crate::worker::ControlReq {
                        kind: crate::worker::ControlKind::PsTree,
                        reply: rtx,
                    })
                    .is_err()
                {
                    subs.push((idx, None));
                    continue;
                }
                let sub = match vm
                    .await_io(crate::io_backend::IoRequest::WorkerRecvTimed { rx: rrx, ms: 700 })?
                {
                    crate::io_backend::IoResult::WorkerMsg(Some(
                        crate::worker::WorkerMsg::Data(dv),
                    )) => Some(dv),
                    _ => None,
                };
                subs.push((idx, sub));
            }
            // Assemble as wire data (reusing the same patch shape the driver
            // uses), then convert ONCE into guest values.
            let data = ps_data(vm);
            let mut local = ps_to_wire(&data);
            if let quoin_ext_proto::DataValue::Map(sections) = &mut local {
                for (k, v) in sections.iter_mut() {
                    if k == "workers"
                        && let quoin_ext_proto::DataValue::List(rows) = v
                    {
                        for (idx, sub) in subs.drain(..) {
                            if let Some(quoin_ext_proto::DataValue::Map(row)) = rows.get_mut(idx) {
                                row.push((
                                    "ps".to_string(),
                                    match sub {
                                        Some(tree) => tree,
                                        None => quoin_ext_proto::DataValue::Str(
                                            "unresponsive".to_string(),
                                        ),
                                    },
                                ));
                            }
                        }
                    }
                }
            }
            crate::runtime::extension::wire_to_value(vm, mc, &local, None)
        })
        .doc(
            "`VM.ps` plus each worker row's 'ps' slot filled with that worker's OWN tree, \
             recursively -- the whole process topology in one call \
             (docs/CONCURRENCY_ARCH.md). Each worker answers between task resumes under a \
             bounded deadline; a silent one reads 'unresponsive'.",
        )
        // `VM.unit` — the ENTRY unit this VM runs (canonicalized path), nil
        // for REPL/eval. The same-unit provisioning primitive:
        // `Worker.spawn:(VM.unit)` runs another copy of this program.
        .class_method("unit", |vm, mc, _receiver, _args| {
            Ok(match &vm.unit_path {
                Some(p) => vm.new_string(mc, p.clone()),
                None => Value::Nil,
            })
        })
        .doc(
            "The canonicalized path of the entry unit this VM runs, or nil in the REPL / \
             `qn -e`. The same-program provisioning primitive: `Worker.spawn:(VM.unit)` \
             runs another copy of this program.",
        )
        .class_method("stats", |vm, mc, _receiver, _args| {
            let mut sections = Vec::new();
            sections.push(("aot".to_string(), aot_section(vm, mc)));
            sections.push(("compute".to_string(), compute_section(vm, mc)));
            sections.push(("workers".to_string(), workers_section(vm, mc)));
            Ok(vm.new_map(mc, sections))
        })
        .doc(
            "The VM's counters as a Map of sections: 'aot' (compiled / refused / skipped \
             plus per-reason counts), 'compute' (offload-pool jobs), 'workers' (isolates \
             spawned / completed, messages copied). `compiled` counts translation events; \
             `refused` / `skipped` count distinct members from a bounded log -- counters, \
             not ledgers.\n\n\
             ```\n\
             VM.stats.keys    \"* -> #(aot compute workers)\n\
             ```",
        )
        // `VM.aotRefusals` -> one Map per distinct refusal/skip, for finding
        // which of YOUR members stayed interpreted and why.
        .class_method("aotRefusals", |vm, mc, _receiver, _args| {
            let records = codegen::refusal_snapshot();
            let mut out = Vec::with_capacity(records.len());
            for r in records {
                let mut m = Vec::new();
                m.push(("selector".to_string(), vm.new_string(mc, r.selector)));
                m.push((
                    "kind".to_string(),
                    vm.new_string(mc, r.kind.name().to_string()),
                ));
                m.push(("reason".to_string(), vm.new_string(mc, r.why)));
                out.push(vm.new_map(mc, m));
            }
            Ok(vm.new_list(mc, out))
        })
        .doc(
            "The AOT drill-down: a List with one `#{ 'selector': ... 'kind': ... 'reason': \
             ... }` Map per distinct refusal or skip -- for finding which of YOUR members \
             stayed interpreted and why. A block literal has no user-facing name, so block \
             templates appear under the pseudo-selector 'block@<template-id>'.",
        )
        // `VM.aotCompiled` -> the positive mirror: what is natively compiled
        // RIGHT NOW (a tombstoned entry drops out, exactly as it stopped
        // being dispatched to). The tier-shape pins assert against this.
        .class_method("aotCompiled", |vm, mc, _receiver, _args| {
            let entries = codegen::compiled_snapshot();
            let mut out = Vec::with_capacity(entries.len());
            for (selector, role) in entries {
                let mut m = Vec::new();
                m.push(("selector".to_string(), vm.new_string(mc, selector)));
                let role = match role {
                    codegen::AotRole::Method => "method",
                    codegen::AotRole::BlockTemplate => "block",
                };
                m.push(("role".to_string(), vm.new_string(mc, role.to_string())));
                out.push(vm.new_map(mc, m));
            }
            Ok(vm.new_list(mc, out))
        })
        .doc(
            "What is natively compiled right now: a List with one `#{ 'selector': ... \
             'role': ... }` Map per registered entry ('method' or 'block'; block templates \
             appear as 'block@<template-id>'). The positive mirror of `aotRefusals` -- an \
             entry that was compiled and later tombstoned (a mispredicting speculation) \
             drops back out. Empty when AOT is disabled; `VM.stats.at:'aot'` carries an \
             `enabled` flag.",
        )
}
