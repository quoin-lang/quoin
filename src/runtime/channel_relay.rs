//! Cross-isolate channel endpoints (docs/internal/ACTOR_OBJECTS.md §6): the
//! REMOTE side of a shipped channel. The channel itself never moves — the
//! owning isolate keeps the one true `NativeChannelState` (its waiter queues
//! now admit remote entries; see channel.rs) — and every other isolate holds a
//! relay endpoint that quacks like `Channel`: `send:` / `receive` / `close` /
//! `each:` forward as correlation-id frames over the worker link's relay lane
//! and park with the ordinary channel machinery.
//!
//! Each side of a link runs one relay-agent task (`Channel.relayAgent:`,
//! spawned lazily via the qnlib `ChannelRelayBoot` helper the first time a
//! channel crosses): it drains the link's inbound lane, applying owner-side
//! frames to rooted channels (`channel_apply_owner_frame`) and resolving
//! endpoint-side answers against the link's pending-op table — the §6 shape:
//! a frame where a wake used to be, `wake_channel_task` as the single local
//! choke point.
//!
//! Semantic edges preserved across the boundary: backpressure is a delayed
//! `Ack` (a full buffer parks remote senders); `close` propagates both ways;
//! a value delivered to a since-cancelled receiver redelivers locally or goes
//! home in a `Return` frame — never silently dropped. Cancellation retracts
//! pending ops with `Cancel` so no ghost edge outlives its task.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use gc_arena::Collect;
use gc_arena::collect::Trace;

use crate::error::QuoinError;
use crate::fiber::YieldReason;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::extension::{value_to_wire, wire_to_value};
use crate::value::{AnyCollect, Value};
use crate::vm::VmState;
use crate::vm_scheduler::{TaskId, Wake};
use crate::worker::{ChanFrame, PendingChanOp};

/// Remote-side endpoint state: which link the channel crossed on, its id in
/// the OWNER's hosted table, and the link's reap for drop-time release.
#[derive(Debug)]
pub struct NativeRelayChannel {
    pub link: usize,
    pub chan: u64,
    reap: Rc<RefCell<Vec<u64>>>,
}

impl Drop for NativeRelayChannel {
    fn drop(&mut self) {
        self.reap.borrow_mut().push(self.chan);
    }
}

impl AnyCollect for NativeRelayChannel {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

unsafe impl<'gc> Collect<'gc> for NativeRelayChannel {
    const NEEDS_TRACE: bool = false;
}

/// Register one worker link's relay lanes in `vm.io.chan_links` and answer
/// the link index endpoints and ships will address.
pub(crate) fn register_chan_link(
    vm: &mut VmState<'_>,
    out: async_channel::Sender<ChanFrame>,
    inbound: async_channel::Receiver<ChanFrame>,
) -> usize {
    vm.io.chan_links.push(crate::worker::ChanLink {
        out,
        inbound,
        agent_running: false,
        next_corr: 1,
        pending: std::collections::HashMap::new(),
        reap: Rc::new(RefCell::new(Vec::new())),
    });
    vm.io.chan_links.len() - 1
}

/// Ship a LOCAL channel argument/return across `link`, refusing what §6
/// refuses: re-shipping a relay endpoint (route through the owner).
pub(crate) fn ship_for_crossing<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    channel: Value<'gc>,
    link: usize,
) -> Result<u64, QuoinError> {
    if relay_parts(channel).is_some() {
        return Err(QuoinError::Other(
            "a remote channel endpoint cannot be shipped onward — pass the channel \
             from its owning isolate instead"
                .to_string(),
        ));
    }
    ship_channel(vm, mc, channel, link)
}

/// True when `v` is a channel of either kind (local or relay endpoint) — the
/// crossing seams' probe.
pub(crate) fn is_channel_value<'gc>(v: Value<'gc>) -> bool {
    v.with_native_state::<crate::runtime::channel::NativeChannelState, _, _>(|_| ())
        .is_ok()
        || relay_parts(v).is_some()
}

/// `(link, chan)` when `receiver` is a relay endpoint, `None` for a local
/// channel — the branch every `Channel` selector takes first.
pub(crate) fn relay_parts<'gc>(receiver: Value<'gc>) -> Option<(usize, u64)> {
    receiver
        .with_native_state::<NativeRelayChannel, _, _>(|s| (s.link, s.chan))
        .ok()
}

/// Emit `Release` frames for endpoints dropped since the last flush (a GC
/// `Drop` can't send one itself — the reap pattern).
pub(crate) fn flush_chan_reap(vm: &VmState<'_>, link: usize) {
    let Some(l) = vm.io.chan_links.get(link) else {
        return;
    };
    let drained: Vec<u64> = l.reap.borrow_mut().drain(..).collect();
    for chan in drained {
        let _ = l.out.try_send(ChanFrame::Release { chan });
    }
}

/// Spawn the link's relay-agent task if it isn't running: resolves the qnlib
/// `ChannelRelayBoot` helper, whose `spawn:` does `Task.spawn:{
/// Channel.relayAgent:link }` — the only way native code mints a task is
/// through a Quoin block, and this is that block.
pub(crate) fn ensure_relay_agent<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    link: usize,
) -> Result<(), QuoinError> {
    let Some(l) = vm.io.chan_links.get_mut(link) else {
        return Err(QuoinError::Other(format!(
            "channel relay: no worker link {link}"
        )));
    };
    if l.agent_running {
        return Ok(());
    }
    l.agent_running = true;
    let boot =
        crate::runtime::extension::resolve_global(vm, "ChannelRelayBoot").ok_or_else(|| {
            QuoinError::Other("channel relay: ChannelRelayBoot is not installed (qnlib)".into())
        })?;
    let id = vm.new_int(mc, link as i64);
    vm.call_method(mc, boot, "spawn:", vec![id])?;
    Ok(())
}

/// Ship `channel` (a LOCAL channel value) across `link`: root it, bump its
/// endpoint refcount, make sure both this side's agent runs, and answer the
/// id the far side's endpoint will address.
pub(crate) fn ship_channel<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    channel: Value<'gc>,
    link: usize,
) -> Result<u64, QuoinError> {
    let id = vm.channel_ship(mc, channel)?;
    ensure_relay_agent(vm, mc, link)?;
    Ok(id)
}

/// Wrap a received channel id as a live relay endpoint of the `Channel`
/// class, and make sure this side's agent runs.
pub(crate) fn relay_endpoint<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    link: usize,
    chan: u64,
) -> Result<Value<'gc>, QuoinError> {
    ensure_relay_agent(vm, mc, link)?;
    relay_endpoint_raw(vm, mc, link, chan)
}

/// The allocation half of [`relay_endpoint`], for callers that already ran
/// `ensure_relay_agent` and must not yield again while unrooted GC values sit
/// on their frame (the agent boot runs Quoin — a task spawn — and can yield;
/// this half cannot).
pub(crate) fn relay_endpoint_raw<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    link: usize,
    chan: u64,
) -> Result<Value<'gc>, QuoinError> {
    let reap = vm
        .io
        .chan_links
        .get(link)
        .map(|l| l.reap.clone())
        .ok_or_else(|| QuoinError::Other(format!("channel relay: no worker link {link}")))?;
    let class = vm.get_or_create_builtin_class(mc, "Channel");
    Ok(vm.new_native_state(mc, class, NativeRelayChannel { link, chan, reap }))
}

/// Register a pending op and emit its frame; answers the correlation id.
fn begin_op<'gc>(
    vm: &mut VmState<'gc>,
    link: usize,
    chan: u64,
    make: impl FnOnce(u64) -> ChanFrame,
) -> Result<u64, QuoinError> {
    let me = vm.sched.current_task.0;
    let epoch = vm.current_park_epoch();
    let Some(l) = vm.io.chan_links.get_mut(link) else {
        return Err(QuoinError::Other(format!(
            "channel relay: no worker link {link}"
        )));
    };
    let corr = l.next_corr;
    l.next_corr += 1;
    l.pending.insert(
        corr,
        PendingChanOp {
            task: me,
            epoch,
            chan,
        },
    );
    let frame = make(corr);
    if l.out.try_send(frame).is_err() {
        l.pending.remove(&corr);
        return Err(QuoinError::Other(
            "channel endpoint: the owning isolate is gone".to_string(),
        ));
    }
    Ok(corr)
}

/// Park the current task on a relay op and hand back the raw wake.
fn relay_park<'gc>(
    vm: &mut VmState<'gc>,
    receiver: Value<'gc>,
    what: &str,
) -> Result<Option<Wake<'gc>>, QuoinError> {
    let me = vm.sched.current_task;
    if let Some(t) = vm.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
        t.parked_on_channel = true;
    }
    vm.set_park_info(what.to_string(), Some(receiver));
    if let Some(yielder) = unsafe { vm.get_yielder() } {
        yielder.suspend(YieldReason::ChannelPark);
    } else {
        return Err(QuoinError::Other(
            "channel operation attempted outside the VM scheduler".to_string(),
        ));
    }
    Ok(vm.sched.wake.take())
}

/// `send:` on a relay endpoint: serialize, frame, park for the `Ack`.
pub(crate) fn relay_send<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    value: Value<'gc>,
) -> Result<(), QuoinError> {
    let (link, chan) = relay_parts(receiver).expect("relay_send on a non-relay receiver");
    flush_chan_reap(vm, link);
    let wire = value_to_wire(value, None).map_err(|e| {
        QuoinError::ValueError(format!("values sent across isolates must be portable: {e}"))
    })?;
    let corr = begin_op(vm, link, chan, |corr| ChanFrame::Send {
        chan,
        corr,
        value: wire,
    })?;
    let wake = relay_park(vm, receiver, "relay channel send")?;
    if vm.sched.cancel_current {
        match wake {
            // The send was already accepted — nothing to retract.
            Some(Wake::ChannelSendOk) => {}
            // The pending entry STAYS until the owner answers — either the
            // op's own answer (racing this cancel) or the ClosedFor that
            // confirms the retraction — so a late answer is never orphaned.
            _ => vm.chan_emit(link, ChanFrame::Cancel { chan, corr }),
        }
        return Err(vm.take_cancellation());
    }
    match wake {
        Some(Wake::ChannelSendOk) => {
            let _ = mc;
            Ok(())
        }
        Some(Wake::ChannelClosed) => Err(QuoinError::ValueError(
            "send on a closed channel".to_string(),
        )),
        Some(Wake::ChannelErr { message }) => Err(QuoinError::ValueError(message)),
        _ => Err(QuoinError::Other(
            "relay channel send resumed without a result".to_string(),
        )),
    }
}

/// `receive` on a relay endpoint: frame, park for the `Value` (or closed →
/// `None`, rendered as nil / end of `each:`).
pub(crate) fn relay_recv<'gc>(
    vm: &mut VmState<'gc>,
    _mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
) -> Result<Option<Value<'gc>>, QuoinError> {
    let (link, chan) = relay_parts(receiver).expect("relay_recv on a non-relay receiver");
    flush_chan_reap(vm, link);
    let corr = begin_op(vm, link, chan, |corr| ChanFrame::Recv { chan, corr })?;
    let wake = relay_park(vm, receiver, "relay channel receive")?;
    if vm.sched.cancel_current {
        match wake {
            // A value already committed to this receiver must not vanish with
            // the cancellation: send it home for redelivery (it arrived over
            // the wire, so it converts back losslessly).
            Some(Wake::ChannelRecv { value }) => {
                if let Ok(wire) = value_to_wire(value, None) {
                    vm.chan_emit(link, ChanFrame::Return { chan, value: wire });
                }
            }
            // As with sends: the entry stays until the owner's answer or its
            // cancel confirmation resolves it.
            _ => vm.chan_emit(link, ChanFrame::Cancel { chan, corr }),
        }
        return Err(vm.take_cancellation());
    }
    match wake {
        Some(Wake::ChannelRecv { value }) => Ok(Some(value)),
        Some(Wake::ChannelClosed) => Ok(None),
        Some(Wake::ChannelErr { message }) => Err(QuoinError::ValueError(message)),
        _ => Err(QuoinError::Other(
            "relay channel receive resumed without a result".to_string(),
        )),
    }
}

/// `each:` on a relay endpoint: the consumer loop, over relay receives (ends
/// when the owner reports closed-and-drained). `block` and `receiver` are the
/// native call's arguments — rooted in `active_native_args` for its whole
/// duration, so holding them across the relay parks is safe (the same rooting
/// `channel_each` relies on).
pub(crate) fn relay_each<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    block: gc_arena::Gc<'gc, crate::value::Block<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    loop {
        match relay_recv(vm, mc, receiver)? {
            Some(value) => {
                vm.execute_block(mc, block, vec![value], None)?;
            }
            None => return Ok(vm.new_nil(mc)),
        }
    }
}

/// `close` on a relay endpoint: propagate to the owner (idempotent there).
pub(crate) fn relay_close<'gc>(vm: &mut VmState<'gc>, receiver: Value<'gc>) {
    let (link, chan) = relay_parts(receiver).expect("relay_close on a non-relay receiver");
    flush_chan_reap(vm, link);
    vm.chan_emit(link, ChanFrame::Close { chan });
}

/// The relay-agent loop (`Channel.relayAgent:`): drain the link's inbound
/// lane until it closes, applying each frame. Owner-side frames go to the
/// channel machinery; endpoint-side answers resolve pending ops.
pub(crate) fn relay_agent<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    link: usize,
) -> Result<(), QuoinError> {
    let inbound = vm
        .io
        .chan_links
        .get(link)
        .map(|l| l.inbound.clone())
        .ok_or_else(|| QuoinError::Other(format!("channel relay: no worker link {link}")))?;
    loop {
        flush_chan_reap(vm, link);
        match vm.await_io(IoRequest::ChanRecv(inbound.clone()))? {
            IoResult::ChanFrame(Some(frame)) => {
                let frame = *frame;
                match frame {
                    ChanFrame::Ack { corr }
                    | ChanFrame::Value { corr, .. }
                    | ChanFrame::ClosedFor { corr }
                    | ChanFrame::RecvError { corr, .. } => {
                        resolve_pending(vm, mc, link, corr, frame)?;
                    }
                    owner_frame => vm.channel_apply_owner_frame(mc, link, owner_frame)?,
                }
            }
            // Link closed: the counterpart isolate is gone; pending ops'
            // tasks stay parked on a lane that will never answer — wake them
            // as closed so nothing hangs.
            IoResult::ChanFrame(None) => {
                let pending: Vec<PendingChanOp> = vm
                    .io
                    .chan_links
                    .get_mut(link)
                    .map(|l| l.pending.drain().map(|(_, p)| p).collect())
                    .unwrap_or_default();
                for p in pending {
                    if vm.channel_waiter_live(TaskId(p.task), p.epoch) {
                        vm.wake_channel_task(TaskId(p.task), Wake::ChannelClosed);
                    }
                }
                return Ok(());
            }
            other => {
                return Err(QuoinError::Other(format!(
                    "channel relay: unexpected result {other:?}"
                )));
            }
        }
    }
}

/// Resolve one endpoint-side answer against the pending-op table.
fn resolve_pending<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    link: usize,
    corr: u64,
    frame: ChanFrame,
) -> Result<(), QuoinError> {
    let Some(p) = vm
        .io
        .chan_links
        .get_mut(link)
        .and_then(|l| l.pending.remove(&corr))
    else {
        // Each correlation gets exactly one answer (the op's, or the owner's
        // cancel confirmation) — a missing entry is a duplicate; ignore it.
        return Ok(());
    };
    let live = vm.channel_waiter_live(TaskId(p.task), p.epoch);
    match frame {
        // Ghost (cancelled) ops: an Ack means the send completed and nothing
        // is owed; a ClosedFor/RecvError has nobody left to tell.
        ChanFrame::Ack { .. } if live => {
            vm.wake_channel_task(TaskId(p.task), Wake::ChannelSendOk);
        }
        ChanFrame::Value { value, .. } => {
            if live {
                let v = wire_to_value(vm, mc, &value, None)?;
                vm.wake_channel_task(TaskId(p.task), Wake::ChannelRecv { value: v });
            } else {
                // Committed value, cancelled receiver: home for redelivery.
                vm.chan_emit(
                    link,
                    ChanFrame::Return {
                        chan: p.chan,
                        value,
                    },
                );
            }
        }
        ChanFrame::ClosedFor { .. } if live => {
            vm.wake_channel_task(TaskId(p.task), Wake::ChannelClosed);
        }
        ChanFrame::RecvError { message, .. } if live => {
            vm.wake_channel_task(TaskId(p.task), Wake::ChannelErr { message });
        }
        _ => {}
    }
    Ok(())
}
