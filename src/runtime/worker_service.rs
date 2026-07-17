//! Hosted objects on the peer protocol (docs/internal/ACTOR_OBJECTS.md §2; the
//! L4 of docs/internal/CONCURRENCY_ARCH.md §10, converged): a block evaluates
//! in a dedicated worker isolate, the object it answers is hosted, and the
//! caller gets a PROXY whose ordinary method sends become peer-protocol
//! `Call` frames. Sticky state, serialized access — an actor.
//!
//! ```text
//! var index = Worker.host:'search/index.qn' with:{ SearchIndex.new };
//! index.add:doc;
//! var hits = index.query:'quoin';
//! ```
//!
//! Proxies are REAL INSTALLED CLASSES (ACTOR_OBJECTS.md §2 manifests): the
//! worker's ready message carries the hosted class's selector manifest, the
//! parent installs a class of `ServiceDispatch` nodes from it (the
//! `install_ext_class` pattern, unbound — the parent's own globals are never
//! touched), and every send is ordinary method lookup landing in
//! `dispatch_service_method` — no VM dispatch hook. A selector outside the
//! manifest is an honest MessageNotUnderstood. Classes the worker never
//! declared up front install LAZILY when their first instance crosses
//! (`CallReturnResourceDecl`). Calls park, so they compose with
//! `Async.gather:`/`timeout:do:` like any parked wait.
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

use crate::error::{PeerDeathReason, QuoinError};
use crate::fiber::YieldReason;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::claims::{Acquire, PeerClaims, WaitKind, would_deadlock};
use crate::runtime::extension::{
    BoundaryStats, record_boundary_row, truncate_blob, value_to_wire, wire_to_value,
};
use crate::runtime::worker::block_parts;
use crate::symbol::Symbol;
use crate::value::{AnyCollect, Value};
use crate::vm::VmState;
use crate::vm::scheduler::{TaskId, Wake};
use crate::worker::{
    DispatchReq, OP_STOP, PortableBlock, WorkerExit, note_message, rebuild_portable_value,
    snapshot_block,
};

/// Proxy-side state: the worker's dispatch lane plus this proxy's hosted-object
/// id. Everything worker-wide (dispatch lane, claims, stop flag, reap queue,
/// open conversations) is shared by every proxy of the worker; only
/// `object_id`/`class_name` are per-proxy.
#[derive(Debug)]
pub struct NativeServiceState {
    dispatch_tx: async_channel::Sender<DispatchReq>,
    done_rx: async_channel::Receiver<Result<WireData, WorkerExit>>,
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
    /// This worker's index in `vm.io.lives` (SUPERVISION.md slice 1) —
    /// worker-wide, shared by every proxy: `serviceEvents` reaches the sink
    /// through it.
    life_idx: usize,
    /// Worker-wide CURRENT incarnation (SUPERVISION.md slice 2), bumped by a
    /// successful `serviceRestart`.
    incarnation: Rc<Cell<u64>>,
    /// The incarnation this proxy was minted under: a mismatch with the
    /// current cell is rule-6 staleness — the typed `#staleIncarnation`
    /// death. Only the root proxy re-stamps at restart.
    minted: u64,
    /// The rule-5 restart-window gate, worker-wide.
    restart: Rc<RefCell<RestartGate>>,
    /// The respawn recipe — ROOT PROXY ONLY (sub-proxies are incarnation
    /// state; they die with it and cannot restart anything).
    recipe: Option<Rc<ServiceRecipe>>,
    /// The supervision policy (SUPERVISION.md slice 3), attached post-spawn
    /// via `serviceSupervise:`. Worker-wide: `note_service_dead` consults it
    /// for the zero-gap gate close, and its presence hands restarts to the
    /// supervisor task (manual `serviceRestart` refuses).
    policy: Rc<RefCell<Option<crate::runtime::supervise::SupervisePolicy>>>,
    /// The user's restart hook (`serviceOnRestart:`) as a `vm.pins` ticket —
    /// ROOT PROXY ONLY, like the recipe. Runs inside every restart attempt,
    /// after the rebind and before the gate reopens; it outlives incarnations
    /// (rebind never touches it).
    hook: Cell<Option<crate::pin_table::PinId>>,
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

/// The frozen respawn recipe (SUPERVISION.md slice 2, §4 rule 2): everything
/// `serviceRestart` re-runs, retained at the ORIGINAL host — the block's
/// captures froze when it first shipped, and data/block args are the exact
/// messages sent then, re-sent verbatim. Channel args are the one GC-bound
/// part: their VALUES pin into `vm.pins` and re-ship against the new
/// incarnation's link. The manifest fields are the rule-9 equality gate: a
/// new incarnation must present the same class or the restart refuses to
/// rebind.
#[derive(Debug)]
struct ServiceRecipe {
    /// The original claims/registry label; incarnations >1 suffix it.
    label: String,
    path: Option<String>,
    pb: PortableBlock,
    lanes: u32,
    backing: &'static str,
    args: Vec<RecipeArg>,
    /// The channel-arg values' `vm.pins` tickets, pinned at the freeze;
    /// `RecipeArg::Channel` indexes here.
    chan_pins: Vec<crate::pin_table::PinId>,
    class_name: String,
    instance_selectors: Vec<String>,
    class_selectors: Vec<String>,
}

#[derive(Debug)]
enum RecipeArg {
    /// A data / portable-block arg: the spawn-time message, re-sent verbatim.
    Plain(crate::worker::WorkerMsg),
    /// A channel arg: index into the recipe's `chan_pins` — re-shipped as a
    /// VALUE so the fresh endpoint binds the new link.
    Channel(usize),
}

/// The rule-5 restart window: while a restart cycle is in flight, top-level
/// sends park here instead of dispatching into the corpse; completion wakes
/// them all — into the new incarnation on success, into the typed death on
/// failure. `GaveUp` (slice 3) is the permanent terminal: the policy's budget
/// is spent, and every sender — parked or future — gets the typed `#gaveUp`.
#[derive(Debug, Default)]
enum GatePhase {
    #[default]
    Open,
    Restarting,
    GaveUp {
        attempts: u32,
        last: String,
    },
}

#[derive(Debug, Default)]
struct RestartGate {
    phase: GatePhase,
    /// `(task, park epoch)` — epoch identity filters cancelled waiters.
    waiters: Vec<(usize, u64)>,
    /// The one task allowed through a `Restarting` gate: the restart hook's.
    /// The hook runs after the rebind (the transport is live) but before the
    /// gate reopens — its own sends to the service must dispatch, not park
    /// behind themselves.
    exempt: Option<usize>,
}

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
    done_rx: async_channel::Receiver<Result<WireData, WorkerExit>>,
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
    life_idx: usize,
    incarnation: Rc<Cell<u64>>,
    minted: u64,
    restart: Rc<RefCell<RestartGate>>,
    policy: Rc<RefCell<Option<crate::runtime::supervise::SupervisePolicy>>>,
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
        life_idx: s.life_idx,
        incarnation: s.incarnation.clone(),
        minted: s.minted,
        restart: s.restart.clone(),
        policy: s.policy.clone(),
    }
}

/// The `Callable::ServiceMethod` body (dispatch.rs): an installed service
/// class's method landed. An INSTANCE send takes its context from the
/// receiver's own state; a CLASS-SIDE send (the receiver is the installed
/// class itself) borrows the root proxy's state and dispatches with `recv 0`
/// — the hosted table's reserved class-side id.
pub(crate) fn dispatch_service_method<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    service: Value<'gc>,
    receiver: Value<'gc>,
    selector: Symbol,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    // SUPERVISION.md slice 2, BEFORE the state snapshot: rule-6 staleness
    // (a sub-proxy of a died incarnation raises the typed #staleIncarnation)
    // and the rule-5 restart window (a parked waiter re-snapshots on wake,
    // so it sees the rebound transport).
    let anchor = if receiver
        .with_native_state::<NativeServiceState, _, _>(|_| ())
        .is_ok()
    {
        receiver
    } else {
        service
    };
    await_restart_window(vm, anchor, selector)?;
    if let Ok(ctx) = receiver.with_native_state::<NativeServiceState, _, _>(snapshot) {
        return service_call(vm, mc, receiver, ctx, selector, &args);
    }
    // Class-side. NOTE a deliberate §5.1-rule-5 deviation: class-side sends
    // claim pseudo-object 0 (serializing them per service) rather than a lane
    // only — safer for class-state mutation, and the lane-only acquire can
    // land later without changing the surface.
    let mut ctx = service
        .with_native_state::<NativeServiceState, _, _>(snapshot)
        .map_err(QuoinError::Other)?;
    ctx.object_id = 0;
    service_call(vm, mc, service, ctx, selector, &args)
}

/// The proxy-owned `==:`: two proxies are equal iff they address the SAME
/// hosted object of the SAME service (worker identity = the shared reap Rc;
/// the worker-side table dedupes by identity, so ids compare faithfully).
fn service_eq<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let mine = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| (s.reap.clone(), s.object_id))
        .map_err(QuoinError::Other)?;
    let other = args.first().and_then(|a| {
        a.with_native_state::<NativeServiceState, _, _>(|s| (s.reap.clone(), s.object_id))
            .ok()
    });
    let eq = matches!(other, Some((reap, id)) if Rc::ptr_eq(&reap, &mine.0) && id == mine.1);
    Ok(vm.new_bool(mc, eq))
}

/// Find or install the service class for `(link, name)`: the shell is created
/// empty, registered, and populated from the manifest — the two-step that
/// breaks the proxy/class circularity (`anchor` is any live proxy of the
/// service; its state carries everything worker-wide).
pub(crate) fn installed_service_class<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    link: usize,
    name: &str,
    instance_selectors: &[String],
    class_selectors: &[String],
    anchor: Value<'gc>,
) -> gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::Class<'gc>>> {
    if let Some(entry) = vm
        .service_classes
        .iter()
        .find(|e| e.link == link && e.name == name)
        && let Value::Class(c) = entry.class
    {
        return c;
    }
    let shell = vm.make_service_class_shell(mc, name);
    vm.service_classes.push(crate::vm::ServiceClassEntry {
        link,
        name: name.to_string(),
        class: Value::Class(shell),
    });
    vm.populate_service_class(
        mc,
        shell,
        anchor,
        instance_selectors,
        class_selectors,
        &[
            ("serviceStop", service_stop),
            ("serviceEvents", service_events),
            ("serviceRestart", service_restart),
            ("serviceSupervise:", service_supervise),
            ("serviceOnRestart:", service_on_restart),
            ("==:", service_eq),
        ],
    );
    shell
}

/// Look up an already-installed service class (sub-proxy returns of an
/// announced class carry only the name).
fn lookup_service_class<'gc>(
    vm: &VmState<'gc>,
    link: usize,
    name: &str,
) -> Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::Class<'gc>>>> {
    vm.service_classes
        .iter()
        .find(|e| e.link == link && e.name == name)
        .and_then(|e| match e.class {
            Value::Class(c) => Some(c),
            _ => None,
        })
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
        // and receives in the worker relay back to this side. `Arg::Chan`
        // rides IN the frame (worker-only kind), so it crosses every carrier:
        // thread lanes, the process socket, and nested calls alike.
        if crate::runtime::channel_relay::is_channel_value(*a) {
            let chan = crate::runtime::channel_relay::ship_for_crossing(vm, mc, *a, ctx.chan_link)
                .map_err(|e| {
                    QuoinError::Other(format!(
                        "service call '{}': argument {}: {e}",
                        selector.as_str(),
                        i + 1
                    ))
                })?;
            method_args.push(Arg::Chan(chan));
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
            reply: reply_tx,
            hostops: hostop_rx,
            handler_micros: handler.clone(),
        })
        .is_err()
    {
        end_call_and_wake(vm, &ctx, me);
        note_service_dead(vm, &ctx, PeerDeathReason::Exited, "the service has exited");
        return Err(QuoinError::peer_died(
            ctx.class_name.clone(),
            PeerDeathReason::Exited,
            format!(
                "service call '{}': the service has exited",
                selector.as_str()
            ),
        ));
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
    let outcome = conversation(
        vm,
        mc,
        &ctx.class_name,
        selector,
        ctx.chan_link,
        &reply_rx,
        &hostop_tx,
    );
    ctx.convs.borrow_mut().remove(&me);
    end_call_and_wake(vm, &ctx, me);
    if let Err(QuoinError::PeerDied {
        reason, message, ..
    }) = &outcome
    {
        let (reason, message) = (*reason, message.clone());
        note_service_dead(vm, &ctx, reason, &message);
    }
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
    let what = format!("service call '{}'", selector.as_str());
    let wait_micros = claim_peer_object(
        vm,
        receiver,
        &ctx.claims,
        ctx.object_id,
        &ctx.class_name,
        &what,
        kind,
        "service claim",
    )?;
    Ok(Claimed { wait_micros })
}

/// The peer-generic body of [`claim_object`], shared with extension dispatch
/// (`runtime/extension.rs`): the §5.1 acquisition drive loop over any
/// registered [`PeerClaims`]. `object_label` names the object in rows and
/// cycle renderings; `what` prefixes the raised errors (e.g. `service call
/// 'sum:'`); `park_label` is the `VM.ps` park annotation. Returns the wait in
/// microseconds (0 on the uncontended path).
#[allow(clippy::too_many_arguments)]
pub(crate) fn claim_peer_object<'gc>(
    vm: &mut VmState<'gc>,
    receiver: Value<'gc>,
    claims: &Rc<RefCell<PeerClaims>>,
    object_id: u64,
    object_label: &str,
    what: &str,
    kind: WaitKind,
    park_label: &'static str,
) -> Result<u64, QuoinError> {
    let me = vm.sched.current_task;
    let epoch = vm.current_park_epoch();
    let nested = kind == WaitKind::Nested;
    let decision = claims
        .borrow_mut()
        .try_acquire(me.0, epoch, object_id, object_label, nested);
    let blocker = match decision {
        Acquire::Granted { .. } | Acquire::Reentrant => {
            return Ok(0);
        }
        Acquire::TooDeep => {
            return Err(QuoinError::Other(format!(
                "{what}: re-entered {object_label} too deeply — mutual recursion?"
            )));
        }
        Acquire::WouldQueue { blocker } => blocker,
    };
    // Rule 6: would parking close a waits-for cycle? (Only an owned object
    // can extend the walk; a reservation blocks on a task that holds nothing.)
    if let Some(start) = blocker {
        let registry = vm.io.claim_peers.clone();
        let label = format!("{object_label}#{object_id}");
        let cycle = {
            let mut live = |t: usize, e: u64| vm.channel_waiter_live(TaskId(t), e);
            would_deadlock(&registry, me.0, start, &label, &mut live)
        };
        if let Some(cycle) = cycle {
            claims.borrow_mut().stats.deadlocks += 1;
            return Err(QuoinError::Other(format!("{what}: {cycle}")));
        }
    }
    claims
        .borrow_mut()
        .enqueue(me.0, epoch, object_id, object_label, kind);
    // Park until the finishing call HANDS us the claim (fair FIFO — the old
    // ext_prelude park verbatim). The wait is boundary-profiled separately:
    // mailbox contention is its own diagnosis (§7).
    let queued_at = Instant::now();
    if let Some(t) = vm.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
        t.parked_on_channel = true;
    }
    vm.set_park_info(park_label.to_string(), Some(receiver));
    if let Some(yielder) = unsafe { vm.get_yielder() } {
        yielder.suspend(YieldReason::ChannelPark);
    } else {
        claims.borrow_mut().retract(me.0, object_id);
        return Err(QuoinError::Other(format!(
            "{what} queued outside the VM scheduler"
        )));
    }
    // On resume: if the claim was already handed to us and a cancel raced in,
    // pass it onward (mirrors channel_redeliver) — never strand the queue.
    let handed = matches!(vm.sched.wake.take(), Some(Wake::ServiceClaim { .. }));
    if vm.sched.cancel_current {
        if handed {
            end_peer_call_and_wake(vm, claims, object_id, me.0);
        } else {
            claims.borrow_mut().retract(me.0, object_id);
        }
        return Err(vm.take_cancellation());
    }
    if !handed {
        return Err(QuoinError::Other(format!(
            "{park_label} park resumed without the claim"
        )));
    }
    Ok(queued_at.elapsed().as_micros() as u64)
}

/// Release one call's claim on the proxy's object (outermost release frees
/// the lane too) and deliver the resulting handoffs.
/// The parent-side "this worker is GONE" housekeeping (SUPERVISION.md slice 0),
/// idempotent — every death-detection seam calls it. Releases the parent-held
/// block handles the worker's stored `HostBlock`s addressed (only the worker
/// could ever invoke them, and it is dead; `service_stop` releases the same
/// list on the clean path), marks the peer's claim rows dead, and records the
/// death on the lifecycle sink — the caller is about to CATCH the typed
/// death, so `serviceRestart`/`VM.peers` must already agree it happened (the
/// mailbox reader's own emission may lag on another thread; first terminal
/// wins, so the double observation collapses).
fn note_service_dead(vm: &mut VmState<'_>, ctx: &CallCtx, reason: PeerDeathReason, detail: &str) {
    for id in ctx.block_handles.borrow_mut().drain(..) {
        vm.hosted_release(id);
    }
    ctx.claims.borrow_mut().gone = Some("died");
    // Zero-gap parking (slice 3, §10.1's deliberate built-in edge): a
    // SUPERVISED service closes the restart gate at the moment of detection,
    // so the caller who catches this death and everyone after them parks
    // through the supervisor's cycle instead of failing in the gap. (Idle
    // deaths detected on the reader thread close it when the supervisor
    // wakes — a microsecond-scale gap, recorded in the doc.)
    if ctx.policy.borrow().is_some() {
        let mut g = ctx.restart.borrow_mut();
        if matches!(g.phase, GatePhase::Open) {
            g.phase = GatePhase::Restarting;
        }
    }
    if let Some(sink) = vm.io.lives.borrow().get(ctx.life_idx) {
        sink.emit_died(reason, detail);
    }
}

fn end_call_and_wake<'gc>(vm: &mut VmState<'gc>, ctx: &CallCtx, task: usize) {
    end_peer_call_and_wake(vm, &ctx.claims, ctx.object_id, task);
}

/// The peer-generic body of [`end_call_and_wake`], shared with extension
/// dispatch: release + FIFO handoff over any registered [`PeerClaims`].
pub(crate) fn end_peer_call_and_wake<'gc>(
    vm: &mut VmState<'gc>,
    claims: &Rc<RefCell<PeerClaims>>,
    object_id: u64,
    task: usize,
) {
    let grants = {
        let mut live = |t: usize, e: u64| vm.channel_waiter_live(TaskId(t), e);
        claims.borrow_mut().end_call(task, object_id, &mut live)
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
        Err(QuoinError::peer_died(
            ctx.class_name.clone(),
            PeerDeathReason::Exited,
            format!(
                "service call '{}': the service exited mid-call",
                selector.as_str()
            ),
        ))
    } else {
        conversation(
            vm,
            mc,
            &ctx.class_name,
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
    if let Err(QuoinError::PeerDied {
        reason, message, ..
    }) = &outcome
    {
        let (reason, message) = (*reason, message.clone());
        note_service_dead(vm, ctx, reason, &message);
    }
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
#[allow(clippy::too_many_arguments)] // the conversation pump threads the whole call context
fn conversation<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    class_name: &str,
    selector: Symbol,
    chan_link: usize,
    reply_rx: &async_channel::Receiver<Msg>,
    hostop_tx: &async_channel::Sender<Msg>,
) -> Result<Msg, QuoinError> {
    loop {
        let msg = match vm.await_io(IoRequest::FrameRecv(reply_rx.clone()))? {
            IoResult::FrameMsg(Some(msg)) => *msg,
            // The reply lane died under the call: the worker is GONE — the
            // typed death (SUPERVISION.md §2); callers run `note_service_dead`.
            IoResult::FrameMsg(None) => {
                return Err(QuoinError::peer_died(
                    class_name,
                    PeerDeathReason::Exited,
                    format!(
                        "service call '{}': the service exited mid-call",
                        selector.as_str()
                    ),
                ));
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
        // A previously-announced class: only the name crosses.
        Msg::CallReturnResource {
            resource,
            class_name,
        } => {
            let Some(class) = lookup_service_class(vm, ctx.chan_link, &class_name) else {
                return Err(QuoinError::Other(format!(
                    "service call '{}': the worker returned an instance of \
                     '{class_name}', which it never declared",
                    selector.as_str()
                )));
            };
            Ok(sub_proxy(vm, mc, ctx, class, resource, class_name))
        }
        // First sighting of a class: the terminal carries its manifest — the
        // parent installs a real class (ACTOR_OBJECTS.md §2), then proceeds
        // as above.
        Msg::CallReturnResourceDecl {
            resource,
            class_name,
            instance_selectors,
            class_selectors,
        } => {
            let class = installed_service_class(
                vm,
                mc,
                ctx.chan_link,
                &class_name,
                &instance_selectors,
                &class_selectors,
                receiver,
            );
            Ok(sub_proxy(vm, mc, ctx, class, resource, class_name))
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

/// Mint a sub-proxy: an instance of the service's installed class for
/// `class_name`, sharing every worker-wide handle with its siblings.
fn sub_proxy<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    ctx: &CallCtx,
    class: gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::Class<'gc>>>,
    resource: u64,
    class_name: String,
) -> Value<'gc> {
    vm.new_native_state(
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
            life_idx: ctx.life_idx,
            incarnation: ctx.incarnation.clone(),
            minted: ctx.minted,
            restart: ctx.restart.clone(),
            recipe: None,
            policy: ctx.policy.clone(),
            hook: Cell::new(None),
        },
    )
}

/// The block forms — `Worker.host:'unit.qn' with:{ Pool.new:cfg }` and the
/// unit-less `Worker.with:{ Timer.new }`: the PORTABLE block ships to the
/// worker, runs there after its unit loads, and the object it answers is
/// hosted as the root. On process backing the block crosses as source +
/// captures. The `args:` list parameterizes the block: each element crosses
/// by the spawn-arg rules ([`spawn_arg`]) as a mailbox message the worker
/// consumes before invoking the block — channels become live relay
/// endpoints, portable values snapshot, anything else refuses loudly here.
#[allow(clippy::too_many_arguments)] // the host forms thread their full spawn context
pub(crate) fn host_block<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    path: Option<String>,
    block: Value<'gc>,
    lanes: u32,
    backing: &'static str,
    args: Option<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let Value::Class(_) = receiver else {
        return Err(QuoinError::Other("Worker.host: bad receiver".into()));
    };
    let Some((template, parent_env)) = crate::runtime::worker::block_parts(block) else {
        return Err(QuoinError::Other(
            "Worker.host:with: expects a Block (the object to host is its answer)".into(),
        ));
    };
    // Arity first, before anything ships: the block's parameters are exactly
    // the args: list (absent = a parameterless block).
    let arg_values = match args {
        Some(list) => crate::runtime::extension::extract_args(list)?,
        None => Vec::new(),
    };
    if template.param_syms.len() != arg_values.len() {
        return Err(QuoinError::Other(format!(
            "Worker.host:with:args: the block takes {} parameter(s) but args: has {}",
            template.param_syms.len(),
            arg_values.len()
        )));
    }
    let pb = snapshot_block(template, parent_env, 0)
        .map_err(|e| QuoinError::Other(format!("Worker.host:with: {e}")))?;
    let label_path = path.clone().unwrap_or_else(|| "{block}".to_string());
    let process = backing == "process";
    let (ch, pid) = match backing {
        // Process backing: the block crosses as SOURCE + captures (the same
        // portability gate as thread backing — snapshot_block above — plus
        // the source-text requirement) and the child compiles it against its
        // own unit after the version gate.
        "process" => {
            let payload = crate::worker::portable_block_to_wire(&pb)
                .map_err(|e| QuoinError::Other(format!("Worker.host:with: {e}")))?;
            let (ch, pid, _grip) = crate::worker::spawn_worker_process(
                path.clone(),
                crate::worker::ProcessBody::Block(payload),
                lanes,
            )
            .map_err(QuoinError::Other)?;
            (ch, Some(pid))
        }
        _ => (
            crate::worker::spawn_worker_hosted_block(path.clone(), pb.clone(), lanes),
            None,
        ),
    };
    // The chan link registers BEFORE the args ship (a channel arg mints its
    // relay bookkeeping against it), and the args ship BEFORE the ready wait
    // (the worker consumes them before running the block — sending after
    // would deadlock the handshake).
    let chan_link = crate::runtime::channel_relay::register_chan_link(
        vm,
        ch.chan_tx.clone(),
        ch.chan_rx.clone(),
    );
    let mut recipe_args = Vec::with_capacity(arg_values.len());
    let mut chan_positions: Vec<usize> = Vec::new();
    for (i, v) in arg_values.iter().enumerate() {
        let msg = spawn_arg(vm, mc, *v, chan_link, process).map_err(|e| {
            QuoinError::Other(format!("Worker.host:with:args: element {}: {e}", i + 1))
        })?;
        // Retain the recipe form (SUPERVISION.md slice 2): data/blocks are the
        // message itself, re-sent verbatim at restart; a channel is retained
        // as a VALUE (below) and re-shipped against the new link.
        recipe_args.push(match &msg {
            crate::worker::WorkerMsg::Channel(_) => {
                chan_positions.push(i);
                RecipeArg::Channel(chan_positions.len() - 1)
            }
            other => RecipeArg::Plain(other.clone()),
        });
        let _ = ch.inbox_tx.try_send(msg);
    }
    let seed = RecipeSeed {
        path,
        pb,
        args: recipe_args,
        chan_values: chan_positions.iter().map(|&i| arg_values[i]).collect(),
    };
    finish_host(
        vm,
        mc,
        ch,
        pid,
        backing,
        lanes,
        &label_path,
        chan_link,
        seed,
    )
}

/// What `host_block` hands `finish_host` toward the respawn recipe: the parts
/// known before the ready manifest (which completes it). `chan_values` are
/// the channel args as VALUES — rooted into `vm.recipe_chans` once the host
/// succeeds.
struct RecipeSeed<'gc> {
    path: Option<String>,
    pb: PortableBlock,
    args: Vec<RecipeArg>,
    chan_values: Vec<Value<'gc>>,
}

/// Classify one spawn-time `args:` element into its crossing form: a channel
/// ships as a relay endpoint id (against the just-registered link), a
/// portable block snapshots (pre-validating the source-text requirement for
/// process backing, so the mailbox pump's encode cannot fail later), and
/// anything else must be portable data. Errors name the reason; the caller
/// prefixes the element index.
pub(crate) fn spawn_arg<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    v: Value<'gc>,
    chan_link: usize,
    process: bool,
) -> Result<crate::worker::WorkerMsg, QuoinError> {
    if crate::runtime::channel_relay::is_channel_value(v) {
        let chan = crate::runtime::channel_relay::ship_for_crossing(vm, mc, v, chan_link)?;
        return Ok(crate::worker::WorkerMsg::Channel(chan));
    }
    if let Some((template, parent_env)) = crate::runtime::worker::block_parts(v) {
        let pb = snapshot_block(template, parent_env, 0)?;
        if process {
            crate::worker::portable_block_to_wire(&pb).map_err(QuoinError::Other)?;
        }
        return Ok(crate::worker::WorkerMsg::Block(pb));
    }
    let dv = crate::runtime::extension::value_to_wire(v, None)?;
    Ok(crate::worker::WorkerMsg::Data(dv))
}

/// The shared back half of hosting: registry rows, the ready/manifest
/// handshake, claim + boundary + relay registration, and the installed-class
/// two-step that mints the root proxy. The hosted class is whatever the
/// ready message names — the block's answer.
/// Park for the worker's ready message and answer the hosted class's manifest
/// (name + sorted selector lists). A closed lane means boot/compile/
/// instantiation failed before ready — flattened to an ordinary error (a peer
/// that never lived didn't die; SUPERVISION.md §2). Shared by the original
/// host and `serviceRestart`, whose rule-9 gate compares this against the
/// installed class.
fn await_ready_manifest<'gc>(
    vm: &mut VmState<'gc>,
    ch: &crate::worker::WorkerChannels,
    what: &str,
) -> Result<(String, Vec<String>, Vec<String>), QuoinError> {
    let (ready_class, instance_selectors, class_selectors) =
        match vm.await_io(IoRequest::WorkerRecv(ch.outbox_rx.clone()))? {
            IoResult::WorkerMsg(Some(msg)) => parse_ready_manifest(&msg),
            IoResult::WorkerMsg(None) => {
                let why = match vm.await_io(IoRequest::WorkerJoin(ch.done_rx.clone()))? {
                    IoResult::WorkerDone(Err(WorkerExit::Failed(msg))) => msg,
                    IoResult::WorkerDone(Err(WorkerExit::Died { detail, .. })) => detail,
                    _ => "the worker exited before reporting ready".to_string(),
                };
                return Err(QuoinError::Other(format!("{what}: {why}")));
            }
            other => {
                return Err(QuoinError::Other(format!(
                    "{what}: unexpected result {other:?}"
                )));
            }
        };
    match ready_class {
        Some(n) => Ok((n, instance_selectors, class_selectors)),
        None => Err(QuoinError::Other(format!(
            "{what}: the worker did not report a hosted class"
        ))),
    }
}

#[allow(clippy::too_many_arguments)] // hosting threads its full spawn context
fn finish_host<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    ch: crate::worker::WorkerChannels,
    pid: Option<u32>,
    backing: &'static str,
    lanes: u32,
    path: &str,
    chan_link: usize,
    seed: RecipeSeed<'gc>,
) -> Result<Value<'gc>, QuoinError> {
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
    // Handshake: the serve loop's first act is a 'ready' message carrying the
    // hosted class's MANIFEST (name + selector lists — ACTOR_OBJECTS.md §2); a
    // closed lane instead means boot/compile/instantiation failed — the done
    // lane says why. Parks, so slow boots don't block other tasks.
    let (class_name, instance_selectors, class_selectors) =
        await_ready_manifest(vm, &ch, "Worker.host")?;
    // Claims: the §5.1 machinery, one per worker, registered for VM.claims
    // and the cross-peer cycle walk; boundary rows registered beside the
    // extensions' (§7 — one diagnosis surface).
    let claims = Rc::new(RefCell::new(PeerClaims::new(label.clone(), lanes)));
    vm.io.claim_peers.borrow_mut().push(claims.clone());
    let boundary = Rc::new(RefCell::new(BoundaryStats {
        peer: label.clone(),
        rows: HashMap::new(),
    }));
    vm.io.ext_stats.borrow_mut().push(boundary.clone());
    // Lifecycle roster row (SUPERVISION.md slice 1): the sink was created at
    // spawn (its producers live in the spawn machinery); the parent registers
    // it here, beside the claims and boundary rows.
    let life_idx = {
        let lives = vm.io.lives.clone();
        let mut lives = lives.borrow_mut();
        lives.push(ch.life.clone());
        lives.len() - 1
    };
    // Complete + freeze the respawn recipe (SUPERVISION.md slice 2): the
    // manifest fields double as the rule-9 equality gate; the channel-arg
    // values get their GC root — a `vm.pins` ticket each — now that the host
    // is definitely succeeding.
    let chan_owner = crate::pin_table::PinOwner {
        kind: "service-recipe",
        id: chan_link as u64,
    };
    let chan_pins = seed
        .chan_values
        .iter()
        .map(|v| vm.pins.pin(chan_owner, *v))
        .collect();
    let recipe = Rc::new(ServiceRecipe {
        label: label.clone(),
        path: seed.path,
        pb: seed.pb,
        lanes,
        backing,
        args: seed.args,
        chan_pins,
        class_name: class_name.clone(),
        instance_selectors: instance_selectors.clone(),
        class_selectors: class_selectors.clone(),
    });
    // Install the hosted class from its manifest (the two-step: the shell
    // exists first so the root proxy can be its instance, then the method
    // nodes — which carry the proxy — fill it in).
    let shell = vm.make_service_class_shell(mc, &class_name);
    let proxy = vm.new_native_state(
        mc,
        shell,
        NativeServiceState {
            dispatch_tx: ch.dispatch_tx,
            done_rx: ch.done_rx,
            claims,
            lanes: lanes.max(1),
            stopped: Rc::new(Cell::new(false)),
            reap: Rc::new(RefCell::new(Vec::new())),
            object_id: 1,
            class_name: class_name.clone(),
            process: backing == "process",
            convs: Rc::new(RefCell::new(HashMap::new())),
            block_handles: Rc::new(RefCell::new(Vec::new())),
            boundary,
            chan_link,
            life_idx,
            incarnation: Rc::new(Cell::new(1)),
            minted: 1,
            restart: Rc::new(RefCell::new(RestartGate::default())),
            recipe: Some(recipe),
            policy: Rc::new(RefCell::new(None)),
            hook: Cell::new(None),
        },
    );
    vm.service_classes.push(crate::vm::ServiceClassEntry {
        link: chan_link,
        name: class_name,
        class: Value::Class(shell),
    });
    vm.populate_service_class(
        mc,
        shell,
        proxy,
        &instance_selectors,
        &class_selectors,
        &[
            ("serviceStop", service_stop),
            ("serviceEvents", service_events),
            ("serviceRestart", service_restart),
            ("serviceSupervise:", service_supervise),
            ("serviceOnRestart:", service_on_restart),
            ("==:", service_eq),
        ],
    );
    Ok(proxy)
}

/// Pull the hosted class name and selector lists out of the ready message
/// (absent pieces — a plain `ready: true` — decode as empty/None).
fn parse_ready_manifest(
    msg: &crate::worker::WorkerMsg,
) -> (Option<String>, Vec<String>, Vec<String>) {
    let crate::worker::WorkerMsg::Data(WireData::Map(entries)) = msg else {
        return (None, Vec::new(), Vec::new());
    };
    let strings = |key: &str| -> Vec<String> {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| match v {
                WireData::List(items) => items
                    .iter()
                    .filter_map(|i| match i {
                        WireData::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default()
    };
    let name = entries
        .iter()
        .find(|(k, _)| k == "className")
        .and_then(|(_, v)| match v {
            WireData::Str(s) => Some(s.clone()),
            _ => None,
        });
    (name, strings("instance"), strings("classSide"))
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

pub(crate) fn backing_arg<'gc>(v: Value<'gc>) -> Result<&'static str, QuoinError> {
    match string_arg(v, "the backing")?.as_str() {
        "thread" => Ok("thread"),
        "process" => Ok("process"),
        other => Err(QuoinError::Other(format!(
            "Worker.host: unknown backing '{other}' (thread|process)"
        ))),
    }
}

pub(crate) fn lanes_arg<'gc>(v: Value<'gc>) -> Result<u32, QuoinError> {
    match v.as_i64() {
        Some(n) if (1..=1024).contains(&n) => Ok(n as u32),
        _ => Err(QuoinError::Other(
            "Worker.host: lanes must be an Integer between 1 and 1024".into(),
        )),
    }
}

pub(crate) fn string_arg<'gc>(v: Value<'gc>, what: &str) -> Result<String, QuoinError> {
    match v {
        Value::Object(obj) => match &obj.borrow().payload {
            crate::value::ObjectPayload::String(s) => Ok(s.to_string()),
            _ => Err(QuoinError::Other(format!(
                "Worker.host: {what} must be a String"
            ))),
        },
        _ => Err(QuoinError::Other(format!(
            "Worker.host: {what} must be a String"
        ))),
    }
}

/// The slice-2 pre-dispatch checks (SUPERVISION.md §4): raise the typed
/// staleness for a proxy of a died incarnation, and park top-level sends
/// through an in-flight restart attempt (rule 5) — woken into the new
/// incarnation on success, into the typed death on failure. Nested calls
/// (an open conversation) skip the gate: their conversation belongs to the
/// dead incarnation and fails fast on its own lanes.
fn await_restart_window<'gc>(
    vm: &mut VmState<'gc>,
    anchor: Value<'gc>,
    selector: Symbol,
) -> Result<(), QuoinError> {
    loop {
        let (minted, incarnation, restart, convs, class_name) = anchor
            .with_native_state::<NativeServiceState, _, _>(|s| {
                (
                    s.minted,
                    s.incarnation.clone(),
                    s.restart.clone(),
                    s.convs.clone(),
                    s.class_name.clone(),
                )
            })
            .map_err(QuoinError::Other)?;
        let cur = incarnation.get();
        if minted != cur {
            return Err(QuoinError::peer_died(
                class_name.clone(),
                PeerDeathReason::StaleIncarnation,
                format!(
                    "service call '{}': this proxy belongs to incarnation {minted} of \
                     `{class_name}`, which died; the service is now incarnation {cur} — \
                     hold the root proxy and re-fetch what you need from it",
                    selector.as_str()
                ),
            ));
        }
        let me = vm.sched.current_task;
        if convs.borrow().contains_key(&me.0) {
            return Ok(());
        }
        let epoch = vm.current_park_epoch();
        {
            let mut g = restart.borrow_mut();
            match &g.phase {
                GatePhase::Open => return Ok(()),
                GatePhase::Restarting => {
                    if g.exempt == Some(me.0) {
                        return Ok(());
                    }
                    g.waiters.push((me.0, epoch))
                }
                // The permanent terminal (slice 3): the policy's budget is
                // spent — every sender gets the typed #gaveUp, forever.
                GatePhase::GaveUp { attempts, last } => {
                    return Err(QuoinError::peer_died(
                        class_name.clone(),
                        PeerDeathReason::GaveUp,
                        format!(
                            "service call '{}': `{class_name}` gave up after {attempts} \
                             death(s) — its supervision budget is spent (last: {last})",
                            selector.as_str()
                        ),
                    ));
                }
            }
        }
        if let Some(t) = vm.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = true;
        }
        vm.set_park_info("service restart wait".to_string(), Some(anchor));
        if let Some(yielder) = unsafe { vm.get_yielder() } {
            yielder.suspend(YieldReason::ChannelPark);
        } else {
            restart.borrow_mut().waiters.retain(|&(t, _)| t != me.0);
            return Err(QuoinError::Other(format!(
                "service call '{}' parked on a restart outside the VM scheduler",
                selector.as_str()
            )));
        }
        let woken = matches!(vm.sched.wake.take(), Some(Wake::ServiceClaim { .. }));
        if vm.sched.cancel_current {
            restart.borrow_mut().waiters.retain(|&(t, _)| t != me.0);
            return Err(vm.take_cancellation());
        }
        if !woken {
            return Err(QuoinError::Other(
                "service restart wait resumed without a wake".to_string(),
            ));
        }
        // Loop: re-read the gate (another restart could already be running)
        // and the incarnation (this proxy may have gone stale meanwhile).
    }
}

/// The proxy-owned `serviceRestart` (SUPERVISION.md slice 2 — the manual
/// trigger, and permanently the library extension point, §10.1): re-run the
/// root's frozen recipe in a fresh isolate and REBIND the root proxy in
/// place. Only follows a DEATH (§2: stop means stop); only the root holds a
/// recipe. On success the incarnation bumps — sub-proxies, handles, and
/// endpoints minted by the dead incarnation raise `#staleIncarnation`
/// forever; parked restart-window senders wake into the new incarnation.
/// On failure they wake into the typed death, the service stays dead, and
/// another restart may be attempted.
pub(crate) fn service_restart<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    _args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let (recipe, incarnation, restart, life_idx, class_name, minted) = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| {
            (
                s.recipe.clone(),
                s.incarnation.clone(),
                s.restart.clone(),
                s.life_idx,
                s.class_name.clone(),
                s.minted,
            )
        })
        .map_err(QuoinError::Other)?;
    let Some(recipe) = recipe else {
        return Err(QuoinError::Other(
            "serviceRestart: only the root proxy holds the respawn recipe \
             (sub-proxies are incarnation state and die with it)"
                .into(),
        ));
    };
    let cur = incarnation.get();
    if minted != cur {
        return Err(QuoinError::peer_died(
            class_name.clone(),
            PeerDeathReason::StaleIncarnation,
            format!("serviceRestart: this root belongs to dead incarnation {minted}"),
        ));
    }
    // A supervised service's restarts belong to its policy (slice 3): the
    // supervisor owns the gate and the budget; a racing manual restart would
    // corrupt both.
    if receiver
        .with_native_state::<NativeServiceState, _, _>(|s| s.policy.borrow().is_some())
        .unwrap_or(false)
    {
        return Err(QuoinError::Other(format!(
            "serviceRestart: `{class_name}` is supervised — the policy owns its restarts"
        )));
    }
    // Rule 1: only death restarts. A stop was an instruction (§2).
    let status = vm.io.lives.borrow().get(life_idx).map(|l| l.status());
    match status {
        Some(crate::runtime::lifecycle::LifeStatus::Died { .. }) => {}
        Some(crate::runtime::lifecycle::LifeStatus::Stopped(_)) => {
            return Err(QuoinError::Other(format!(
                "serviceRestart: `{class_name}` was STOPPED, not died — stop means stop \
                 (SUPERVISION.md §2); host it again instead"
            )));
        }
        _ => {
            return Err(QuoinError::Other(format!(
                "serviceRestart: `{class_name}` is running — restart only follows a death"
            )));
        }
    }
    {
        let mut g = restart.borrow_mut();
        match g.phase {
            GatePhase::Open => g.phase = GatePhase::Restarting,
            GatePhase::Restarting => {
                return Err(QuoinError::Other(
                    "serviceRestart: a restart is already in flight".into(),
                ));
            }
            GatePhase::GaveUp { .. } => {
                return Err(QuoinError::Other(format!(
                    "serviceRestart: `{class_name}` gave up — permanently dead this process"
                )));
            }
        }
    }
    let outcome = restart_attempt(vm, mc, receiver, &recipe, cur);
    // Release the gate and wake every parked sender EITHER WAY: into the new
    // incarnation, or into the typed death (the rule-5 wake). A failed MANUAL
    // restart re-opens the gate — dead but retryable.
    let waiters = {
        let mut g = restart.borrow_mut();
        g.phase = GatePhase::Open;
        std::mem::take(&mut g.waiters)
    };
    for (task, epoch) in waiters {
        if vm.channel_waiter_live(TaskId(task), epoch) {
            vm.wake_channel_task(TaskId(task), Wake::ServiceClaim { lane: None });
        }
    }
    outcome.map(|_| vm.new_nil(mc))
}

/// The respawn body: spawn from the frozen recipe, re-ship the args (channels
/// against the NEW link), gate the ready manifest against the installed class
/// (rule 9), mint the per-incarnation rows, and rebind the root state.
fn restart_attempt<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    recipe: &Rc<ServiceRecipe>,
    cur: u64,
) -> Result<(), QuoinError> {
    let next = cur + 1;
    let process = recipe.backing == "process";
    let (ch, pid) = match recipe.backing {
        "process" => {
            let payload = crate::worker::portable_block_to_wire(&recipe.pb)
                .map_err(|e| QuoinError::Other(format!("serviceRestart: {e}")))?;
            let (ch, pid, _grip) = crate::worker::spawn_worker_process(
                recipe.path.clone(),
                crate::worker::ProcessBody::Block(payload),
                recipe.lanes,
            )
            .map_err(QuoinError::Other)?;
            (ch, Some(pid))
        }
        _ => (
            crate::worker::spawn_worker_hosted_block(
                recipe.path.clone(),
                recipe.pb.clone(),
                recipe.lanes,
            ),
            None,
        ),
    };
    let chan_link = crate::runtime::channel_relay::register_chan_link(
        vm,
        ch.chan_tx.clone(),
        ch.chan_rx.clone(),
    );
    for (i, arg) in recipe.args.iter().enumerate() {
        let msg = match arg {
            RecipeArg::Plain(m) => m.clone(),
            RecipeArg::Channel(idx) => {
                let v = recipe
                    .chan_pins
                    .get(*idx)
                    .and_then(|pin| vm.pins.get(*pin))
                    .ok_or_else(|| {
                        QuoinError::Other(
                            "serviceRestart: a channel argument's recipe root vanished".into(),
                        )
                    })?;
                spawn_arg(vm, mc, v, chan_link, process).map_err(|e| {
                    QuoinError::Other(format!("serviceRestart: args element {}: {e}", i + 1))
                })?
            }
        };
        let _ = ch.inbox_tx.try_send(msg);
    }
    // Rule 9: the new incarnation must be the SAME class, selector for
    // selector — a differing manifest means the recipe isn't deterministic
    // (or the code changed underfoot); refuse to rebind. Dropping `ch`
    // orphans the new worker, whose lanes closing shut it down.
    let (name, inst, cls) = await_ready_manifest(vm, &ch, "serviceRestart")?;
    if name != recipe.class_name
        || inst != recipe.instance_selectors
        || cls != recipe.class_selectors
    {
        return Err(QuoinError::Other(format!(
            "serviceRestart: the new incarnation's manifest does not match the installed \
             class `{}` (it reported `{name}`) — the recipe is not deterministic, or the \
             code changed underfoot; refusing to rebind (SUPERVISION.md §4 rule 9)",
            recipe.class_name
        )));
    }
    // Per-incarnation rows: fresh claims (rule 8 — empty mailboxes, fresh
    // lane pool), a registry row, and the fresh lifecycle sink the spawn
    // minted (its `spawned` event is already staged). Boundary rows stay
    // shared: one merged cost table per service across incarnations.
    let label = format!("{} (incarnation {next})", recipe.label);
    let claims = Rc::new(RefCell::new(PeerClaims::new(label.clone(), recipe.lanes)));
    vm.io.claim_peers.borrow_mut().push(claims.clone());
    vm.worker_registry.push(crate::worker::WorkerReg {
        unit: label.clone(),
        label,
        backing: recipe.backing,
        pid,
        inbox_tx: ch.inbox_tx.clone(),
        outbox_rx: ch.outbox_rx.clone(),
        control_tx: ch.control_tx.clone(),
    });
    ch.life
        .incarnation
        .store(next, std::sync::atomic::Ordering::Relaxed);
    let life_idx = {
        let lives = vm.io.lives.clone();
        let mut lives = lives.borrow_mut();
        lives.push(ch.life.clone());
        lives.len() - 1
    };
    // Rule 3: rebind the ROOT in place and bump the incarnation — everything
    // the dead incarnation minted goes #staleIncarnation from here.
    receiver
        .with_native_state_mut::<NativeServiceState, _, _>(mc, |s| {
            s.dispatch_tx = ch.dispatch_tx;
            s.done_rx = ch.done_rx;
            s.claims = claims;
            s.stopped = Rc::new(Cell::new(false));
            s.reap = Rc::new(RefCell::new(Vec::new()));
            s.convs = Rc::new(RefCell::new(HashMap::new()));
            s.block_handles = Rc::new(RefCell::new(Vec::new()));
            s.chan_link = chan_link;
            s.life_idx = life_idx;
            s.minted = next;
            s.incarnation.set(next);
        })
        .map_err(QuoinError::Other)?;
    run_restart_hook(vm, mc, receiver, next)
}

/// The user's restart hook (`serviceOnRestart:`), run as the restart
/// attempt's TAIL: after the rebind — the transport is live, so the hook can
/// call the service — but before the gate reopens, so parked senders resume
/// only against a hooked-up incarnation. The hook's own sends pass the closed
/// gate through the exemption. A hook failure fails the ATTEMPT: the fresh
/// worker is stopped cleanly (it must not serve half-set-up), the supervisor
/// counts it against the budget, a manual restart relays it typed.
fn run_restart_hook<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    incarnation: u64,
) -> Result<(), QuoinError> {
    let (hook, restart, class_name) = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| {
            (s.hook.get(), s.restart.clone(), s.class_name.clone())
        })
        .map_err(QuoinError::Other)?;
    let Some(block) = hook.and_then(|pin| vm.pins.get(pin)) else {
        return Ok(());
    };
    restart.borrow_mut().exempt = Some(vm.sched.current_task.0);
    let outcome = vm.call_method(mc, block, "value:", vec![receiver]);
    restart.borrow_mut().exempt = None;
    if let Err(e) = outcome {
        // This native swallows the hook's throw, so it must also clear the
        // in-flight exception VALUE — a stale `active` would be handed to the
        // next `catch:` anywhere in the VM in place of its real error.
        vm.exceptions.active = None;
        let _ = service_stop(vm, mc, receiver, Vec::new());
        vm.exceptions.active = None;
        return Err(QuoinError::Other(format!(
            "`{class_name}` restart hook (incarnation {incarnation}): {e}"
        )));
    }
    Ok(())
}

/// The proxy-owned `serviceOnRestart:` — install (a Block taking the root
/// proxy), replace, or clear (nil) the restart hook. Root-only, like the
/// recipe it complements; the block pins into `vm.pins` and survives
/// incarnations.
pub(crate) fn service_on_restart<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let arg = *args.first().ok_or_else(|| {
        QuoinError::Other("serviceOnRestart: expects a Block (or nil to clear)".into())
    })?;
    let (has_recipe, old, life_idx) = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| {
            (s.recipe.is_some(), s.hook.get(), s.life_idx)
        })
        .map_err(QuoinError::Other)?;
    if !has_recipe {
        return Err(QuoinError::Other(
            "serviceOnRestart: only the root proxy holds the respawn recipe".into(),
        ));
    }
    let new = if matches!(arg, Value::Nil) {
        None
    } else if crate::runtime::worker::block_parts(arg).is_some() {
        Some(vm.pins.pin(
            crate::pin_table::PinOwner {
                kind: "restart-hook",
                id: life_idx as u64,
            },
            arg,
        ))
    } else {
        return Err(QuoinError::Other(
            "serviceOnRestart: expects a Block (or nil to clear)".into(),
        ));
    };
    if let Some(pin) = old {
        vm.pins.unpin(pin);
    }
    receiver
        .with_native_state::<NativeServiceState, _, _>(|s| s.hook.set(new))
        .map_err(QuoinError::Other)?;
    Ok(vm.new_nil(mc))
}

/// The proxy-owned `serviceSupervise:` (SUPERVISION.md slice 3): attach a
/// `Supervise` policy post-spawn — the runtime interprets the data directly,
/// and a per-service supervisor task (the `SuperviseBoot` pattern) owns the
/// restart cycle from then on. Root only (the recipe holder); attach once
/// (v1 has no detach); `Supervise.never` is a no-op when nothing is attached.
pub(crate) fn service_supervise<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let arg = *args
        .first()
        .ok_or_else(|| QuoinError::Other("serviceSupervise: expects a Supervise policy".into()))?;
    let (has_recipe, policy_cell, class_name) = receiver
        .with_native_state::<NativeServiceState, _, _>(|st| {
            (
                st.recipe.is_some(),
                st.policy.clone(),
                st.class_name.clone(),
            )
        })
        .map_err(QuoinError::Other)?;
    if !has_recipe {
        return Err(QuoinError::Other(
            "serviceSupervise: only the root proxy holds the respawn recipe".into(),
        ));
    }
    let parsed = crate::runtime::supervise::parse_policy(vm, mc, arg, "serviceSupervise:")?;
    let Some(policy) = parsed else {
        // #never: fail-fast is the default; refusing a detach keeps the
        // supervisor task's lifecycle simple in v1.
        if policy_cell.borrow().is_some() {
            return Err(QuoinError::Other(format!(
                "serviceSupervise: `{class_name}` is already supervised — detaching is \
                 not supported (v1)"
            )));
        }
        return Ok(vm.new_nil(mc));
    };
    {
        let mut cell = policy_cell.borrow_mut();
        if cell.is_some() {
            return Err(QuoinError::Other(format!(
                "serviceSupervise: `{class_name}` is already supervised"
            )));
        }
        *cell = Some(policy);
    }
    // The supervisor task (native loop through a qnlib boot block — a task
    // always runs a Quoin block). Capturing the proxy roots it: a supervised
    // service is program-lifetime by nature. An already-dead peer is fine:
    // the loop's first watch() delivers the terminal immediately.
    let boot = crate::runtime::extension::resolve_global(vm, "SuperviseBoot").ok_or_else(|| {
        QuoinError::Other("serviceSupervise: SuperviseBoot is not installed (qnlib)".into())
    })?;
    vm.call_method(mc, boot, "service:", vec![receiver])?;
    Ok(vm.new_nil(mc))
}

/// The per-service supervisor (SUPERVISION.md §4 rule 7), running on its own
/// task: park on the sink's terminal watch; on a death, run the restart cycle
/// — exponential backoff between attempts, every death (including failed
/// attempts, §2) counted against the policy's window — and either re-open the
/// gate into the new incarnation or GIVE UP: the gate goes permanently
/// `GaveUp`, parked and future senders get the typed `#gaveUp`, and the last
/// incarnation's roster row says so. A clean stop ends the supervisor (§2:
/// stop means stop). The window bookkeeping reads wall time — a documented
/// replay divergence point (§7): intensity is inherently wall-clock.
pub(crate) fn supervise_service_loop<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    svc: Value<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    let mut deaths: Vec<Instant> = Vec::new();
    loop {
        let (life_idx, policy, gate, recipe) = svc
            .with_native_state::<NativeServiceState, _, _>(|st| {
                (
                    st.life_idx,
                    *st.policy.borrow(),
                    st.restart.clone(),
                    st.recipe.clone(),
                )
            })
            .map_err(QuoinError::Other)?;
        let (Some(policy), Some(recipe)) = (policy, recipe) else {
            return Ok(vm.new_nil(mc));
        };
        let Some(sink) = vm.io.lives.borrow().get(life_idx).cloned() else {
            return Ok(vm.new_nil(mc));
        };
        let watch_rx = sink.watch();
        match vm.await_io(IoRequest::WorkerRecv(watch_rx))? {
            IoResult::WorkerMsg(_) => {}
            other => {
                return Err(QuoinError::Other(format!(
                    "supervisor: unexpected result {other:?}"
                )));
            }
        }
        match sink.status() {
            // Clean stop, or a spurious wake on a live peer: not the
            // supervisor's business (§2).
            crate::runtime::lifecycle::LifeStatus::Stopped(_) => return Ok(vm.new_nil(mc)),
            crate::runtime::lifecycle::LifeStatus::Running => continue,
            crate::runtime::lifecycle::LifeStatus::Died { .. } => {}
        }
        // Idle deaths (reader-thread detected) close the gate here; parent-
        // detected ones already closed it in `note_service_dead`.
        {
            let mut g = gate.borrow_mut();
            if matches!(g.phase, GatePhase::Open) {
                g.phase = GatePhase::Restarting;
            }
        }
        deaths.push(Instant::now());
        let mut attempt: u32 = 0;
        let window = std::time::Duration::from_millis(policy.window_ms);
        let outcome: Result<(), String> = loop {
            deaths.retain(|t| t.elapsed() <= window);
            if deaths.len() as u32 > policy.max_restarts {
                break Err(format!(
                    "{} death(s) inside {}ms exceeded the budget of {}",
                    deaths.len(),
                    policy.window_ms,
                    policy.max_restarts
                ));
            }
            attempt += 1;
            let delay = policy.delay_ms(attempt);
            if delay > 0 {
                vm.await_io(IoRequest::Sleep { ms: delay })?;
            }
            let cur = svc
                .with_native_state::<NativeServiceState, _, _>(|st| st.incarnation.get())
                .map_err(QuoinError::Other)?;
            match restart_attempt(vm, mc, svc, &recipe, cur) {
                Ok(()) => break Ok(()),
                // A failed attempt is a death of the new incarnation (§2):
                // it feeds the window and the backoff keeps doubling.
                Err(_) => deaths.push(Instant::now()),
            }
        };
        let gave_up = outcome.is_err();
        let waiters = {
            let mut g = gate.borrow_mut();
            g.phase = match outcome {
                Ok(()) => GatePhase::Open,
                Err(last) => GatePhase::GaveUp {
                    attempts: attempt,
                    last,
                },
            };
            std::mem::take(&mut g.waiters)
        };
        for (task, epoch) in waiters {
            if vm.channel_waiter_live(TaskId(task), epoch) {
                vm.wake_channel_task(TaskId(task), Wake::ServiceClaim { lane: None });
            }
        }
        if gave_up {
            sink.gave_up
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(vm.new_nil(mc));
        }
        // Success: loop — the next iteration re-reads the rebound state and
        // watches the NEW incarnation's sink.
    }
}

/// The proxy-owned `serviceEvents` (installed on every service class beside
/// `serviceStop` — a hosted class's own manifest may legitimately contain
/// `events`, whence the prefix): the worker's lifecycle events channel
/// (SUPERVISION.md slice 1). Worker-wide, so every proxy of one worker
/// answers the same channel.
pub(crate) fn service_events<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    _args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let life_idx = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| s.life_idx)
        .map_err(QuoinError::Other)?;
    crate::runtime::worker::life_events_channel(vm, mc, life_idx)
}

/// The proxy-owned `serviceStop` (installed on every service class): flag +
/// drain (refuse new calls, wait for every in-flight conversation), then one
/// stop op per lane, then join. Worker-wide — every proxy of the service
/// refuses calls afterwards.
pub(crate) fn service_stop<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    _args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let ctx = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| {
            s.stopped.set(true);
            snapshot(s)
        })
        .map_err(QuoinError::Other)?;
    drain_lanes(vm, receiver, &ctx)?;
    // One reserved stop op per lane fiber; a dead worker skips straight to
    // the join, which reports why.
    for _ in 0..ctx.lanes {
        let (reply_tx, reply_rx) = async_channel::bounded::<Msg>(1);
        let (_hostop_tx, hostop_rx) = async_channel::bounded::<Msg>(1);
        let frame = hosted_call(&ctx, OP_STOP.to_string(), Vec::new(), Vec::new());
        if ctx
            .dispatch_tx
            .try_send(DispatchReq {
                frame,
                blocks: Vec::new(),
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
    // The worker is gone: release the block handles its stored HostBlocks
    // addressed (minted for block arguments that crossed as handles).
    for id in ctx.block_handles.borrow_mut().drain(..) {
        vm.hosted_release(id);
    }
    match joined {
        IoResult::WorkerDone(Ok(_)) => {
            ctx.claims.borrow_mut().gone = Some("stopped");
            Ok(vm.new_nil(mc))
        }
        IoResult::WorkerDone(Err(WorkerExit::Failed(msg))) => {
            ctx.claims.borrow_mut().gone = Some("stopped");
            Err(QuoinError::Other(msg))
        }
        // The stop found a corpse (or the worker died during the drain).
        IoResult::WorkerDone(Err(WorkerExit::Died { reason, detail })) => {
            ctx.claims.borrow_mut().gone = Some("died");
            Err(QuoinError::peer_died(
                ctx.class_name.clone(),
                reason,
                detail,
            ))
        }
        other => Err(QuoinError::Other(format!(
            "serviceStop: unexpected result {other:?}"
        ))),
    }
}
