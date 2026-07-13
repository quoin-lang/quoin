//! C2 v1 worker isolates (docs/internal/CONCURRENCY_ARCH.md §5): one arena + one
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
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gc_arena::Gc;
use gc_arena::lock::RefLock;
#[cfg(not(target_arch = "wasm32"))]
use gc_arena::{Arena, Rootable};
use quoin_ext_proto::DataValue as WireData;

#[cfg(not(target_arch = "wasm32"))]
use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::gcl;
use crate::instruction::{Constant, Instruction, StaticBlock};
#[cfg(not(target_arch = "wasm32"))]
use crate::parser::{NodeValue, try_parse_quoin_string_named};
#[cfg(not(target_arch = "wasm32"))]
use crate::runner::{
    ReplArena, compile_unit_aot, drive_main_task, install_main_task, prelude_asts,
    register_builtins, unit_compiler,
};
use crate::runtime::extension::{value_to_wire, wire_to_value};
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::runtime::build_block;
use crate::symbol::{Symbol, self_symbol};
use crate::value::{Block, EnvFrame, NamespacedName, ObjectPayload, Value};
#[cfg(not(target_arch = "wasm32"))]
use crate::vm::VmOptions;
use crate::vm::VmState;

/// A parent-initiated control request (docs/internal/CONCURRENCY_ARCH.md §13.3).
/// Each request CARRIES its reply lane, so any number may be in flight
/// with no routing table; the worker's DRIVER answers opportunistically
/// once per loop iteration — bounded staleness, no preemption.
pub struct ControlReq {
    pub kind: ControlKind,
    pub reply: async_channel::Sender<WorkerMsg>,
}

pub enum ControlKind {
    /// Answer with this worker's ps tree — its own `VM.ps` with each
    /// sub-worker row's 'ps' filled by the same request, recursively.
    PsTree,
}

/// The worker-side half of the lanes, injected into the worker's `VmState`
/// at boot: `Worker.receive` parks on `inbox_rx`, `Worker.send:` pushes to
/// `outbox_tx`; the driver services `control_rx` (§13.3).
pub struct WorkerLink {
    pub inbox_rx: async_channel::Receiver<WorkerMsg>,
    pub outbox_tx: async_channel::Sender<WorkerMsg>,
    pub control_rx: async_channel::Receiver<ControlReq>,
    /// True inside a PROCESS-backed worker: blocks refuse at the lane
    /// (templates are `Arc` references — meaningless across a process).
    pub process: bool,
}

/// Registry entry for `VM.ps`: plain lane clones — `async_channel`'s
/// `len()`/`is_closed()` give live queue depths and running/exited state
/// with zero bookkeeping. Registered at spawn, never removed (worker
/// counts are small and the entries are a few pointers).
pub struct WorkerReg {
    pub unit: String,
    /// Human-readable label for `VM.ps`/`psTree` (defaults to `unit`; the
    /// Join layer stamps its own — internal ids mean nothing to a Quoin
    /// developer).
    pub label: String,
    /// 'thread' | 'process' (§13.2).
    pub backing: &'static str,
    /// The child's pid for process backing (`None` for threads).
    pub pid: Option<u32>,
    pub inbox_tx: async_channel::Sender<WorkerMsg>,
    pub outbox_rx: async_channel::Receiver<WorkerMsg>,
    pub control_tx: async_channel::Sender<ControlReq>,
}

/// The parent-side half, held by the `Worker` handle instance.
pub struct WorkerChannels {
    pub inbox_tx: async_channel::Sender<WorkerMsg>,
    pub outbox_rx: async_channel::Receiver<WorkerMsg>,
    pub done_rx: async_channel::Receiver<Result<WireData, String>>,
    pub control_tx: async_channel::Sender<ControlReq>,
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

/// One cross-worker message: plain data (the wire taxonomy), or — the L3
/// enabler — a PORTABLE BLOCK, so pool workers can receive jobs and
/// combinator wrappers can capture the user's per-item block. Blocks cross
/// only at top level and as block-captures; a block nested INSIDE a data
/// structure still refuses (the wire walkers own that taxonomy).
#[derive(Clone, Debug)]
pub enum WorkerMsg {
    Data(WireData),
    Block(PortableBlock),
}

/// A block shipped across a worker boundary (docs/internal/CONCURRENCY_ARCH.md §10):
/// the `Send` template reference, the deep-copied snapshot of its free
/// reads (RECURSIVE — a captured block ships as its own `PortableBlock`),
/// and the global names each level resolves — checked against the worker's
/// own globals before running, so a missing user definition is a clear
/// error instead of a silent nil.
#[derive(Clone, Debug)]
pub struct PortableBlock {
    pub template: Arc<StaticBlock>,
    pub captures: Vec<(Symbol, PortableCapture)>,
    pub globals: Vec<NamespacedName>,
}

#[derive(Clone, Debug)]
pub enum PortableCapture {
    Data(WireData),
    Block(Box<PortableBlock>),
}

/// Snapshot a block value into its portable form: scan for portability,
/// then deep-copy each free read out of the capture chain — recursing when
/// a capture is itself a block (its own scan applies). The depth cap
/// converts capture cycles (`var f = ...; f = { f }`) into a clear error.
pub fn snapshot_block<'gc>(
    template: Arc<StaticBlock>,
    parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    depth: usize,
) -> Result<PortableBlock, QuoinError> {
    if depth > 8 {
        return Err(QuoinError::Other(
            "portable block captures nest too deeply (a block capturing itself?)".into(),
        ));
    }
    let (frees, globals) = scan_portable(&template)
        .map_err(|why| QuoinError::Other(format!("block is not portable: {why}")))?;
    let mut captures = Vec::with_capacity(frees.len());
    for sym in frees {
        // Absent up the chain reads as nil in the interpreter; the snapshot
        // mirrors that.
        let val = parent_env
            .and_then(|env| EnvFrame::get(env, sym))
            .unwrap_or(Value::Nil);
        let block_parts = if let Value::Object(obj) = val {
            let borrowed = obj.borrow();
            if let ObjectPayload::Block(b) = &borrowed.payload {
                Some((b.template.clone(), b.parent_env))
            } else {
                None
            }
        } else {
            None
        };
        let cap = match block_parts {
            Some((t, env)) => PortableCapture::Block(Box::new(
                snapshot_block(t, env, depth + 1)
                    .map_err(|e| QuoinError::Other(format!("capture '{}': {e}", sym.as_str())))?,
            )),
            None => PortableCapture::Data(value_to_wire(val, None).map_err(|e| {
                QuoinError::Other(format!("capture '{}' is not portable: {e}", sym.as_str()))
            })?),
        };
        captures.push((sym, cap));
    }
    Ok(PortableBlock {
        template,
        captures,
        globals,
    })
}

/// Deep-copy a shipped template into WORKER-LOCAL allocations, recursing
/// into nested block constants. Shipped `Arc<StaticBlock>`s are shared with
/// the parent (and every sibling worker); each closure materialization
/// bumps the Arc refcount, and that RMW invalidates the cache line the hot
/// dispatch loop in every OTHER worker constantly reads through (template →
/// bytecode derefs on each frame push). Measured: the shared-template pool
/// path scaled 1.8x WORSE than share-nothing unit workers on identical
/// work (profiling/worker-scaling/notes.md). Localizing is a one-time,
/// bytes-sized copy per shipped job that removes every cross-worker line.
fn localize_template(sb: &StaticBlock) -> Arc<StaticBlock> {
    let mut copy: StaticBlock = (*sb).clone();
    let mut bytecode: Vec<Instruction> = copy.bytecode.iter().cloned().collect();
    for inst in bytecode.iter_mut() {
        match inst {
            Instruction::Push(Constant::Block(inner)) => {
                *inner = localize_template(inner);
            }
            Instruction::SendConst(Constant::Block(inner), _, _)
            | Instruction::SendLocalConst(_, Constant::Block(inner), _, _) => {
                *inner = localize_template(inner);
            }
            _ => {}
        }
    }
    copy.bytecode = bytecode.into();
    if let Some(decl) = &copy.decl_block {
        copy.decl_block = Some(localize_template(decl));
    }
    Arc::new(copy)
}

/// Rebuild a shipped block as a live closure in THIS worker's arena:
/// verify its global references, wire-copy the captures into a snapshot
/// env frame (recursing for block captures), and close the template over
/// it. Used for top-level jobs and for block-valued lane messages alike.
pub(crate) fn rebuild_portable_value<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    pb: &PortableBlock,
) -> Result<Value<'gc>, String> {
    let env = rebuild_env(vm, mc, pb)?;
    let template = localize_template(&pb.template);
    let inline_cache = vm.ic_cell_for(mc, &template);
    Ok(vm.new_block(
        mc,
        Block {
            template,
            parent_env: Some(env),
            enclosing_method_id: None,
            decl_block: None,
            inline_cache,
        },
    ))
}

fn rebuild_env<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    pb: &PortableBlock,
) -> Result<Gc<'gc, RefLock<EnvFrame<'gc>>>, String> {
    for g in &pb.globals {
        if vm.globals.borrow().get(g).is_none() {
            return Err(format!(
                "global '{g}' is not defined in the worker (workers boot qnlib only \
                 — put user definitions in a unit and Worker.spawn: it)"
            ));
        }
    }
    let mut env = EnvFrame::new(None);
    for (sym, cap) in &pb.captures {
        let v = match cap {
            PortableCapture::Data(dv) => wire_to_value(vm, mc, dv, None)
                .map_err(|e| format!("capture '{}': {e}", sym.as_str()))?,
            PortableCapture::Block(inner) => rebuild_portable_value(vm, mc, inner)?,
        };
        env.bind(*sym, v);
    }
    Ok(gcl!(mc, env))
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
            Instruction::StoreLocal(s) | Instruction::StoreLocalKeep(s)
                if !defined.contains(s) && !sb.is_init_literal =>
            {
                return Err(format!(
                    "writes captured binding '{}' (the worker gets a snapshot; \
                         writes could never reach the original)",
                    s.as_str()
                ));
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
            Instruction::LoadGlobal(n) if seen_globals.insert(n.clone()) => {
                globals.push(n.clone());
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

/// Shared grip on the child for `terminate` (guest-side cancellation) and
/// the pump reader's reap; `None` once reaped.
pub type ChildGrip = std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>;

// The spawn/boot machinery (worker threads boot a full runner; process backing rides a
// Unix socket) is native-only, split into a `#[path]` child file. On wasm32 the four
// spawn entry points still exist so the `Worker` class compiles, but every spawn is
// stillborn: the done lane is primed with an error before the channels are returned,
// so `join`/`receive` surface a catchable "not supported" instead of hanging.
#[cfg(not(target_arch = "wasm32"))]
#[path = "worker_spawn.rs"]
mod worker_spawn;
#[cfg(not(target_arch = "wasm32"))]
pub use worker_spawn::{
    spawn_worker, spawn_worker_block, spawn_worker_process, spawn_worker_service, worker_serve_main,
};

#[cfg(target_arch = "wasm32")]
fn stillborn_channels() -> WorkerChannels {
    let (inbox_tx, _inbox_rx) = async_channel::unbounded();
    let (_outbox_tx, outbox_rx) = async_channel::unbounded();
    let (done_tx, done_rx) = async_channel::bounded(1);
    let (control_tx, _control_rx) = async_channel::unbounded();
    // `try_send` (send_blocking is compiled out on wasm): the lane is a fresh
    // bounded(1), so the one slot is guaranteed free.
    let _ = done_tx.try_send(Err("workers are not supported on this platform".to_string()));
    WorkerChannels {
        inbox_tx,
        outbox_rx,
        done_rx,
        control_tx,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker(_path: String) -> WorkerChannels {
    stillborn_channels()
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker_block(_job: PortableBlock) -> WorkerChannels {
    stillborn_channels()
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker_service(_path: String, _class_name: String) -> WorkerChannels {
    stillborn_channels()
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker_process(
    _unit: String,
    _service: Option<String>,
) -> Result<(WorkerChannels, u32, ChildGrip), String> {
    Err("workers are not supported on this platform".to_string())
}
