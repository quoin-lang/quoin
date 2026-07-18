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

/// One hosted-object dispatch request crossing to a worker
/// (docs/internal/ACTOR_OBJECTS.md §2): a peer-protocol `Call` frame plus the
/// CONVERSATION's two lanes. For thread backing the request IS the transport
/// (owned `Msg` values over in-memory lanes — the §1 "same protocol, cheaper
/// carrier" row); for process backing the conversation pumps relay the same
/// frames over the socket.
///
/// A conversation is strictly LIFO (the extension protocol's shape): frames
/// worker→parent ride `reply` — zero or more host-op `Call`s (a parent-held
/// handle driven from worker code), then the terminal `CallReturn*` — and
/// frames parent→worker ride `hostops` — each host-op's reply, or a NESTED
/// parent→worker `Call` riding the bound conversation (§5.1 rule 3).
#[derive(Clone, Debug)]
pub struct DispatchReq {
    pub frame: quoin_ext_proto::Msg,
    /// Shipped portable-block arguments, out-of-band of the wire frame
    /// (ACTOR_OBJECTS.md §3a): `(argument position, snapshot)` pairs; the
    /// frame's `method_args` holds a Null placeholder at each position. Only
    /// the in-memory thread lane may carry these — the same
    /// richer-than-wire-taxonomy allowance as `WorkerMsg::Block`; unportable
    /// blocks and process peers take the handle path instead.
    pub blocks: Vec<(usize, PortableBlock)>,
    pub reply: async_channel::Sender<quoin_ext_proto::Msg>,
    /// Parent→worker frames for this conversation (host-op replies and
    /// nested calls). A closed lane means the caller abandoned the
    /// conversation (cancellation) — the worker surfaces that as a catchable
    /// error in the invoking code.
    pub hostops: async_channel::Receiver<quoin_ext_proto::Msg>,
    /// The worker stamps its handler time here (µs) — the boundary-profiling
    /// decomposition (§7) for thread backing, where no wire exists to carry
    /// `ReplyMeta`. Process backing leaves it 0 until the pumps carry meta.
    pub handler_micros: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// The worker-side handles of the conversation a serve task is currently
/// inside, keyed by task in `VmState::worker_convs`: host-op `Call`s go up
/// `reply_tx`, their answers (and nested parent→worker calls) arrive on
/// `hostops_rx`. `depth` counts open host-op conversations (capped as
/// extensions cap theirs).
#[derive(Clone, Debug)]
pub struct ConvHandles {
    pub reply_tx: async_channel::Sender<quoin_ext_proto::Msg>,
    pub hostops_rx: async_channel::Receiver<quoin_ext_proto::Msg>,
    pub depth: u32,
}

/// One channel-relay event crossing a worker link (docs/internal/ACTOR_OBJECTS.md
/// §6): the frames of a shipped channel's protocol. NOT conversational — wakes
/// arrive in any order, so ops carry correlation ids (`corr`) instead of riding
/// the LIFO conversation shape. `chan` is the id the OWNER side rooted the
/// channel under in its `vm.hosted` table. Thread links carry these as owned
/// values (the richer-than-wire allowance); the wire encoding is the process
/// slice.
#[derive(Clone, Debug)]
pub enum ChanFrame {
    /// endpoint → owner: send `value`; answered by `Ack` (accepted), or
    /// `ClosedFor` (channel closed — the send raises).
    Send {
        chan: u64,
        corr: u64,
        value: WireData,
    },
    /// owner → endpoint: the send with this correlation was accepted.
    Ack { corr: u64 },
    /// endpoint → owner: receive a value; answered by `Value`, `ClosedFor`
    /// (closed and drained — nil / end of `each:`), or `RecvError`.
    Recv { chan: u64, corr: u64 },
    /// owner → endpoint: the receive's value.
    Value { corr: u64, value: WireData },
    /// owner → endpoint: the pending op's channel is closed (a receiver
    /// observes closed-and-drained; a sender raises).
    ClosedFor { corr: u64 },
    /// owner → endpoint: the receive failed (a buffered value that predates
    /// shipping turned out not to be portable).
    RecvError { corr: u64, message: String },
    /// endpoint → owner: close the channel (propagates; idempotent).
    Close { chan: u64 },
    /// endpoint → owner: retract a pending op (its task was cancelled).
    Cancel { chan: u64, corr: u64 },
    /// endpoint → owner: a relay endpoint was dropped (refcounted release).
    Release { chan: u64 },
    /// endpoint → owner: a value delivered to a since-cancelled receiver,
    /// going home for redelivery (the send already reported success — the
    /// value must not vanish).
    Return { chan: u64, value: WireData },
}

// `ChanFrame`'s wire discriminants (`Msg::Chan.kind`, docs/internal/ACTOR_OBJECTS.md
// §6): stable, append-only — the relay socket's whole vocabulary. Native-only,
// like the socket pumps that speak them (thread links move `ChanFrame` values).
#[cfg(not(target_arch = "wasm32"))]
const CK_SEND: u8 = 0;
#[cfg(not(target_arch = "wasm32"))]
const CK_ACK: u8 = 1;
#[cfg(not(target_arch = "wasm32"))]
const CK_RECV: u8 = 2;
#[cfg(not(target_arch = "wasm32"))]
const CK_VALUE: u8 = 3;
#[cfg(not(target_arch = "wasm32"))]
const CK_CLOSED_FOR: u8 = 4;
#[cfg(not(target_arch = "wasm32"))]
const CK_RECV_ERROR: u8 = 5;
#[cfg(not(target_arch = "wasm32"))]
const CK_CLOSE: u8 = 6;
#[cfg(not(target_arch = "wasm32"))]
const CK_CANCEL: u8 = 7;
#[cfg(not(target_arch = "wasm32"))]
const CK_RELEASE: u8 = 8;
#[cfg(not(target_arch = "wasm32"))]
const CK_RETURN: u8 = 9;

/// Encode one relay event as its wire frame (`Msg::Chan`) — the process
/// links' transport; thread links move `ChanFrame` values directly.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn chan_frame_to_msg(f: ChanFrame) -> quoin_ext_proto::Msg {
    use quoin_ext_proto::Msg;
    let (kind, chan, corr, value, message) = match f {
        ChanFrame::Send { chan, corr, value } => (CK_SEND, chan, corr, Some(value), String::new()),
        ChanFrame::Ack { corr } => (CK_ACK, 0, corr, None, String::new()),
        ChanFrame::Recv { chan, corr } => (CK_RECV, chan, corr, None, String::new()),
        ChanFrame::Value { corr, value } => (CK_VALUE, 0, corr, Some(value), String::new()),
        ChanFrame::ClosedFor { corr } => (CK_CLOSED_FOR, 0, corr, None, String::new()),
        ChanFrame::RecvError { corr, message } => (CK_RECV_ERROR, 0, corr, None, message),
        ChanFrame::Close { chan } => (CK_CLOSE, chan, 0, None, String::new()),
        ChanFrame::Cancel { chan, corr } => (CK_CANCEL, chan, corr, None, String::new()),
        ChanFrame::Release { chan } => (CK_RELEASE, chan, 0, None, String::new()),
        ChanFrame::Return { chan, value } => (CK_RETURN, chan, 0, Some(value), String::new()),
    };
    Msg::Chan {
        kind,
        chan,
        corr,
        value,
        message,
    }
}

/// Decode a wire frame back into a relay event; `None` for a frame that is
/// not a `Msg::Chan` or carries an unknown kind (skipped, append-only rule).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn msg_to_chan_frame(m: quoin_ext_proto::Msg) -> Option<ChanFrame> {
    let quoin_ext_proto::Msg::Chan {
        kind,
        chan,
        corr,
        value,
        message,
    } = m
    else {
        return None;
    };
    let value_or_null = || value.clone().unwrap_or(WireData::Null);
    Some(match kind {
        CK_SEND => ChanFrame::Send {
            chan,
            corr,
            value: value_or_null(),
        },
        CK_ACK => ChanFrame::Ack { corr },
        CK_RECV => ChanFrame::Recv { chan, corr },
        CK_VALUE => ChanFrame::Value {
            corr,
            value: value_or_null(),
        },
        CK_CLOSED_FOR => ChanFrame::ClosedFor { corr },
        CK_RECV_ERROR => ChanFrame::RecvError { corr, message },
        CK_CLOSE => ChanFrame::Close { chan },
        CK_CANCEL => ChanFrame::Cancel { chan, corr },
        CK_RELEASE => ChanFrame::Release { chan },
        CK_RETURN => ChanFrame::Return {
            chan,
            value: value_or_null(),
        },
        _ => return None,
    })
}

/// One worker link's channel-relay state, registered in `vm.io.chan_links`
/// (each side of a link has one): the outbound lane, the inbound lane (drained
/// by this side's relay-agent task), the pending ops this side has in flight,
/// and the reap of dropped endpoints awaiting `Release`.
#[derive(Debug)]
pub struct ChanLink {
    pub out: async_channel::Sender<ChanFrame>,
    pub inbound: async_channel::Receiver<ChanFrame>,
    /// The relay agent (`Channel.relayAgent:`) has been spawned for this link.
    pub agent_running: bool,
    /// Correlation-id source for this side's pending ops.
    pub next_corr: u64,
    /// corr → the parked task awaiting the op's answer (park-epoch identity,
    /// the channel.rs ghost rule) and the channel it targets (for `Return`).
    pub pending: std::collections::HashMap<u64, PendingChanOp>,
    /// Dropped-endpoint channel ids awaiting a `Release` frame (a GC `Drop`
    /// can't send one; flushed by the agent and by relay ops).
    pub reap: std::rc::Rc<std::cell::RefCell<Vec<u64>>>,
    /// OWNER side: hosted channel ids shipped over THIS link → how many remote
    /// endpoints this link's peer holds. `Release` frames decrement; link death
    /// drains the map — the dead isolate's endpoints can never Release, so the
    /// counts are what the owner unroots by (SUPERVISION.md slice 0).
    pub shipped: std::collections::HashMap<u64, usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingChanOp {
    pub task: usize,
    pub epoch: u64,
    pub chan: u64,
}

/// The worker link's reserved ops on the `Call` frame (`class_name` routes a
/// hosted-object dispatch; these built-ins keep it empty).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const OP_SEND: &str = "send";
/// A channel endpoint crossing the mailbox lane (`Worker.send:` of a Channel
/// over a process link): `recv` carries the owner-side channel id (§6).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const OP_SEND_CHAN: &str = "sendChan";
/// Mailbox op: a portable block, encoded as source + captures
/// (`portable_block_to_wire`) in the frame's `data`. Parent -> child only
/// today (spawn-time `args:`); plain `Worker.send:` still refuses blocks on
/// process backing at the send seam.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const OP_SEND_BLOCK: &str = "sendBlock";
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const OP_PS_TREE: &str = "psTree";
/// Ends the hosted serve loop (`WorkerService`'s `serviceStop`). Shadows a
/// hosted method of the same name by design — the proxy owns the selector.
pub(crate) const OP_STOP: &str = "serviceStop";

/// The worker-side half of the lanes, injected into the worker's `VmState`
/// at boot: `Worker.receive` parks on `inbox_rx`, `Worker.send:` pushes to
/// `outbox_tx`; the driver services `control_rx` (§13.3); the hosted serve
/// loop (`Worker.hostServe:`) parks on `dispatch_rx`.
pub struct WorkerLink {
    pub inbox_rx: async_channel::Receiver<WorkerMsg>,
    pub outbox_tx: async_channel::Sender<WorkerMsg>,
    pub control_rx: async_channel::Receiver<ControlReq>,
    pub dispatch_rx: async_channel::Receiver<DispatchReq>,
    /// Channel-relay lane, worker side: frames to the parent / from the
    /// parent (§6). Registered as a `ChanLink` when the worker boots.
    pub chan_tx: async_channel::Sender<ChanFrame>,
    pub chan_rx: async_channel::Receiver<ChanFrame>,
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

/// How a worker's done lane resolved when the body did not answer a value.
/// The distinction matters (`docs/internal/SUPERVISION.md` §2): `Failed` is the
/// body *reporting* — a unit compile error, a job raising — an ordinary
/// catchable error; `Died` is the isolate *disappearing* — process gone without
/// a terminal, thread body panicked — surfaced as the typed `PeerDiedError`.
#[derive(Debug, Clone)]
pub enum WorkerExit {
    Failed(String),
    Died {
        reason: crate::error::PeerDeathReason,
        detail: String,
    },
}

/// The parent-side half, held by the `Worker` handle instance.
pub struct WorkerChannels {
    pub inbox_tx: async_channel::Sender<WorkerMsg>,
    pub outbox_rx: async_channel::Receiver<WorkerMsg>,
    pub done_rx: async_channel::Receiver<Result<WireData, WorkerExit>>,
    /// The peer's lifecycle sink (SUPERVISION.md slice 1), created at spawn so
    /// events staged before anyone asks for `events` are kept; the parent
    /// registers it in `vm.io.lives` when the handle/proxy is minted.
    pub life: std::sync::Arc<crate::runtime::lifecycle::LifeSink>,
    pub control_tx: async_channel::Sender<ControlReq>,
    /// Hosted-object dispatch (the service proxy's lane); unused by plain
    /// workers, whose serve loop never reads the other end.
    pub dispatch_tx: async_channel::Sender<DispatchReq>,
    /// Channel-relay lane, parent side (§6): frames to the worker / from the
    /// worker. Registered as a `ChanLink` when the handle/proxy is minted.
    pub chan_tx: async_channel::Sender<ChanFrame>,
    pub chan_rx: async_channel::Receiver<ChanFrame>,
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
    /// A shipped channel endpoint (§6): the owner-side channel id; the
    /// receiving side wraps it as a relay endpoint on its link.
    Channel(u64),
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

/// Encode a portable block for a PROCESS crossing: the block literal's SOURCE
/// TEXT plus the wire-encoded capture snapshot and the global names. Source
/// text is the crossing medium — bytecode is this process's compilation, and
/// the child (the same binary, behind the version gate) re-parses and
/// compiles the text against its own unit, then binds the shipped captures
/// (`portable_block_from_wire`). Portability already guarantees the captures
/// are wire data, so the only new refusal is a block with no recorded source
/// (assembled at runtime, e.g. via eval) — loud, at ship time.
pub fn portable_block_to_wire(pb: &PortableBlock) -> Result<WireData, String> {
    let si = pb.template.source_info.as_ref();
    let source = si.and_then(|s| s.source_text.clone()).ok_or_else(|| {
        "the block carries no source text (assembled at runtime?) — it cannot \
         cross a process boundary"
            .to_string()
    })?;
    let filename = si.map(|s| s.filename.clone()).unwrap_or_default();
    let mut captures = Vec::with_capacity(pb.captures.len());
    for (sym, cap) in &pb.captures {
        let name = ("name".to_string(), WireData::Str(sym.as_str().to_string()));
        let payload = match cap {
            PortableCapture::Data(d) => ("data".to_string(), d.clone()),
            PortableCapture::Block(inner) => ("block".to_string(), portable_block_to_wire(inner)?),
        };
        captures.push(WireData::Map(vec![name, payload]));
    }
    Ok(WireData::Map(vec![
        ("source".to_string(), WireData::Str(source)),
        ("filename".to_string(), WireData::Str(filename)),
        ("captures".to_string(), WireData::List(captures)),
        (
            "globals".to_string(),
            WireData::List(
                pb.globals
                    .iter()
                    .map(|n| WireData::Str(n.to_string()))
                    .collect(),
            ),
        ),
    ]))
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
        ScanMode::Portable,
        &HashSet::new(),
        &mut free_reads,
        &mut seen_reads,
        &mut globals,
        &mut seen_globals,
    )?;
    Ok((free_reads, globals))
}

/// Collect a block's free variable names WITHOUT the portability refusals:
/// every local the block reads or writes that no enclosing shipped scope
/// binds, in first-touch order. `self`/`@field` access, `^^`, guards, and
/// definitions are simply skipped — they are not captures, and this scan
/// serves reflection (`Block#captures`), not shipping. Infallible: with
/// every refusal arm disarmed, `scan_nest` has no error path left.
pub fn scan_captures(template: &StaticBlock) -> Vec<Symbol> {
    let mut free_reads = Vec::new();
    let mut globals = Vec::new();
    let mut seen_reads = HashSet::new();
    let mut seen_globals = HashSet::new();
    scan_nest(
        template,
        ScanMode::FreeReads,
        &HashSet::new(),
        &mut free_reads,
        &mut seen_reads,
        &mut globals,
        &mut seen_globals,
    )
    .expect("FreeReads scan has no refusal arms");
    free_reads
}

/// How `scan_nest` treats the shapes that cannot cross a worker boundary:
/// `Portable` refuses them (shipping), `FreeReads` skips them (reflection).
#[derive(Clone, Copy, PartialEq)]
enum ScanMode {
    Portable,
    FreeReads,
}

fn scan_nest(
    sb: &StaticBlock,
    mode: ScanMode,
    bound: &HashSet<Symbol>,
    free_reads: &mut Vec<Symbol>,
    seen_reads: &mut HashSet<Symbol>,
    globals: &mut Vec<NamespacedName>,
    seen_globals: &mut HashSet<NamespacedName>,
) -> Result<(), String> {
    if sb.decl_block.is_some() && mode == ScanMode::Portable {
        return Err("guarded/typed blocks are not portable".to_string());
    }

    // A guard's bytecode is part of the block for reflection purposes; the
    // Portable path never gets here (guarded blocks refused above).
    if let Some(decl) = &sb.decl_block
        && mode == ScanMode::FreeReads
    {
        scan_nest(
            decl,
            mode,
            bound,
            free_reads,
            seen_reads,
            globals,
            seen_globals,
        )?;
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
            | Instruction::ExecuteBlockWithSelf
                if mode == ScanMode::Portable =>
            {
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
            if mode == ScanMode::FreeReads {
                return Ok(()); // `self` is instance state, not a capture
            }
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
                if mode == ScanMode::FreeReads {
                    // A write-captured binding is still a free variable with
                    // a current value; reflection reports it like a read.
                    read(*s, &defined, free_reads, seen_reads)?;
                } else {
                    return Err(format!(
                        "writes captured binding '{}' (the worker gets a snapshot; \
                         writes could never reach the original)",
                        s.as_str()
                    ));
                }
            }
            Instruction::MethodReturn if mode == ScanMode::Portable => {
                return Err(
                    "contains a non-local return (^^) — its home method cannot exist \
                     in the worker"
                        .to_string(),
                );
            }
            Instruction::LoadField(f)
            | Instruction::StoreField(f)
            | Instruction::StoreFieldKeep(f)
                if mode == ScanMode::Portable =>
            {
                return Err(format!("touches instance state (@{f}) — not portable"));
            }
            Instruction::SendField(f, _, _) if mode == ScanMode::Portable => {
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
                mode,
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
                mode,
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
// spawn entry points still exist so the `Worker` class compiles, but every spawn is a
// dead letter: the done lane is primed with an error before the channels are returned,
// so `join`/`receive` surface a catchable "not supported" instead of hanging.
#[cfg(not(target_arch = "wasm32"))]
#[path = "worker_spawn.rs"]
mod worker_spawn;
#[cfg(not(target_arch = "wasm32"))]
pub use worker_spawn::{
    spawn_worker, spawn_worker_block, spawn_worker_hosted_block, spawn_worker_process,
    worker_serve_main,
};

/// What a spawned worker PROCESS runs, beyond its unit: nothing (a plain
/// worker), a hosted service by class name, or a hosted BLOCK — shipped as
/// source text + wire-encoded captures (`portable_block_to_wire`), which the
/// child re-compiles against its own unit after the version gate.
#[derive(Clone, Debug)]
pub enum ProcessBody {
    Plain,
    Block(WireData),
    /// A plain JOB block (`Worker.start:` on process backing): same shipping
    /// as `Block`, but the child runs it as its whole life and the done
    /// terminal carries its value (`join`'s answer).
    Job(WireData),
}

#[cfg(target_arch = "wasm32")]
fn dead_letter_channels() -> WorkerChannels {
    let (inbox_tx, _inbox_rx) = async_channel::unbounded();
    let (_outbox_tx, outbox_rx) = async_channel::unbounded();
    let (done_tx, done_rx) = async_channel::bounded(1);
    let (control_tx, _control_rx) = async_channel::unbounded();
    let (dispatch_tx, _dispatch_rx) = async_channel::unbounded();
    let (chan_tx, _chan_inert_rx) = async_channel::unbounded();
    let (_chan_inert_tx, chan_rx) = async_channel::unbounded();
    // `try_send` (send_blocking is compiled out on wasm): the lane is a fresh
    // bounded(1), so the one slot is guaranteed free.
    let _ = done_tx.try_send(Err(WorkerExit::Failed(
        "workers are not supported on this platform".to_string(),
    )));
    let life =
        crate::runtime::lifecycle::LifeSink::new("<wasm>".to_string(), "worker", "thread", None);
    life.emit_stopped("workers are not supported on this platform");
    WorkerChannels {
        inbox_tx,
        outbox_rx,
        done_rx,
        control_tx,
        dispatch_tx,
        chan_tx,
        chan_rx,
        life,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker(_path: String) -> WorkerChannels {
    dead_letter_channels()
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker_block(_job: PortableBlock) -> WorkerChannels {
    dead_letter_channels()
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker_hosted_block(
    _path: Option<String>,
    _pb: PortableBlock,
    _lanes: u32,
) -> WorkerChannels {
    dead_letter_channels()
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_worker_process(
    _unit: Option<String>,
    _body: ProcessBody,
    _lanes: u32,
) -> Result<(WorkerChannels, u32, ChildGrip), String> {
    Err("workers are not supported on this platform".to_string())
}
