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

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use gc_arena::{Arena, Rootable};
use quoin_ext_proto::DataValue as WireData;

use crate::compiler::Compiler;
use crate::parser::{NodeValue, try_parse_quoin_string_named};
use crate::runner::{
    ReplArena, compile_unit_aot, drive_main_task, install_main_task, prelude_asts,
    register_builtins, unit_compiler,
};
use crate::runtime::runtime::build_block;
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

/// Spawn a worker running the unit at `path` on its own OS thread. Returns
/// immediately with the parent's channel ends; boot/parse/run failures
/// travel the done lane. The thread is detached — its lifecycle is observed
/// through the lanes (`join`), and process exit ends unjoined workers.
pub fn spawn_worker(path: String) -> WorkerChannels {
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
                run_worker_unit(
                    &path,
                    WorkerLink {
                        inbox_rx,
                        outbox_tx,
                    },
                )
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
