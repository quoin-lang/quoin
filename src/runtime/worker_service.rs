//! `WorkerService` — hosted objects on the peer protocol (docs/internal/ACTOR_OBJECTS.md
//! §2; the L4 of docs/internal/CONCURRENCY_ARCH.md §10, converged): host a class in a
//! dedicated worker isolate and get a PROXY whose ordinary method sends become
//! peer-protocol `Call` frames. Sticky state, serialized access — an actor.
//!
//! ```text
//! var index = WorkerService.host:'search/index.qn' class:'SearchIndex';
//! index.add:doc;
//! var hits = index.query:'quoin';
//! ```
//!
//! The proxy forwards through the dispatch MNU seam: a selector the proxy's own
//! class doesn't define (everything except `serviceStop`) builds a
//! `Call{class_name, op, recv, method_args}` and parks for its `CallReturn*`
//! terminal — so callers compose with `Async.gather:`/`timeout:do:` like any
//! parked wait, and the hook costs nothing on the hot path (it sits on the
//! lookup-miss branch). The hook is TEMPORARY by decision (2026-07-14): once
//! hosted classes declare their selectors, the parent installs a real class
//! (the `install_ext_class` pattern) and this seam goes away — see
//! ACTOR_OBJECTS.md §10.
//!
//! HOSTED RETURNS — the actor-object rule: a method's portable return COPIES
//! back (`CallReturnData`); a non-portable object return is HOSTED in the
//! worker's table and comes back as `CallReturnResource`, which this side wraps
//! as a SUB-PROXY (same worker, its own object id). Sub-proxies are ordinary
//! receivers — including as ARGUMENTS to further calls on the same worker,
//! where they travel as live references (`Arg::Resource`). A dropped proxy's id
//! is reaped and flushed on the next call (`Call.releases`).
//!
//! SERIALIZATION — per-object mailboxes + lanes (ACTOR_OBJECTS.md §5.1): a
//! top-level send acquires (its object's claim, a lane) JOINTLY and
//! atomically via the shared `PeerClaims` state machine (`claims.rs`) — sends
//! to one object serialize FIFO (the mailbox), sends to different objects
//! overlap up to `lanes:` (each lane is a worker serve fiber). A queued
//! caller holds nothing while it waits; a nested send rides its bound lane
//! and waits only for objects; every remaining deadlock is an object-claim
//! cycle, detected at park time and raised catchably at the task that closes
//! it. Shapes are observable via `VM.claims` / `VM.claimsReport`, and calls
//! feed `VM.boundaryStats` rows beside the extensions'.
//!
//! CONVERSATIONS (host-ops, §3a): a call is a strictly LIFO conversation, not a
//! single round trip. While the caller pumps toward its terminal, the worker
//! may send host-op `Call`s on parent-held handles — block arguments that
//! crossed as `Arg::Handle` — which are serviced HERE, on the caller's own
//! fiber. A send that servicing code makes back into the same worker is a
//! NESTED call riding the open conversation (§5.1 rule 3 with one lane; the
//! `active` record is that rule's N=1 form, absorbed by the claim machinery in
//! the mailboxes+lanes slice). Cancellation mid-conversation ABANDONS it: the
//! channels drop, the worker (or the process pump) answers its own pending
//! host-ops with errors and unwinds to the terminal, and the service stays
//! usable — unlike extensions, whose framed socket desyncs and kills the peer.
//!
//! Errors in the hosted method — including MessageNotUnderstood — come back as
//! `CallReturnError` and raise catchably at the call site, carrying the
//! worker's rendered stack as `ex.remoteStack` (the extension error shape).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc as StdArc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use gc_arena::Collect;
use gc_arena::collect::Trace;
use quoin_ext_proto::{Arg, DataValue as WireData, Msg};

use crate::error::QuoinError;
use crate::fiber::YieldReason;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::claims::{Acquire, PeerClaims, WaitKind, would_deadlock};
use crate::runtime::extension::{
    BoundaryStats, record_boundary_row, truncate_blob, value_to_wire, wire_to_value,
};
use crate::runtime::worker::block_parts;
use crate::symbol::Symbol;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;
use crate::vm_scheduler::{TaskId, Wake};
use crate::worker::{
    DispatchReq, OP_STOP, PortableBlock, note_message, rebuild_portable_value, snapshot_block,
    spawn_worker_service,
};

/// Proxy-side state: the worker's dispatch lane plus this proxy's hosted-object
/// id. Everything worker-wide (dispatch lane, claims, stop flag, reap queue,
/// open conversations) is shared by every proxy of the worker; only
/// `object_id`/`class_name` are per-proxy.
#[derive(Debug)]
pub struct NativeServiceState {
    dispatch_tx: async_channel::Sender<DispatchReq>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
    /// The §5.1 claim machinery: per-object mailboxes + the lane pool.
    /// Registered in `vm.io.claim_peers` (`VM.claims`, cycle walks).
    claims: Rc<RefCell<PeerClaims>>,
    /// Lane count (in-flight conversation bound; also how many stop ops
    /// `serviceStop` sends — one per worker serve fiber).
    lanes: u32,
    /// Worker-wide stop flag — a stopped service refuses calls from every proxy.
    stopped: Rc<Cell<bool>>,
    /// Dropped-proxy ids awaiting flush as `Call.releases` (the reap pattern:
    /// a GC `Drop` can't send a frame).
    reap: Rc<RefCell<Vec<u64>>>,
    /// This proxy's hosted-object id (the root instance is 1).
    object_id: u64,
    /// The hosted object's class name — routes the dispatch worker-side.
    class_name: String,
    /// True for process backing: block arguments take the handle path (no
    /// shipping — templates are in-process references; ACTOR_OBJECTS.md §3a).
    process: bool,
    /// The conversation each task currently has open on this worker: a send
    /// from a task that is servicing a host-op rides its own conversation as
    /// a NESTED call (§5.1 rule 3) instead of deadlocking behind itself.
    convs: Rc<RefCell<HashMap<usize, ActiveConv>>>,
    /// Parent-side hosted-table ids minted for block arguments that crossed
    /// as handles; released when the service stops (a stored `HostBlock` in
    /// the worker may be invoked by any later call, so per-call release would
    /// be wrong — the reap pattern doesn't reach worker-held handles yet).
    block_handles: Rc<RefCell<Vec<u64>>>,
    /// Boundary-profiling rows (§7), registered in `vm.io.ext_stats` beside
    /// the extensions' — one diagnosis surface.
    boundary: Rc<RefCell<BoundaryStats>>,
    /// This link's index in `vm.io.chan_links` (§6 channel relay).
    chan_link: usize,
}

/// The channels of the conversation a task currently has open on a worker:
/// nested sends push their `Call` down `hostop_tx` and read frames (their
/// terminal, and any deeper host-ops) from `reply_rx` — strictly LIFO, all on
/// the one fiber that owns the conversation.
#[derive(Clone, Debug)]
struct ActiveConv {
    depth: u32,
    hostop_tx: async_channel::Sender<Msg>,
    reply_rx: async_channel::Receiver<Msg>,
}

/// The most deeply one task may nest calls on a worker conversation (mirrors
/// the extension cap; each level is live frames on both sides).
const MAX_CONV_DEPTH: u32 = 16;

impl Drop for NativeServiceState {
    fn drop(&mut self) {
        self.reap.borrow_mut().push(self.object_id);
    }
}

impl AnyCollect for NativeServiceState {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

unsafe impl<'gc> Collect<'gc> for NativeServiceState {
    const NEEDS_TRACE: bool = false;
}

/// The per-call snapshot of a proxy's state (cloned out so the native-state
/// borrow ends before any park).
struct CallCtx {
    dispatch_tx: async_channel::Sender<DispatchReq>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
    claims: Rc<RefCell<PeerClaims>>,
    lanes: u32,
    stopped: Rc<Cell<bool>>,
    reap: Rc<RefCell<Vec<u64>>>,
    object_id: u64,
    class_name: String,
    process: bool,
    convs: Rc<RefCell<HashMap<usize, ActiveConv>>>,
    block_handles: Rc<RefCell<Vec<u64>>>,
    boundary: Rc<RefCell<BoundaryStats>>,
    chan_link: usize,
}

fn snapshot(s: &NativeServiceState) -> CallCtx {
    CallCtx {
        dispatch_tx: s.dispatch_tx.clone(),
        done_rx: s.done_rx.clone(),
        claims: s.claims.clone(),
        lanes: s.lanes,
        stopped: s.stopped.clone(),
        reap: s.reap.clone(),
        object_id: s.object_id,
        class_name: s.class_name.clone(),
        process: s.process,
        convs: s.convs.clone(),
        block_handles: s.block_handles.clone(),
        boundary: s.boundary.clone(),
        chan_link: s.chan_link,
    }
}

/// The dispatch MNU-seam hook (see `exec_send` / `call_method_cached_inner`):
/// `None` means "not a service proxy — raise the MNU as usual".
pub(crate) fn try_service_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    selector: Symbol,
    args: &[Value<'gc>],
) -> Option<Result<Value<'gc>, QuoinError>> {
    let ctx = receiver
        .with_native_state::<NativeServiceState, _, _>(snapshot)
        .ok()?;
    Some(service_call(vm, mc, receiver, ctx, selector, args))
}

/// A bare `Call` frame for a hosted-object dispatch.
fn hosted_call(ctx: &CallCtx, op: String, method_args: Vec<Arg>, releases: Vec<u64>) -> Msg {
    Msg::Call {
        op,
        arg: String::new(),
        handles: Vec::new(),
        resources: Vec::new(),
        releases,
        arrays: Vec::new(),
        data: None,
        class_name: ctx.class_name.clone(),
        recv: ctx.object_id,
        method_args,
    }
}

fn service_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    ctx: CallCtx,
    selector: Symbol,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    // A send from a task with an OPEN conversation on this worker is a NESTED
    // call: it rides that conversation (§5.1 rule 3) rather than parking on a
    // lane it transitively holds. Checked before the stopped flag — stop is
    // flag + drain, and a nested call is part of an in-flight conversation,
    // which drain lets finish.
    let me = vm.sched.current_task.0;
    let nested = ctx.convs.borrow().contains_key(&me);
    if !nested && ctx.stopped.get() {
        return Err(QuoinError::Other(format!(
            "service call '{}': the service is stopped",
            selector.as_str()
        )));
    }
    // Encode BEFORE claiming: a refused argument shouldn't occupy the
    // service. A proxy of the SAME worker travels as a live reference; a
    // portable BLOCK ships to a thread peer as a capture snapshot riding the
    // dispatch request out-of-band (§3a); any other block crosses as a HANDLE
    // the worker drives via host-op round trips (§3a fallback — never an
    // error). Nested frames have no sidecar, so nested sends use handles for
    // every block.
    let mut method_args = Vec::with_capacity(args.len());
    let mut blocks: Vec<(usize, PortableBlock)> = Vec::new();
    let mut chans: Vec<(usize, u64)> = Vec::new();
    for (i, a) in args.iter().enumerate() {
        let same_worker_id = a
            .with_native_state::<NativeServiceState, _, _>(|s| {
                Rc::ptr_eq(&s.reap, &ctx.reap).then_some(s.object_id)
            })
            .ok()
            .flatten();
        if let Some(id) = same_worker_id {
            method_args.push(Arg::Resource(id));
            continue;
        }
        // A CHANNEL argument ships as a live relay endpoint (§6) — its sends
        // and receives in the worker relay back to this side.
        if crate::runtime::channel_relay::is_channel_value(*a) {
            let chan = crate::runtime::channel_relay::ship_for_crossing(vm, mc, *a, ctx.chan_link)
                .map_err(|e| {
                    QuoinError::Other(format!(
                        "service call '{}': argument {}: {e}",
                        selector.as_str(),
                        i + 1
                    ))
                })?;
            chans.push((i, chan));
            method_args.push(Arg::Data(WireData::Null));
            continue;
        }
        if let Some((template, parent_env)) = block_parts(*a) {
            if !ctx.process
                && !nested
                && let Ok(pb) = snapshot_block(template.clone(), parent_env, 0)
            {
                blocks.push((i, pb));
                method_args.push(Arg::Data(WireData::Null));
                continue;
            }
            // Handle path. A PORTABLE block headed here (process backing, or a
            // nested frame) is snapshot-rebuilt locally first, so its captures
            // freeze at send time exactly as shipping would — the ship/handle
            // choice must not change semantics for blocks that have a choice.
            // An unportable block is handled live (write-captures exist to see
            // live state; that is why it could not ship).
            let handled: Value<'gc> = match snapshot_block(template, parent_env, 0) {
                Ok(pb) => rebuild_portable_value(vm, mc, &pb).map_err(|e| {
                    QuoinError::Other(format!(
                        "service call '{}': argument {}: {e}",
                        selector.as_str(),
                        i + 1
                    ))
                })?,
                Err(_) => *a,
            };
            let id = vm.hosted_insert(handled);
            ctx.block_handles.borrow_mut().push(id);
            method_args.push(Arg::Handle(id));
            continue;
        }
        method_args.push(Arg::Data(value_to_wire(*a, None).map_err(|e| {
            QuoinError::Other(format!(
                "service call '{}': argument {} is not portable: {e}",
                selector.as_str(),
                i + 1
            ))
        })?));
    }
    let releases: Vec<u64> = ctx.reap.borrow_mut().drain(..).collect();
    let frame = hosted_call(&ctx, selector.as_str().to_string(), method_args, releases);

    if nested {
        if !chans.is_empty() {
            return Err(QuoinError::Other(format!(
                "service call '{}': channels cannot cross on a nested call yet — \
                 pass them in a top-level call",
                selector.as_str()
            )));
        }
        return nested_call(vm, mc, receiver, &ctx, selector, frame);
    }

    // Claim (object, lane) jointly and atomically (§5.1 rule 2): granted both
    // or parked wanting both — a queued caller holds NOTHING while it waits.
    let claim = claim_object(vm, receiver, &ctx, selector, WaitKind::TopLevel)?;
    note_message();
    let started = Instant::now();
    let handler = StdArc::new(AtomicU64::new(0));
    let (reply_tx, reply_rx) = async_channel::unbounded::<Msg>();
    let (hostop_tx, hostop_rx) = async_channel::unbounded::<Msg>();
    if ctx
        .dispatch_tx
        .try_send(DispatchReq {
            frame,
            blocks,
            chans,
            reply: reply_tx,
            hostops: hostop_rx,
            handler_micros: handler.clone(),
        })
        .is_err()
    {
        end_call_and_wake(vm, &ctx, me);
        return Err(QuoinError::Other(format!(
            "service call '{}': the service has exited",
            selector.as_str()
        )));
    }
    // Open the conversation (nested sends from host-op callbacks ride it),
    // pump it to the terminal, then close it and release the claim — on the
    // error path too (cancellation mid-conversation): dropping our channel
    // ends tells the worker the conversation was abandoned, and it unwinds
    // catchably rather than wedging.
    ctx.convs.borrow_mut().insert(
        me,
        ActiveConv {
            depth: 1,
            hostop_tx: hostop_tx.clone(),
            reply_rx: reply_rx.clone(),
        },
    );
    let outcome = conversation(vm, mc, selector, ctx.chan_link, &reply_rx, &hostop_tx);
    ctx.convs.borrow_mut().remove(&me);
    end_call_and_wake(vm, &ctx, me);
    record_boundary_row(
        &ctx.boundary,
        &ctx.class_name,
        selector.as_str(),
        started.elapsed().as_micros() as u64,
        claim.wait_micros,
        handler.load(Ordering::Relaxed),
        outcome.is_err() || matches!(outcome, Ok(Msg::CallReturnError { .. })),
    );
    interpret_terminal(vm, mc, receiver, &ctx, selector, outcome?)
}

/// The outcome of a successful claim: which lane came with it (top-level
/// grants) and how long the caller queued.
struct Claimed {
    wait_micros: u64,
}

/// Acquire the receiver's object claim per §5.1 — jointly with a lane for a
/// top-level send, object-only for a nested one — parking FIFO when
/// contended, with the rule-6 cycle walk run before every park: a walk that
/// closes on this task raises catchably instead of parking (the deadlock
/// lands at the task that closes the cycle, by decision).
fn claim_object<'gc>(
    vm: &mut VmState<'gc>,
    receiver: Value<'gc>,
    ctx: &CallCtx,
    selector: Symbol,
    kind: WaitKind,
) -> Result<Claimed, QuoinError> {
    let me = vm.sched.current_task;
    let epoch = vm.current_park_epoch();
    let nested = kind == WaitKind::Nested;
    let decision =
        ctx.claims
            .borrow_mut()
            .try_acquire(me.0, epoch, ctx.object_id, &ctx.class_name, nested);
    let blocker = match decision {
        Acquire::Granted { .. } | Acquire::Reentrant => {
            return Ok(Claimed { wait_micros: 0 });
        }
        Acquire::TooDeep => {
            return Err(QuoinError::Other(format!(
                "service call '{}': re-entered {} too deeply — mutual recursion?",
                selector.as_str(),
                ctx.class_name
            )));
        }
        Acquire::WouldQueue { blocker } => blocker,
    };
    // Rule 6: would parking close a waits-for cycle? (Only an owned object
    // can extend the walk; a reservation blocks on a task that holds nothing.)
    if let Some(start) = blocker {
        let registry = vm.io.claim_peers.clone();
        let label = format!("{}#{}", ctx.class_name, ctx.object_id);
        let cycle = {
            let mut live = |t: usize, e: u64| vm.channel_waiter_live(TaskId(t), e);
            would_deadlock(&registry, me.0, start, &label, &mut live)
        };
        if let Some(cycle) = cycle {
            ctx.claims.borrow_mut().stats.deadlocks += 1;
            return Err(QuoinError::Other(format!(
                "service call '{}': {cycle}",
                selector.as_str()
            )));
        }
    }
    ctx.claims
        .borrow_mut()
        .enqueue(me.0, epoch, ctx.object_id, &ctx.class_name, kind);
    // Park until the finishing call HANDS us the claim (fair FIFO — the
    // ext_prelude park verbatim). The wait is boundary-profiled separately:
    // mailbox contention is its own diagnosis (§7).
    let queued_at = Instant::now();
    if let Some(t) = vm.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
        t.parked_on_channel = true;
    }
    vm.set_park_info("service claim".to_string(), Some(receiver));
    if let Some(yielder) = unsafe { vm.get_yielder() } {
        yielder.suspend(YieldReason::ChannelPark);
    } else {
        ctx.claims.borrow_mut().retract(me.0, ctx.object_id);
        return Err(QuoinError::Other(
            "service call queued outside the VM scheduler".to_string(),
        ));
    }
    // On resume: if the claim was already handed to us and a cancel raced in,
    // pass it onward (mirrors ext_prelude) — never strand the queue.
    let handed = matches!(vm.sched.wake.take(), Some(Wake::ServiceClaim { .. }));
    if vm.sched.cancel_current {
        if handed {
            end_call_and_wake(vm, ctx, me.0);
        } else {
            ctx.claims.borrow_mut().retract(me.0, ctx.object_id);
        }
        return Err(vm.take_cancellation());
    }
    if !handed {
        return Err(QuoinError::Other(
            "service claim park resumed without the claim".to_string(),
        ));
    }
    Ok(Claimed {
        wait_micros: queued_at.elapsed().as_micros() as u64,
    })
}

/// Release one call's claim on the proxy's object (outermost release frees
/// the lane too) and deliver the resulting handoffs.
fn end_call_and_wake<'gc>(vm: &mut VmState<'gc>, ctx: &CallCtx, task: usize) {
    let grants = {
        let mut live = |t: usize, e: u64| vm.channel_waiter_live(TaskId(t), e);
        ctx.claims
            .borrow_mut()
            .end_call(task, ctx.object_id, &mut live)
    };
    for g in grants {
        vm.wake_channel_task(TaskId(g.task), Wake::ServiceClaim { lane: g.lane });
    }
}

/// A nested call riding the task's open conversation: LIFO — the frame goes
/// down the parent→worker lane, and the reply pump below reads worker→parent
/// frames until this call's terminal (servicing any deeper host-ops on the
/// way), all within the outer conversation's stack. The target object's claim
/// is acquired object-only (§5.1 rule 3: a nested send never waits for a
/// lane); same-owner re-entry nests depth-capped (rule 4).
fn nested_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    ctx: &CallCtx,
    selector: Symbol,
    frame: Msg,
) -> Result<Value<'gc>, QuoinError> {
    let me = vm.sched.current_task.0;
    let claim = claim_object(vm, receiver, ctx, selector, WaitKind::Nested)?;
    let conv = {
        let mut convs = ctx.convs.borrow_mut();
        let Some(conv) = convs.get_mut(&me) else {
            end_call_and_wake(vm, ctx, me);
            return Err(QuoinError::Other(format!(
                "service call '{}': the conversation closed mid-call",
                selector.as_str()
            )));
        };
        if conv.depth >= MAX_CONV_DEPTH {
            let cap = MAX_CONV_DEPTH;
            end_call_and_wake(vm, ctx, me);
            return Err(QuoinError::Other(format!(
                "service call '{}': calls nested {cap} levels deep on one \
                 conversation — mutual parent<->worker recursion?",
                selector.as_str()
            )));
        }
        conv.depth += 1;
        conv.clone()
    };
    note_message();
    let started = Instant::now();
    let outcome = if conv.hostop_tx.try_send(frame).is_err() {
        Err(QuoinError::Other(format!(
            "service call '{}': the service exited mid-call",
            selector.as_str()
        )))
    } else {
        conversation(
            vm,
            mc,
            selector,
            ctx.chan_link,
            &conv.reply_rx,
            &conv.hostop_tx,
        )
    };
    if let Some(c) = ctx.convs.borrow_mut().get_mut(&me) {
        c.depth = c.depth.saturating_sub(1);
    }
    end_call_and_wake(vm, ctx, me);
    record_boundary_row(
        &ctx.boundary,
        &ctx.class_name,
        selector.as_str(),
        started.elapsed().as_micros() as u64,
        claim.wait_micros,
        0,
        outcome.is_err() || matches!(outcome, Ok(Msg::CallReturnError { .. })),
    );
    interpret_terminal(vm, mc, receiver, ctx, selector, outcome?)
}

/// Pump one conversation level to its terminal: worker→parent frames are
/// either host-op `Call`s on a parent-held handle — serviced HERE, on the
/// calling task's own fiber (so claims and stacks compose; §5.1) — or the
/// `CallReturn*` terminal this level is waiting for.
fn conversation<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    selector: Symbol,
    chan_link: usize,
    reply_rx: &async_channel::Receiver<Msg>,
    hostop_tx: &async_channel::Sender<Msg>,
) -> Result<Msg, QuoinError> {
    loop {
        let msg = match vm.await_io(IoRequest::FrameRecv(reply_rx.clone()))? {
            IoResult::FrameMsg(Some(msg)) => *msg,
            IoResult::FrameMsg(None) => {
                return Err(QuoinError::Other(format!(
                    "service call '{}': the service exited mid-call",
                    selector.as_str()
                )));
            }
            other => {
                return Err(QuoinError::Other(format!(
                    "service call '{}': unexpected result {other:?}",
                    selector.as_str()
                )));
            }
        };
        match msg {
            Msg::Call {
                op,
                recv,
                method_args,
                ..
            } => {
                let reply = service_parent_hostop(vm, mc, chan_link, &op, recv, &method_args)?;
                if hostop_tx.try_send(reply).is_err() {
                    return Err(QuoinError::Other(format!(
                        "service call '{}': the service exited mid-call",
                        selector.as_str()
                    )));
                }
            }
            terminal => return Ok(terminal),
        }
    }
}

/// Service one host-op the worker issued mid-call: a `Call` on a parent-held
/// handle (a block that crossed as `Arg::Handle`). The block runs on THIS
/// task's fiber — a send it makes back into the service is a nested call on
/// the open conversation. Errors become error frames for the worker, EXCEPT a
/// cancellation (`Async.timeout:do:`, task cancel), which re-raises unchanged
/// — abandoning the conversation, exactly as the extension path treats it —
/// so the timeout combinator still sees its `Cancelled`.
fn service_parent_hostop<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    chan_link: usize,
    op: &str,
    recv: u64,
    method_args: &[Arg],
) -> Result<Msg, QuoinError> {
    let err = |message: String| Msg::CallReturnError {
        message,
        remote_stack: String::new(),
    };
    let Some(target) = vm.hosted_get(recv) else {
        return Ok(err(format!("host block '{op}': no live handle {recv}")));
    };
    let mut argv = Vec::with_capacity(method_args.len());
    for (i, a) in method_args.iter().enumerate() {
        match a {
            Arg::Data(dv) => match wire_to_value(vm, mc, dv, None) {
                Ok(v) => argv.push(v),
                Err(e) => return Ok(err(format!("host block '{op}': argument {}: {e}", i + 1))),
            },
            _ => {
                return Ok(err(format!(
                    "host block '{op}': argument {} has an unsupported kind",
                    i + 1
                )));
            }
        }
    }
    match vm.call_method_mnu(mc, target, op, argv) {
        Ok(v) => match value_to_wire(v, None) {
            Ok(dv) => Ok(Msg::CallReturnData { value: dv }),
            Err(e) => {
                // A block answering a CHANNEL ships it as a live endpoint (§6).
                if crate::runtime::channel_relay::is_channel_value(v) {
                    return match crate::runtime::channel_relay::ship_for_crossing(
                        vm, mc, v, chan_link,
                    ) {
                        Ok(chan) => Ok(Msg::CallReturnChannel { chan }),
                        Err(e) => Ok(err(format!("host block '{op}': {e}"))),
                    };
                }
                Ok(err(format!(
                    "host block '{op}': the block's result cannot cross to the worker: {e}"
                )))
            }
        },
        Err(QuoinError::Cancelled) => Err(QuoinError::Cancelled),
        Err(e) => Ok(crate::runtime::worker::error_terminal(vm, &e, "parent")),
    }
}

/// Materialize a `CallReturn*` terminal: data through the wire walkers, a hosted
/// resource as a SUB-PROXY of the same worker, an error as the extension error
/// shape (message + `ex.remoteStack`).
fn interpret_terminal<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    ctx: &CallCtx,
    selector: Symbol,
    msg: Msg,
) -> Result<Value<'gc>, QuoinError> {
    match msg {
        Msg::CallReturnData { value } => wire_to_value(vm, mc, &value, None),
        // A WORKER-owned channel comes back as a live relay endpoint (§6).
        Msg::CallReturnChannel { chan } => {
            crate::runtime::channel_relay::relay_endpoint(vm, mc, ctx.chan_link, chan)
        }
        Msg::CallReturnResource {
            resource,
            class_name,
        } => {
            let Value::Object(obj) = receiver else {
                return Err(QuoinError::Other(format!(
                    "service call '{}': bad proxy receiver",
                    selector.as_str()
                )));
            };
            let class = obj.borrow().class;
            Ok(vm.new_native_state(
                mc,
                class,
                NativeServiceState {
                    dispatch_tx: ctx.dispatch_tx.clone(),
                    done_rx: ctx.done_rx.clone(),
                    claims: ctx.claims.clone(),
                    lanes: ctx.lanes,
                    stopped: ctx.stopped.clone(),
                    reap: ctx.reap.clone(),
                    object_id: resource,
                    class_name,
                    process: ctx.process,
                    convs: ctx.convs.clone(),
                    block_handles: ctx.block_handles.clone(),
                    boundary: ctx.boundary.clone(),
                    chan_link: ctx.chan_link,
                },
            ))
        }
        Msg::CallReturnError {
            message,
            remote_stack,
        } => Err(QuoinError::ExtensionError {
            message,
            remote_stack: truncate_blob(remote_stack),
        }),
        other => Err(QuoinError::Other(format!(
            "service call '{}': unexpected terminal {other:?}",
            selector.as_str()
        ))),
    }
}

fn host<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    path: String,
    class_name: String,
    backing: &'static str,
    lanes: u32,
) -> Result<Value<'gc>, QuoinError> {
    let Value::Class(class) = receiver else {
        return Err(QuoinError::Other("WorkerService: bad receiver".into()));
    };
    let (ch, pid) = match backing {
        "process" => {
            let (ch, pid, _grip) =
                crate::worker::spawn_worker_process(path.clone(), Some(class_name.clone()), lanes)
                    .map_err(QuoinError::Other)?;
            (ch, Some(pid))
        }
        _ => (
            spawn_worker_service(path.clone(), class_name.clone(), lanes),
            None,
        ),
    };
    // Disambiguate the display label when several services host the same
    // unit: the first keeps the bare label, later ones gain an ordinal —
    // otherwise their VM.claims / boundary / ps rows are indistinguishable.
    let label = {
        let base = format!("svc:{path}");
        let dupes = vm
            .io
            .claim_peers
            .borrow()
            .iter()
            .filter(|p| {
                let l = &p.borrow().label;
                *l == base || l.starts_with(&format!("{base}#"))
            })
            .count();
        if dupes == 0 {
            base
        } else {
            format!("{base}#{}", dupes + 1)
        }
    };
    vm.worker_registry.push(crate::worker::WorkerReg {
        unit: format!("svc:{path}"),
        label: label.clone(),
        backing,
        pid,
        inbox_tx: ch.inbox_tx.clone(),
        outbox_rx: ch.outbox_rx.clone(),
        control_tx: ch.control_tx.clone(),
    });
    // Handshake: the serve loop's first act is a 'ready' message; a closed lane
    // instead means boot/compile/instantiation failed — the done lane says
    // why. Parks, so slow boots don't block other tasks.
    match vm.await_io(IoRequest::WorkerRecv(ch.outbox_rx.clone()))? {
        IoResult::WorkerMsg(Some(_)) => {}
        IoResult::WorkerMsg(None) => {
            let why = match vm.await_io(IoRequest::WorkerJoin(ch.done_rx.clone()))? {
                IoResult::WorkerDone(Err(msg)) => msg,
                _ => "the service exited before reporting ready".to_string(),
            };
            return Err(QuoinError::Other(format!("WorkerService.host: {why}")));
        }
        other => {
            return Err(QuoinError::Other(format!(
                "WorkerService.host: unexpected result {other:?}"
            )));
        }
    }
    // Claims: the §5.1 machinery, one per worker, registered for VM.claims
    // and the cross-peer cycle walk; boundary rows registered beside the
    // extensions' (§7 — one diagnosis surface).
    let claims = Rc::new(RefCell::new(PeerClaims::new(label.clone(), lanes)));
    vm.io.claim_peers.borrow_mut().push(claims.clone());
    let boundary = Rc::new(RefCell::new(BoundaryStats {
        peer: label,
        rows: HashMap::new(),
    }));
    vm.io.ext_stats.borrow_mut().push(boundary.clone());
    let chan_link = crate::runtime::channel_relay::register_chan_link(
        vm,
        ch.chan_tx.clone(),
        ch.chan_rx.clone(),
        backing == "process",
    );
    Ok(vm.new_native_state(
        mc,
        class,
        NativeServiceState {
            dispatch_tx: ch.dispatch_tx,
            done_rx: ch.done_rx,
            claims,
            lanes: lanes.max(1),
            stopped: Rc::new(Cell::new(false)),
            reap: Rc::new(RefCell::new(Vec::new())),
            object_id: 1,
            class_name,
            process: backing == "process",
            convs: Rc::new(RefCell::new(HashMap::new())),
            block_handles: Rc::new(RefCell::new(Vec::new())),
            boundary,
            chan_link,
        },
    ))
}

/// `serviceStop`'s drain: park until every lane is home (each in-flight
/// conversation finished). New top-level sends are already refused by the
/// stopped flag, so the wait is bounded by running work.
fn drain_lanes<'gc>(
    vm: &mut VmState<'gc>,
    receiver: Value<'gc>,
    ctx: &CallCtx,
) -> Result<(), QuoinError> {
    let me = vm.sched.current_task;
    let epoch = vm.current_park_epoch();
    if ctx.claims.borrow_mut().request_drain(me.0, epoch) {
        return Ok(());
    }
    if let Some(t) = vm.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
        t.parked_on_channel = true;
    }
    vm.set_park_info("service drain".to_string(), Some(receiver));
    if let Some(yielder) = unsafe { vm.get_yielder() } {
        yielder.suspend(YieldReason::ChannelPark);
    } else {
        return Err(QuoinError::Other(
            "service drain outside the VM scheduler".to_string(),
        ));
    }
    let woken = matches!(vm.sched.wake.take(), Some(Wake::ServiceClaim { .. }));
    if vm.sched.cancel_current {
        return Err(vm.take_cancellation());
    }
    if !woken {
        return Err(QuoinError::Other(
            "service drain resumed without a wake".to_string(),
        ));
    }
    Ok(())
}

fn backing_arg<'gc>(v: Value<'gc>) -> Result<&'static str, QuoinError> {
    match string_arg(v, "the backing")?.as_str() {
        "thread" => Ok("thread"),
        "process" => Ok("process"),
        other => Err(QuoinError::Other(format!(
            "WorkerService: unknown backing '{other}' (thread|process)"
        ))),
    }
}

fn lanes_arg<'gc>(v: Value<'gc>) -> Result<u32, QuoinError> {
    match v.as_i64() {
        Some(n) if (1..=1024).contains(&n) => Ok(n as u32),
        _ => Err(QuoinError::Other(
            "WorkerService: lanes must be an Integer between 1 and 1024".into(),
        )),
    }
}

fn string_arg<'gc>(v: Value<'gc>, what: &str) -> Result<String, QuoinError> {
    match v {
        Value::Object(obj) => match &obj.borrow().payload {
            crate::value::ObjectPayload::String(s) => Ok((**s).clone()),
            _ => Err(QuoinError::Other(format!(
                "WorkerService: {what} must be a String"
            ))),
        },
        _ => Err(QuoinError::Other(format!(
            "WorkerService: {what} must be a String"
        ))),
    }
}

pub fn build_worker_service_class() -> NativeClassBuilder {
    NativeClassBuilder::new("WorkerService", Some("Object"))
        .construct_with("use WorkerService.host:class:")
        .class_doc(
            "Host a class in a dedicated worker isolate and get a PROXY whose ordinary \
             method sends become peer-protocol calls: sticky state with serialized \
             access -- an actor, effectively. Portable arguments and returns deep-copy; \
             a method that returns a NON-portable object HOSTS it -- the answer is a \
             sub-proxy addressing it, usable like any receiver (including as an argument \
             to further calls on the same service). A BLOCK argument always crosses: a \
             portable block ships to a thread-backed service and runs worker-side on a \
             snapshot of its captures (one crossing however many invocations); any other \
             block -- unportable, or bound for a process -- crosses as a HANDLE the worker \
             invokes back in the parent, where write-captures see live state. Code the \
             worker runs this way may call back into the service; the nested call rides \
             the open conversation. Errors in the hosted method raise \
             catchably at the call site, with the worker's stack as `ex.remoteStack`; \
             one call runs at a time (concurrent callers queue fairly).\n\n\
             ```\n\
             var index = WorkerService.host:'search/index.qn' class:'SearchIndex';\n\
             index.add:doc;\n\
             var hits = index.query:'quoin'\n\
             ```",
        )
        .class_method("host:class:", |vm, mc, receiver, args| {
            let path = string_arg(args[0], "the unit path")?;
            let class_name = string_arg(args[1], "the class name")?;
            host(vm, mc, receiver, path, class_name, "thread", 1)
        })
        .doc(
            "Spawn a worker running the unit at the path, instantiate the named class in it \
             (`TheClass.new`), and answer the proxy once the service reports ready. Every \
             selector the proxy doesn't define itself (everything except `serviceStop`) \
             forwards as a call and parks for the reply, so calls compose with \
             `Async.gather:` / `timeout:do:` like any parked wait.",
        )
        .class_method("host:class:lanes:", |vm, mc, receiver, args| {
            let path = string_arg(args[0], "the unit path")?;
            let class_name = string_arg(args[1], "the class name")?;
            let lanes = lanes_arg(args[2])?;
            host(vm, mc, receiver, path, class_name, "thread", lanes)
        })
        .doc(
            "As `host:class:` with N concurrent lanes (docs/internal/ACTOR_OBJECTS.md \
             \u{a7}5): calls to DIFFERENT objects of the service overlap, up to N in \
             flight; calls to one object still serialize (its mailbox). Worker-side, \
             each lane is a cooperative fiber -- an object parked on IO doesn't block \
             its isolate-mates.",
        )
        .class_method("host:class:backing:", |vm, mc, receiver, args| {
            let path = string_arg(args[0], "the unit path")?;
            let class_name = string_arg(args[1], "the class name")?;
            let backing = backing_arg(args[2])?;
            host(vm, mc, receiver, path, class_name, backing, 1)
        })
        .doc(
            "As `host:class:`, choosing the backing at spawn time: 'thread' (the default) \
             or 'process' (a child qn process -- the escape from the in-process thread \
             ceiling for compute-heavy services).",
        )
        .class_method("host:class:backing:lanes:", |vm, mc, receiver, args| {
            let path = string_arg(args[0], "the unit path")?;
            let class_name = string_arg(args[1], "the class name")?;
            let backing = backing_arg(args[2])?;
            let lanes = lanes_arg(args[3])?;
            host(vm, mc, receiver, path, class_name, backing, lanes)
        })
        .doc(
            "Backing and lanes together: N concurrent lanes on either backing. A \
             process-backed service opens one conversation socket per lane; a \
             thread-backed one runs one serve fiber per lane. Semantics are identical \
             -- calls to one object serialize, calls to different objects overlap.",
        )
        // Stop the service: flag + drain (refuse new calls, wait for every
        // in-flight conversation), then one stop op per lane, then join.
        // Worker-wide: every proxy of the service refuses calls afterwards.
        .instance_method("serviceStop", |vm, mc, receiver, _args| {
            let ctx = receiver
                .with_native_state::<NativeServiceState, _, _>(|s| {
                    s.stopped.set(true);
                    snapshot(s)
                })
                .map_err(QuoinError::Other)?;
            drain_lanes(vm, receiver, &ctx)?;
            // One reserved stop op per lane fiber; a dead worker skips
            // straight to the join, which reports why.
            for _ in 0..ctx.lanes {
                let (reply_tx, reply_rx) = async_channel::bounded::<Msg>(1);
                let (_hostop_tx, hostop_rx) = async_channel::bounded::<Msg>(1);
                let frame = hosted_call(&ctx, OP_STOP.to_string(), Vec::new(), Vec::new());
                if ctx
                    .dispatch_tx
                    .try_send(DispatchReq {
                        frame,
                        blocks: Vec::new(),
                        chans: Vec::new(),
                        reply: reply_tx,
                        hostops: hostop_rx,
                        handler_micros: StdArc::new(AtomicU64::new(0)),
                    })
                    .is_err()
                {
                    break;
                }
                let _ = vm.await_io(IoRequest::FrameRecv(reply_rx))?;
            }
            let joined = vm.await_io(IoRequest::WorkerJoin(ctx.done_rx.clone()))?;
            // The worker is gone: release the block handles its stored
            // HostBlocks addressed (minted for block arguments that crossed
            // as handles).
            for id in ctx.block_handles.borrow_mut().drain(..) {
                vm.hosted_release(id);
            }
            match joined {
                IoResult::WorkerDone(Ok(_)) => Ok(vm.new_nil(mc)),
                IoResult::WorkerDone(Err(msg)) => Err(QuoinError::Other(msg)),
                other => Err(QuoinError::Other(format!(
                    "serviceStop: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Stop the service: wait for in-flight calls to finish, send the stop message, \
             and join the worker. Worker-wide -- further calls through ANY proxy of this \
             service raise 'the service is stopped'. Answers nil.",
        )
}
