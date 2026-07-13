//! The `Worker` class — C2 v1 isolates (docs/internal/CONCURRENCY_ARCH.md §5).
//!
//! Parent side: `Worker.spawn:'unit.qn'` boots a fresh VM on its own OS
//! thread and returns a handle; `handle.send:` deep-copies a data value in,
//! `handle.receive` parks until the worker sends one back, `handle.join`
//! parks until the unit finishes (raising the worker's error, catchable).
//! Worker side (class-side, inside the spawned unit): `Worker.receive` /
//! `Worker.send:` are the mirror lanes and `Worker.worker?` says which side
//! you're on.
//!
//! Receives and joins park through `await_io` like any IO wait — a worker
//! handle IS a parked task, so `Async.gather:`/`timeout:do:`/cancellation
//! compose over worker waits unchanged (the §10 L2 property).
//!
//! What crosses: the extension wire's data taxonomy via the same walkers
//! (numbers, strings, booleans, nil, Bytes, lists, maps, big numerics).
//! Blocks, symbols, instances, and resources refuse with the wire's errors.
//! A receive on an exited worker's drained outbox answers nil.

use std::any::Any;

use gc_arena::Collect;
use gc_arena::collect::Trace;
use quoin_ext_proto::DataValue as WireData;

use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::extension::{value_to_wire, wire_to_value};
use crate::value::ObjectPayload;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::worker::{
    WorkerMsg, note_message, rebuild_portable_value, snapshot_block, spawn_worker,
    spawn_worker_block,
};

/// Parent-side handle state: the three channel ends. Plain `Send` data —
/// nothing here touches the arena.
#[derive(Debug)]
pub struct NativeWorkerHandle {
    backing: &'static str,
    /// Registry slot, for `label:` restamps (`VM.ps` rows are the audience).
    reg_idx: usize,
    /// Process backing only: the grip `terminate` kills through.
    grip: Option<crate::worker::ChildGrip>,
    inbox_tx: async_channel::Sender<WorkerMsg>,
    outbox_rx: async_channel::Receiver<WorkerMsg>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
    /// `join` consumes the done lane (its channel holds exactly one value);
    /// a second join is a clear error rather than a confusing hang.
    joined: std::cell::Cell<bool>,
}

impl AnyCollect for NativeWorkerHandle {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

unsafe impl<'gc> Collect<'gc> for NativeWorkerHandle {
    const NEEDS_TRACE: bool = false;
}

/// Copy a guest value into a cross-worker message. A BLOCK value ships as
/// a portable block (template + capture snapshot — same rules as
/// `Worker.start:`); everything else takes the wire walkers, whose
/// taxonomy still refuses symbols/instances/resources — and blocks nested
/// INSIDE data structures.
fn to_message<'gc>(v: Value<'gc>, allow_blocks: bool) -> Result<WorkerMsg, QuoinError> {
    if let Value::Object(obj) = v {
        let block_parts = {
            let borrowed = obj.borrow();
            if let ObjectPayload::Block(b) = &borrowed.payload {
                Some((b.template.clone(), b.parent_env))
            } else {
                None
            }
        };
        if let Some((template, parent_env)) = block_parts {
            if !allow_blocks {
                return Err(QuoinError::Other(
                    "blocks cannot cross a process boundary (templates are \
                     in-process references) — send data, or use thread backing"
                        .into(),
                ));
            }
            let pb = snapshot_block(template, parent_env, 0)?;
            note_message();
            return Ok(WorkerMsg::Block(pb));
        }
    }
    let dv = value_to_wire(v, None)?;
    note_message();
    Ok(WorkerMsg::Data(dv))
}

/// Decode a received cross-worker message into a live value: data through
/// the wire walkers, blocks rebuilt over their capture snapshots.
fn from_message<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    msg: &WorkerMsg,
) -> Result<Value<'gc>, QuoinError> {
    match msg {
        WorkerMsg::Data(dv) => wire_to_value(vm, mc, dv, None),
        WorkerMsg::Block(pb) => rebuild_portable_value(vm, mc, pb).map_err(QuoinError::Other),
    }
}

/// Wrap freshly spawned lanes in a Worker-class handle instance.
#[allow(clippy::too_many_arguments)] // handle-wrapping helper threads the worker/channel context
fn wrap_handle<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    unit: &str,
    backing: &'static str,
    pid: Option<u32>,
    grip: Option<crate::worker::ChildGrip>,
    ch: crate::worker::WorkerChannels,
) -> Result<Value<'gc>, QuoinError> {
    let Value::Class(class) = receiver else {
        return Err(QuoinError::Other("Worker: bad receiver".into()));
    };
    let reg_idx = vm.worker_registry.len();
    vm.worker_registry.push(crate::worker::WorkerReg {
        unit: unit.to_string(),
        label: unit.to_string(),
        backing,
        pid,
        inbox_tx: ch.inbox_tx.clone(),
        outbox_rx: ch.outbox_rx.clone(),
        control_tx: ch.control_tx.clone(),
    });
    Ok(vm.new_native_state(
        mc,
        class,
        NativeWorkerHandle {
            backing,
            reg_idx,
            grip,
            inbox_tx: ch.inbox_tx,
            outbox_rx: ch.outbox_rx,
            done_rx: ch.done_rx,
            joined: std::cell::Cell::new(false),
        },
    ))
}

pub fn build_worker_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Worker", Some("Object"))
        .construct_with("use Worker.spawn: / Worker.start:")
        .class_doc(
            "An isolate: a fresh VM on its own OS thread (or child process) with message \
             lanes to its parent. Parent side: `Worker.spawn:'unit.qn'` answers a handle -- \
             `send:` / `receive` exchange values, `join` parks until the unit finishes. \
             Worker side, inside the spawned unit: class-side `Worker.receive` / \
             `Worker.send:` are the mirror lanes, and `Worker.worker?` says which side you \
             are on. Messages deep-copy plain data (numbers, strings, booleans, nil, Bytes, \
             Lists, Maps); symbols, instances, and resources refuse -- and blocks cross \
             only as a whole thread-backed message. See docs/internal/CONCURRENCY_ARCH.md.\n\n\
             ```\n\
             var w = Worker.spawn:'jobs/indexer.qn';\n\
             w.send:#( 'index' 'docs/' );\n\
             var answer = w.receive;\n\
             w.join\n\
             ```",
        )
        // ---- parent side (class-side spawn, instance-side lanes) ----
        .class_method("spawn:", |vm, mc, receiver, args| {
            let path = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("Worker.spawn: expects a String path".into()))?;
            let reg = path.clone();
            wrap_handle(
                vm,
                mc,
                receiver,
                &reg,
                "thread",
                None,
                None,
                spawn_worker(path),
            )
        })
        .doc(
            "Boot a fresh VM running the unit at the String path on its own OS thread and \
             answer its handle immediately. The unit runs to completion; `join` observes \
             it. `Worker.spawn:(VM.unit)` runs another copy of the current program.",
        )
        // §13.2: backing is a spawn-time choice — thread (default) or a
        // child qn process bridged by the pump.
        .class_method("spawn:backing:", |vm, mc, receiver, args| {
            let path = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("Worker.spawn: expects a String path".into()))?;
            let backing = args[1]
                .as_string()
                .ok_or_else(|| QuoinError::Other("backing: expects a String".into()))?;
            match backing.as_str() {
                "thread" => {
                    let reg = path.clone();
                    wrap_handle(
                        vm,
                        mc,
                        receiver,
                        &reg,
                        "thread",
                        None,
                        None,
                        spawn_worker(path),
                    )
                }
                "process" => {
                    let reg = path.clone();
                    let (ch, pid, grip) = crate::worker::spawn_worker_process(path, None)
                        .map_err(QuoinError::Other)?;
                    wrap_handle(vm, mc, receiver, &reg, "process", Some(pid), Some(grip), ch)
                }
                other => Err(QuoinError::Other(format!(
                    "Worker.spawn: unknown backing '{other}' (thread|process)"
                ))),
            }
        })
        .doc(
            "As `spawn:`, choosing the backing at spawn time: 'thread' (the default) or \
             'process' -- a child qn process bridged over the extension wire, whose \
             messages carry data only (blocks cannot cross a process boundary).",
        )
        // Portable blocks are in-process by nature; the explicit form
        // documents WHY rather than silently doing the wrong thing.
        .class_method("start:backing:", |vm, mc, receiver, args| {
            let backing = args[1]
                .as_string()
                .ok_or_else(|| QuoinError::Other("backing: expects a String".into()))?;
            match backing.as_str() {
                "thread" => {
                    // Same as Worker.start: — reuse its body via the send.
                    vm.call_method(mc, receiver, "start:", vec![args[0]])
                }
                "process" => Err(QuoinError::Other(
                    "Worker.start: blocks cannot cross a process boundary — put the \
                     code in a unit and Worker.spawn:backing:'process' it"
                        .into(),
                )),
                other => Err(QuoinError::Other(format!(
                    "Worker.start: unknown backing '{other}' (thread|process)"
                ))),
            }
        })
        .doc(
            "As `start:` for 'thread' backing. Blocks cannot cross a process boundary \
             (templates are in-process references), so 'process' refuses loudly -- put the \
             code in a unit and `Worker.spawn:backing:` it instead.",
        )
        // Portable blocks (docs/internal/CONCURRENCY_ARCH.md §10): ship the block's
        // template by reference plus a deep-copied SNAPSHOT of its free
        // reads; join returns the block's value. The portability scan
        // refuses write-captures, ^^, self/@fields, guarded blocks, and
        // class/method definition — loudly, at submit time.
        .class_method("start:", |vm, mc, receiver, args| {
            let Value::Object(obj) = args[0] else {
                return Err(QuoinError::Other("Worker.start: expects a Block".into()));
            };
            let (template, parent_env) = {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::Block(b) => (b.template.clone(), b.parent_env),
                    _ => {
                        return Err(QuoinError::Other("Worker.start: expects a Block".into()));
                    }
                }
            };
            if !template.param_syms.is_empty() {
                return Err(QuoinError::Other(
                    "Worker.start: the block takes no parameters (send it data through \
                     the lanes instead)"
                        .into(),
                ));
            }
            let pb = snapshot_block(template, parent_env, 0)
                .map_err(|e| QuoinError::Other(format!("Worker.start: {e}")))?;
            wrap_handle(
                vm,
                mc,
                receiver,
                "<block>",
                "thread",
                None,
                None,
                spawn_worker_block(pb),
            )
        })
        .doc(
            "Spawn a thread-backed worker from a portable BLOCK instead of a unit file: the \
             block's template ships by reference plus a deep-copied snapshot of its free \
             reads, and `join` answers the block's value (unlike a unit worker's nil). The \
             portability scan refuses write-captures, `^^`, `self`/`@fields`, guarded \
             blocks, and class/method definition -- loudly, at submit time. The block takes \
             no parameters; send it data through the lanes.\n\n\
             ```\n\
             var h = Worker.start:{ 21 * 2 };\n\
             h.join    \"* -> 42\n\
             ```",
        )
        .instance_method("send:", |vm, _mc, receiver, args| {
            let (tx, backing) = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| (h.inbox_tx.clone(), h.backing))
                .map_err(QuoinError::Other)?;
            let dv = to_message(args[0], backing == "thread")?;
            tx.try_send(dv)
                .map_err(|_| QuoinError::Other("Worker.send: the worker has exited".into()))?;
            Ok(vm.new_nil(_mc))
        })
        .doc(
            "Send a value into the worker's inbox (deep-copied; a thread-backed worker also \
             accepts a portable block). Raises if the worker has exited. Answers nil.",
        )
        .instance_method("receive", |vm, mc, receiver, _args| {
            let rx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.outbox_rx.clone())
                .map_err(QuoinError::Other)?;
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                IoResult::WorkerMsg(Some(msg)) => from_message(vm, mc, &msg),
                IoResult::WorkerMsg(None) => Ok(vm.new_nil(mc)),
                other => Err(QuoinError::Other(format!(
                    "Worker.receive: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Park until the worker sends a value back (its class-side `Worker.send:`); nil \
             once the worker has exited and its outbox is drained. Parks like any I/O wait, \
             so it composes with `Async.gather:` / `timeout:do:` / cancellation.",
        )
        // handle.label:'name' — restamp the registry row (VM.ps/psTree show
        // it; the Plan layer marks ownership and orphans this way).
        .instance_method("label:", |vm, _mc, receiver, args| {
            let label = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("label: expects a String".into()))?;
            let idx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.reg_idx)
                .map_err(QuoinError::Other)?;
            if let Some(reg) = vm.worker_registry.get_mut(idx) {
                reg.label = label;
            }
            Ok(receiver)
        })
        .doc(
            "Restamp the worker's row in `VM.ps` / `VM.psTree` with a human-readable name; \
             answers the handle. The Plan layer marks ownership and orphans this way.",
        )
        // handle.terminate — REAL cancellation, process backing only (a
        // thread worker cannot be killed; orphan it instead). Idempotent;
        // join afterwards reports the exit as an error.
        .instance_method("terminate", |_vm, _mc, receiver, _args| {
            let (grip, backing) = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| (h.grip.clone(), h.backing))
                .map_err(QuoinError::Other)?;
            let Some(grip) = grip else {
                return Err(QuoinError::Other(format!(
                    "terminate: only process-backed workers can be killed \
                     (this one is {backing}-backed) — orphan or join it instead"
                )));
            };
            if let Some(c) = grip.lock().expect("child grip").as_mut() {
                let _ = c.kill();
            }
            Ok(Value::Nil)
        })
        .doc(
            "Kill a process-backed worker -- REAL cancellation, idempotent; a later `join` \
             reports the exit as an error. A thread-backed worker cannot be killed: this \
             raises, so orphan or join it instead.",
        )
        .instance_method("join", |vm, mc, receiver, _args| {
            let rx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| {
                    if h.joined.get() {
                        None
                    } else {
                        h.joined.set(true);
                        Some(h.done_rx.clone())
                    }
                })
                .map_err(QuoinError::Other)?;
            let Some(rx) = rx else {
                return Err(QuoinError::Other("Worker.join: already joined".into()));
            };
            match vm.await_io(IoRequest::WorkerJoin(rx))? {
                IoResult::WorkerDone(Ok(dv)) => wire_to_value(vm, mc, &dv, None),
                IoResult::WorkerDone(Err(msg)) => Err(QuoinError::Other(msg)),
                other => Err(QuoinError::Other(format!(
                    "Worker.join: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Park until the worker finishes. A `Worker.start:` block worker answers the \
             block's value (copied); a unit worker answers nil. Raises the worker's error, \
             catchably, if it failed. A handle can be joined once -- a second join raises.",
        )
        // ---- worker side (class-side lanes, live only inside a worker) ----
        .class_method("receive", |vm, mc, _receiver, _args| {
            let Some(link) = vm.worker_link.as_ref() else {
                return Err(QuoinError::Other(
                    "Worker.receive: not inside a worker (spawn one with Worker.spawn:)".into(),
                ));
            };
            let rx = link.inbox_rx.clone();
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                IoResult::WorkerMsg(Some(msg)) => from_message(vm, mc, &msg),
                IoResult::WorkerMsg(None) => Ok(vm.new_nil(mc)),
                other => Err(QuoinError::Other(format!(
                    "Worker.receive: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Worker side (inside a spawned unit): park for the next value from the parent; \
             nil once the parent's lane is closed and drained. Raises when not inside a \
             worker.",
        )
        .class_method("send:", |vm, mc, _receiver, args| {
            let Some(link) = vm.worker_link.as_ref() else {
                return Err(QuoinError::Other("Worker.send: not inside a worker".into()));
            };
            let tx = link.outbox_tx.clone();
            let allow_blocks = !link.process;
            let dv = to_message(args[0], allow_blocks)?;
            tx.try_send(dv)
                .map_err(|_| QuoinError::Other("Worker.send: the parent has gone away".into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Worker side (inside a spawned unit): send a value to the parent's `receive` \
             lane (deep-copied). Raises when not inside a worker, or when the parent has \
             gone away. Answers nil.",
        )
        .class_method("worker?", |vm, mc, _receiver, _args| {
            Ok(vm.new_bool(mc, vm.worker_link.is_some()))
        })
        .doc(
            "True inside a spawned worker, false in the main program -- how a unit that can \
             run both ways tells which side it is on.",
        )
}

/// Value → String helper used by `spawn:` (mirrors `arg!` string extraction
/// without the macro's class plumbing).
trait AsStringArg {
    fn as_string(&self) -> Option<String>;
}

impl<'gc> AsStringArg for Value<'gc> {
    fn as_string(&self) -> Option<String> {
        match self {
            Value::Object(obj) => match &obj.borrow().payload {
                crate::value::ObjectPayload::String(s) => Some((**s).clone()),
                _ => None,
            },
            _ => None,
        }
    }
}
