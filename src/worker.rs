//! C2 v1 worker isolates (docs/CONCURRENCY_ARCH.md §5): one arena + one
//! `VmState` + one cooperative scheduler per OS thread, communicating by
//! MESSAGE PASSING with deep copy — the extension wire's value taxonomy
//! (data crosses, blocks/instances refuse), in-memory: the `WireData` tree
//! itself is the transfer format, no socket, no msgpack.
//!
//! Tasks pin to their worker: a worker is a scheduling domain, not a thread
//! in a pool. The parent talks to it through three `async_channel` lanes
//! (inbox, outbox, done) whose endpoints are plain `Send` data — so a
//! parked receive/join is an ordinary driver future, woken through the same
//! path as an fd event, and every existing async combinator
//! (`Async.gather:`/`timeout:do:`/cancellation) composes over worker waits
//! with no new vocabulary (the L2 handle-as-task property, §10).
//!
//! The worker thread boots the full stdlib exactly like `qn <file>` does
//! (per the audit: arena construction is self-contained; every process
//! global is already `Sync`; extension spawns cannot collide). Definitions
//! made after boot are worker-local. Errors — parse, compile, runtime,
//! panic — travel the done lane as strings and surface to `join` as a
//! catchable error.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gc_arena::lock::RefLock;
use gc_arena::{Arena, Gc, Rootable};
use quoin_ext_proto::DataValue as WireData;

use crate::compiler::Compiler;
use crate::gcl;
use crate::instruction::{Constant, Instruction, StaticBlock};
use crate::parser::{NodeValue, try_parse_quoin_string_named};
use crate::runner::{
    ReplArena, compile_unit_aot, drive_main_task, install_main_task, prelude_asts,
    register_builtins, unit_compiler,
};
use crate::runtime::extension::{value_to_wire, wire_to_value};
use crate::runtime::runtime::build_block;
use crate::symbol::{Symbol, self_symbol};
use crate::value::{EnvFrame, NamespacedName, Value};
use crate::vm::{VmOptions, VmState};

/// The worker-side half of the lanes, injected into the worker's `VmState`
/// at boot: `Worker.receive` parks on `inbox_rx`, `Worker.send:` pushes to
/// `outbox_tx`.
pub struct WorkerLink {
    pub inbox_rx: async_channel::Receiver<WireData>,
    pub outbox_tx: async_channel::Sender<WireData>,
}

/// The parent-side half, held by the `Worker` handle instance.
pub struct WorkerChannels {
    pub inbox_tx: async_channel::Sender<WireData>,
    pub outbox_rx: async_channel::Receiver<WireData>,
    pub done_rx: async_channel::Receiver<Result<WireData, String>>,
}

// Counters for the `VM.stats` 'workers' section. Message counts are bumped
// at the send seams (both directions), where the copy happens.
static SPAWNED: AtomicUsize = AtomicUsize::new(0);
static COMPLETED: AtomicUsize = AtomicUsize::new(0);
static MESSAGES: AtomicUsize = AtomicUsize::new(0);

/// `(spawned, completed, messages)` across the process so far.
pub fn stats() -> (usize, usize, usize) {
    (
        SPAWNED.load(Ordering::Relaxed),
        COMPLETED.load(Ordering::Relaxed),
        MESSAGES.load(Ordering::Relaxed),
    )
}

/// Record one cross-worker message (either direction).
pub fn note_message() {
    MESSAGES.fetch_add(1, Ordering::Relaxed);
}

/// A block shipped to a worker (docs/CONCURRENCY_ARCH.md §10, PORTABLE
/// BLOCKS): the `Send` template reference, the deep-copied snapshot of its
/// free reads, and the global names it resolves — checked against the
/// worker's own globals before the block runs, so a missing user definition
/// is a clear error instead of a silent nil.
pub struct PortableJob {
    pub template: Arc<StaticBlock>,
    pub captures: Vec<(Symbol, WireData)>,
    pub globals: Vec<NamespacedName>,
}

/// Scan a block template (recursively through nested literals) for
/// portability: collect its free READS (names not bound by any shipped
/// scope — these get snapshot-copied) and its global references, and refuse
/// the shapes that cannot cross a worker boundary — write-captures (the
/// worker's copy could never write back), `^^` (its home method cannot
/// exist over there), `self`/`@field` access (instance state is
/// arena-local), guarded/typed blocks, and class/method definition (use a
/// unit + `Worker.spawn:` for those).
///
/// Shadowing mirrors the AOT nest scanner (`scan_materialized_nest`): a
/// level's bound set is its params plus every `DefineLocal` in it,
/// collected up front.
pub fn scan_portable(template: &StaticBlock) -> Result<(Vec<Symbol>, Vec<NamespacedName>), String> {
    let mut free_reads = Vec::new();
    let mut globals = Vec::new();
    let mut seen_reads = HashSet::new();
    let mut seen_globals = HashSet::new();
    scan_nest(
        template,
        &HashSet::new(),
        &mut free_reads,
        &mut seen_reads,
        &mut globals,
        &mut seen_globals,
    )?;
    Ok((free_reads, globals))
}

fn scan_nest(
    sb: &StaticBlock,
    bound: &HashSet<Symbol>,
    free_reads: &mut Vec<Symbol>,
    seen_reads: &mut HashSet<Symbol>,
    globals: &mut Vec<NamespacedName>,
    seen_globals: &mut HashSet<NamespacedName>,
) -> Result<(), String> {
    if sb.decl_block.is_some() {
        return Err("guarded/typed blocks are not portable".to_string());
    }

    let mut defined: HashSet<Symbol> = bound.clone();
    defined.extend(sb.param_syms.iter().copied());
    for inst in sb.bytecode.iter() {
        match inst {
            Instruction::DefineLocal(s) | Instruction::DefineLocalKeep(s) => {
                defined.insert(*s);
            }
            // Class/method definition machinery drags method bodies along
            // (whose ^^/@field use is legitimate there) — refuse the whole
            // shape up front, before recursion could misread those bodies.
            Instruction::DefineClass { .. }
            | Instruction::DefineMethod(_)
            | Instruction::OverrideMethod(_)
            | Instruction::ExecuteBlockWithSelf => {
                return Err(
                    "defines a class or method — put definitions in a unit and use \
                     Worker.spawn: instead"
                        .to_string(),
                );
            }
            _ => {}
        }
    }

    let read = |s: Symbol,
                defined: &HashSet<Symbol>,
                free_reads: &mut Vec<Symbol>,
                seen_reads: &mut HashSet<Symbol>|
     -> Result<(), String> {
        if defined.contains(&s) {
            return Ok(());
        }
        if s == self_symbol() {
            return Err("references `self` (instance state is not portable)".to_string());
        }
        if seen_reads.insert(s) {
            free_reads.push(s);
        }
        Ok(())
    };

    for inst in sb.bytecode.iter() {
        match inst {
            Instruction::LoadLocal(s) => read(*s, &defined, free_reads, seen_reads)?,
            Instruction::SendLocal(v, _, _) => read(*v, &defined, free_reads, seen_reads)?,
            Instruction::SendLocalLocal(a, b, _, _) => {
                read(*a, &defined, free_reads, seen_reads)?;
                read(*b, &defined, free_reads, seen_reads)?;
            }
            Instruction::SendLocalConst(a, _, _, _) => read(*a, &defined, free_reads, seen_reads)?,
            Instruction::IntBinLL(a, b, _) | Instruction::DoubleBinLL(a, b, _) => {
                read(*a, &defined, free_reads, seen_reads)?;
                read(*b, &defined, free_reads, seen_reads)?;
            }
            Instruction::IntBinLC(a, _, _) | Instruction::DoubleBinLC(a, _, _) => {
                read(*a, &defined, free_reads, seen_reads)?
            }
            Instruction::StoreLocal(s) | Instruction::StoreLocalKeep(s) => {
                if !defined.contains(s) && !sb.is_init_literal {
                    return Err(format!(
                        "writes captured binding '{}' (the worker gets a snapshot; \
                         writes could never reach the original)",
                        s.as_str()
                    ));
                }
            }
            Instruction::MethodReturn => {
                return Err(
                    "contains a non-local return (^^) — its home method cannot exist \
                     in the worker"
                        .to_string(),
                );
            }
            Instruction::LoadField(f)
            | Instruction::StoreField(f)
            | Instruction::StoreFieldKeep(f) => {
                return Err(format!("touches instance state (@{f}) — not portable"));
            }
            Instruction::SendField(f, _, _) => {
                return Err(format!("touches instance state (@{f}) — not portable"));
            }
            Instruction::LoadGlobal(n) => {
                if seen_globals.insert(n.clone()) {
                    globals.push(n.clone());
                }
            }
            _ => {}
        }
        if let Instruction::Push(Constant::Block(inner)) = inst {
            scan_nest(
                inner,
                &defined,
                free_reads,
                seen_reads,
                globals,
                seen_globals,
            )?;
        }
        if let Some((_, _, Some(Constant::Block(inner)))) = inst.send_parts() {
            scan_nest(
                inner,
                &defined,
                free_reads,
                seen_reads,
                globals,
                seen_globals,
            )?;
        }
    }
    Ok(())
}

/// Spawn a worker running the unit at `path` on its own OS thread. Returns
/// immediately with the parent's channel ends; boot/parse/run failures
/// travel the done lane. The thread is detached — its lifecycle is observed
/// through the lanes (`join`), and process exit ends unjoined workers.
pub fn spawn_worker(path: String) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_unit(&path, link))
}

/// Spawn a worker running a portable block (docs/CONCURRENCY_ARCH.md §10):
/// same lanes, same lifecycle; `join` returns the BLOCK'S VALUE (copied),
/// unlike unit workers' nil.
pub fn spawn_worker_block(job: PortableJob) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_block(job, link))
}

/// The shared thread + lane setup; `body` is the worker's whole life.
fn spawn_worker_with(
    body: impl FnOnce(WorkerLink) -> Result<WireData, String> + Send + 'static,
) -> WorkerChannels {
    let (inbox_tx, inbox_rx) = async_channel::unbounded();
    let (outbox_tx, outbox_rx) = async_channel::unbounded();
    let (done_tx, done_rx) = async_channel::bounded(1);
    let id = SPAWNED.fetch_add(1, Ordering::Relaxed);
    std::thread::Builder::new()
        .name(format!("qn-worker-{id}"))
        .spawn(move || {
            // A panic anywhere in the worker (parser internals, VM bugs)
            // must not tear down the process silently — it becomes the
            // done-lane error. The closure owns everything it touches, so
            // unwind-safety is vacuous.
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                body(WorkerLink {
                    inbox_rx,
                    outbox_tx,
                })
            }))
            .unwrap_or_else(|p| {
                let what = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown panic".to_string());
                Err(format!("worker panicked: {what}"))
            });
            COMPLETED.fetch_add(1, Ordering::Relaxed);
            let _ = done_tx.send_blocking(out);
        })
        .expect("spawn worker thread");
    WorkerChannels {
        inbox_tx,
        outbox_rx,
        done_rx,
    }
}

/// The worker thread body: boot a fresh VM (builtins + full qnlib prelude,
/// exactly the `qn <file>` recipe), inject the link, compile and drive the
/// unit to completion. v1 join carries no payload (`Null` on success) —
/// results travel as messages.
fn run_worker_unit(path: &str, link: WorkerLink) -> Result<WireData, String> {
    let source = std::fs::read_to_string(PathBuf::from(path))
        .map_err(|e| format!("worker unit {path}: {e}"))?;
    let ast = try_parse_quoin_string_named(&source, path)
        .map_err(|e| format!("worker unit {path}: parse error: {e}"))?;
    let NodeValue::Program(program_node) = &ast.value else {
        return Err(format!("worker unit {path}: root AST is not a program"));
    };

    let mut arena = boot_worker_arena(link)?;

    let mut compile_err = None;
    arena.mutate_root(|mc, vm| {
        let mut compiler = unit_compiler();
        compiler.set_seen_types(vm.options.seen_types.clone());
        compiler.set_class_table(vm.options.class_table.clone());
        crate::class_table::populate_from_vm(vm, &vm.options.class_table);
        let program = match compiler.compile_program(program_node) {
            Ok(p) => p,
            Err(e) => {
                compile_err = Some(format!("worker unit {path}: compile error: {e}"));
                return;
            }
        };
        vm.report_type_warnings(compiler.diagnostics());
        compile_unit_aot(vm, &mut compiler);
        let main_block = vm.block_from_template(mc, std::sync::Arc::new(program), None, None);
        vm.start_block(mc, main_block, Vec::new(), None, None);
        install_main_task(mc, vm);
    });
    if let Some(msg) = compile_err {
        return Err(msg);
    }

    drive_main_task(&mut arena).map_err(|e| format!("worker unit {path}: {e}"))?;
    Ok(WireData::Null)
}

/// Boot a fresh worker VM: arena + native builtins + the full qnlib prelude
/// (the exact `qn <file>` recipe), with the parent link injected. Shared by
/// the unit and portable-block worker bodies.
fn boot_worker_arena(link: WorkerLink) -> Result<ReplArena, String> {
    let mut arena: ReplArena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        register_builtins(mc, &mut vm);
        vm.worker_link = Some(link);
        vm
    });
    arena.metrics().set_pacing(crate::vm::gc_pacing());

    for ast in prelude_asts() {
        let mut failed = None;
        arena.mutate_root(|mc, vm| {
            let NodeValue::Program(p) = &ast.value else {
                return;
            };
            match Compiler::new().with_template_ids().compile_program(p) {
                Ok(sb) => {
                    let block = build_block(mc, &sb);
                    if let Err(e) = vm.execute_block(mc, block, Vec::new(), None) {
                        failed = Some(format!("worker prelude failed: {e}"));
                    }
                }
                Err(e) => failed = Some(format!("worker prelude compile error: {e}")),
            }
        });
        if let Some(msg) = failed {
            return Err(msg);
        }
    }
    Ok(arena)
}

/// The portable-block worker body: boot, verify the block's global
/// references against THIS VM's globals (clear error over silent nil),
/// rebuild the closure over a snapshot env frame, drive it as the main
/// task, and copy its value back for `join`.
fn run_worker_block(job: PortableJob, link: WorkerLink) -> Result<WireData, String> {
    let mut arena = boot_worker_arena(link)?;

    let mut start_err = None;
    arena.mutate_root(|mc, vm| {
        for g in &job.globals {
            if vm.globals.borrow().get(g).is_none() {
                start_err = Some(format!(
                    "global '{g}' is not defined in the worker (workers boot qnlib \
                     only — put user definitions in a unit and Worker.spawn: it)"
                ));
                return;
            }
        }
        let mut env = EnvFrame::new(None);
        for (sym, dv) in &job.captures {
            match wire_to_value(vm, mc, dv, None) {
                Ok(v) => env.bind(*sym, v),
                Err(e) => {
                    start_err = Some(format!("capture '{}': {e}", sym.as_str()));
                    return;
                }
            }
        }
        let env_ref = gcl!(mc, env);
        let block = vm.block_from_template(mc, job.template.clone(), Some(env_ref), None);
        vm.start_block(mc, block, Vec::new(), None, None);
        install_main_task(mc, vm);
    });
    if let Some(msg) = start_err {
        return Err(msg);
    }

    drive_main_task(&mut arena).map_err(|e| format!("worker block: {e}"))?;

    // The completed main task leaves the block's value on the stack top.
    arena.mutate_root(|_mc, vm| {
        let v = vm.stack.last().copied().unwrap_or(Value::Nil);
        value_to_wire(v, None)
            .map_err(|e| format!("the worker block's result is not portable data: {e}"))
    })
}
