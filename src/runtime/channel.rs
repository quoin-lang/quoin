use crate::arg;
use crate::error::QuoinError;
use crate::fiber::YieldReason;
use crate::value::{AnyCollect, Block, NativeClassBuilder, Value};
use crate::vm::VmState;
use crate::vm_scheduler::{TaskId, Wake};

use gc_arena::Gc;
use gc_arena::Mutation;
use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::collections::VecDeque;
use std::mem::transmute;

// ============================================================================
// Channel — CSP-style async message passing between tasks.
//
// A channel coordinates *tasks* purely inside the VM (no I/O backend), so it follows
// the `gather`/`join` park/wake model rather than `await_io`: a `send:`/`receive` with
// no ready counterpart registers the running task in a waiter queue and suspends with
// `YieldReason::ChannelPark`; a counterpart (or `close`) sets that task's `Wake` and
// re-enqueues it to `ready`. State (the buffer + waiter queues) lives in the channel
// object — like `Map`/`Set`/`List` — so GC tracing is natural and no reap is needed.
//
// Capacity 0 is an unbuffered rendezvous (`Channel.new`): a send hands its value
// directly to a waiting receiver (or parks). A buffered channel (`Channel.buffered: n`)
// accepts up to `n` queued values before a send parks.
// ============================================================================

/// A parked receiver: a local task (park-epoch identity, the ghost rule), or a
/// remote endpoint's pending op on some worker link (ACTOR_OBJECTS.md §6) —
/// answered with a frame where a local waiter gets a wake.
#[derive(Debug, Clone, Copy)]
pub enum RecvWaiter {
    Local { task: TaskId, epoch: u64 },
    Remote { link: usize, corr: u64 },
}

/// A parked sender with the value it is trying to deliver.
#[derive(Debug)]
pub enum SendWaiter {
    Local {
        task: TaskId,
        epoch: u64,
        value: Value<'static>,
    },
    Remote {
        link: usize,
        corr: u64,
        value: Value<'static>,
    },
}

/// A live receiver popped off the queue — where the value goes.
enum PoppedRecv {
    Local(TaskId),
    Remote { link: usize, corr: u64 },
}

/// A live sender popped off the queue — its value, and who to tell.
enum PoppedSend<'gc> {
    Local(TaskId, Value<'gc>),
    Remote {
        link: usize,
        corr: u64,
        value: Value<'gc>,
    },
}

impl<'gc> PoppedSend<'gc> {
    fn value(&self) -> Value<'gc> {
        match self {
            PoppedSend::Local(_, v) => *v,
            PoppedSend::Remote { value, .. } => *value,
        }
    }
}

/// Set once a channel has been SHIPPED to another isolate: the id it is rooted
/// under in this VM's `hosted` table, and how many remote endpoints exist
/// (`Release` frames decrement; 0 unroots).
#[derive(Debug, Clone, Copy)]
pub struct ChanShip {
    pub id: u64,
    pub refs: usize,
}

/// Backing state for a `Channel`. Holds `Value`s (the buffer, and each parked sender's
/// pending value) that must outlive a yield, so `trace_gc` roots them.
#[derive(Debug)]
pub struct NativeChannelState {
    /// Buffer capacity; `0` is an unbuffered (rendezvous) channel.
    pub cap: usize,
    /// Set by `close`; rejects further sends and ends `receive`/`each:` once drained.
    pub closed: bool,
    /// Queued values awaiting a receiver (FIFO), up to `cap`.
    pub buffer: VecDeque<Value<'static>>,
    /// Parked receivers (FIFO): local tasks and remote endpoints alike — one
    /// queue, one fairness order.
    pub recv_waiters: VecDeque<RecvWaiter>,
    /// Parked senders with their values (FIFO).
    pub send_waiters: VecDeque<SendWaiter>,
    /// Present once shipped across an isolate boundary (see [`ChanShip`]).
    pub ship: Option<ChanShip>,
}

impl NativeChannelState {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            closed: false,
            buffer: VecDeque::new(),
            recv_waiters: VecDeque::new(),
            send_waiters: VecDeque::new(),
            ship: None,
        }
    }

    pub(crate) fn buffer_mut<'gc>(&mut self) -> &mut VecDeque<Value<'gc>> {
        unsafe { transmute(&mut self.buffer) }
    }

    pub(crate) fn push_send_waiter<'gc>(&mut self, task: TaskId, epoch: u64, value: Value<'gc>) {
        self.send_waiters.push_back(SendWaiter::Local {
            task,
            epoch,
            value: unsafe { transmute::<Value<'gc>, Value<'static>>(value) },
        });
    }

    pub(crate) fn push_remote_send<'gc>(&mut self, link: usize, corr: u64, value: Value<'gc>) {
        self.send_waiters.push_back(SendWaiter::Remote {
            link,
            corr,
            value: unsafe { transmute::<Value<'gc>, Value<'static>>(value) },
        });
    }
}

impl AnyCollect for NativeChannelState {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        for val in &self.buffer {
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
        for w in &self.send_waiters {
            let (SendWaiter::Local { value, .. } | SendWaiter::Remote { value, .. }) = w;
            let val_gc: &Value<'gc> = unsafe { transmute(value) };
            val_gc.dyn_trace(cc);
        }
    }
}

fn make_channel<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, cap: usize) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "Channel");
    vm.new_native_state(mc, class, NativeChannelState::new(cap))
}

/// How a `ChannelPark` resumed: a receiver was handed a value, a sender's value was
/// accepted, or the channel was closed while the task was parked.
enum ParkOutcome<'gc> {
    Received(Value<'gc>),
    SendAccepted,
    Closed,
}

impl<'gc> VmState<'gc> {
    /// Suspend the running task on a channel rendezvous and report how it was resumed.
    /// The caller has already registered itself in the channel's waiter queue and set
    /// `parked_on_channel`. `channel` (the native method's receiver, rooted in
    /// `active_native_args` — the `await_timeout` rooting argument) is needed on the
    /// resume side: a cancelled receiver may hold a wake whose value must go back.
    fn channel_park(
        &mut self,
        mc: &Mutation<'gc>,
        channel: Value<'gc>,
    ) -> Result<ParkOutcome<'gc>, QuoinError> {
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::ChannelPark);
        } else {
            return Err(QuoinError::Other(
                "channel operation attempted outside the VM scheduler".to_string(),
            ));
        }
        // On resume: a pending cancel on this task raises before consuming any result —
        // but a value a sender already committed to this receiver (the send reported
        // success) must not vanish with the cancellation: hand it to the next live
        // receiver, or put it back at the front of the buffer.
        if self.sched.cancel_current {
            if let Some(Wake::ChannelRecv { value }) = self.sched.wake.take() {
                self.channel_redeliver(mc, channel, value)?;
            }
            return Err(self.take_cancellation());
        }
        match self.sched.wake.take() {
            Some(Wake::ChannelRecv { value }) => Ok(ParkOutcome::Received(value)),
            Some(Wake::ChannelSendOk) => Ok(ParkOutcome::SendAccepted),
            Some(Wake::ChannelClosed) => Ok(ParkOutcome::Closed),
            _ => Err(QuoinError::Other(
                "channel park resumed without a result".to_string(),
            )),
        }
    }

    /// Emit a relay frame on a worker link's outbound lane. A closed lane means
    /// the counterpart isolate is gone — the frame is dropped (pending ops there
    /// observe the closure through their own lane).
    pub(crate) fn chan_emit(&self, link: usize, frame: crate::worker::ChanFrame) {
        if let Some(l) = self.io.chan_links.get(link) {
            let _ = l.out.try_send(frame);
        }
    }

    /// Hand `value` to a popped receiver — a wake for a local task, a `Value`
    /// frame for a remote endpoint (with `RecvError` + front-of-buffer refiling
    /// when a pre-shipping value turns out not to be portable).
    fn deliver_to_recv(
        &mut self,
        mc: &Mutation<'gc>,
        channel: Value<'gc>,
        popped: PoppedRecv,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        match popped {
            PoppedRecv::Local(task) => {
                self.wake_channel_task(task, Wake::ChannelRecv { value });
                Ok(true)
            }
            PoppedRecv::Remote { link, corr } => {
                match crate::runtime::extension::value_to_wire(value, None) {
                    Ok(wire) => {
                        self.chan_emit(link, crate::worker::ChanFrame::Value { corr, value: wire });
                        Ok(true)
                    }
                    Err(e) => {
                        self.chan_emit(
                            link,
                            crate::worker::ChanFrame::RecvError {
                                corr,
                                message: format!(
                                    "the received value cannot cross to the remote endpoint: {e}"
                                ),
                            },
                        );
                        channel
                            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                                ch.buffer_mut().push_front(value)
                            })
                            .map_err(QuoinError::Other)?;
                        Ok(false)
                    }
                }
            }
        }
    }

    /// Re-deliver `value` — already committed to `channel` by a completed send — after
    /// its intended receiver was cancelled: the next live parked receiver gets it, else
    /// it goes to the *front* of the buffer (even if that momentarily exceeds `cap`;
    /// the alternative is silently dropping a value whose send reported success).
    pub(crate) fn channel_redeliver(
        &mut self,
        mc: &Mutation<'gc>,
        channel: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        if let Some(popped) = self.pop_live_recv(mc, channel)? {
            if self.deliver_to_recv(mc, channel, popped, value)? {
                return Ok(());
            }
            // A non-portable value bounced off a remote receiver and was
            // refiled at the buffer front — done.
            return Ok(());
        }
        channel
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                ch.buffer_mut().push_front(value)
            })
            .map_err(QuoinError::Other)
    }

    /// True if `id` is still parked on a channel *at the same park epoch the entry was
    /// enqueued with* and not cancelled — i.e. a live waiter, not a stale ("ghost")
    /// queue entry left by a task that was cancelled or already woken. The epoch match
    /// is what makes this exact: `parked_on_channel` alone says the slot is parked on
    /// *some* channel, but after a cancel (or slot reuse — epochs are scheduler-global,
    /// so a recycled slot never repeats one) that can be a different channel than the
    /// one holding this entry, and delivering to it would misroute the value.
    pub(crate) fn channel_waiter_live(&self, id: TaskId, epoch: u64) -> bool {
        self.sched
            .tasks
            .get(id.0)
            .and_then(|t| t.as_ref())
            .map(|t| t.parked_on_channel && t.park_epoch == epoch && !t.cancel_requested)
            .unwrap_or(false)
    }

    /// The current task's park epoch, captured alongside its waiter-queue entry so a
    /// counterpart can later tell this park apart from any other (see
    /// `channel_waiter_live`).
    pub(crate) fn current_park_epoch(&self) -> u64 {
        self.sched
            .tasks
            .get(self.sched.current_task.0)
            .and_then(|t| t.as_ref())
            .map(|t| t.park_epoch)
            .unwrap_or(0)
    }

    /// Deliver `wake` to a parked channel task: clear its park flag and enqueue it ready.
    pub(crate) fn wake_channel_task(&mut self, id: TaskId, wake: Wake<'gc>) {
        if let Some(t) = self.sched.tasks.get_mut(id.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = false;
            t.wake = Some(wake);
            self.sched.ready.push_back(id);
        }
    }

    /// Pop the next *live* parked receiver, skipping local ghosts. Remote
    /// entries are always live — a cancelled remote op retracts itself with a
    /// `Cancel` frame before this could see it. `None` if none waits.
    fn pop_live_recv(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Option<PoppedRecv>, QuoinError> {
        loop {
            let entry = receiver
                .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                    ch.recv_waiters.pop_front()
                })
                .map_err(QuoinError::Other)?;
            match entry {
                None => return Ok(None),
                Some(RecvWaiter::Local { task, epoch }) => {
                    if self.channel_waiter_live(task, epoch) {
                        return Ok(Some(PoppedRecv::Local(task)));
                    }
                    // ghost — skip
                }
                Some(RecvWaiter::Remote { link, corr }) => {
                    return Ok(Some(PoppedRecv::Remote { link, corr }));
                }
            }
        }
    }

    /// Pop the next *live* parked sender and its value, skipping local ghosts
    /// (whose pending values are dropped — that sender was cancelled). `None`
    /// if none waits.
    fn pop_live_send(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Option<PoppedSend<'gc>>, QuoinError> {
        loop {
            let entry = receiver
                .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                    ch.send_waiters.pop_front()
                })
                .map_err(QuoinError::Other)?;
            match entry {
                None => return Ok(None),
                Some(SendWaiter::Local { task, epoch, value }) => {
                    if self.channel_waiter_live(task, epoch) {
                        let value: Value<'gc> = unsafe { transmute(value) };
                        return Ok(Some(PoppedSend::Local(task, value)));
                    }
                    // ghost — skip (its value is discarded)
                }
                Some(SendWaiter::Remote { link, corr, value }) => {
                    let value: Value<'gc> = unsafe { transmute(value) };
                    return Ok(Some(PoppedSend::Remote { link, corr, value }));
                }
            }
        }
    }

    /// A promoted sender's completion: wake a local task, ack a remote op.
    fn complete_send(&mut self, popped_from: &PoppedSend<'gc>) {
        match popped_from {
            PoppedSend::Local(task, _) => self.wake_channel_task(*task, Wake::ChannelSendOk),
            PoppedSend::Remote { link, corr, .. } => {
                self.chan_emit(*link, crate::worker::ChanFrame::Ack { corr: *corr });
            }
        }
    }

    fn channel_is_closed(&self, receiver: Value<'gc>) -> Result<bool, QuoinError> {
        receiver
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.closed)
            .map_err(QuoinError::Other)
    }

    /// Send `value` on `receiver`. Hands off to a waiting receiver, else buffers if there
    /// is room, else parks until a receiver takes it (or the channel closes → error).
    pub(crate) fn channel_send(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        if self.channel_is_closed(receiver)? {
            return Err(QuoinError::ValueError(
                "send on a closed channel".to_string(),
            ));
        }
        // A SHIPPED channel has remote receivers: every value must be able to
        // cross, checked here so the failure lands at the sender, catchably
        // and immediately, rather than at some later remote handoff.
        let shipped = receiver
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.ship.is_some())
            .map_err(QuoinError::Other)?;
        if shipped && let Err(e) = crate::runtime::extension::value_to_wire(value, None) {
            return Err(QuoinError::ValueError(format!(
                "this channel has remote endpoints; values must be portable: {e}"
            )));
        }
        // A waiting receiver takes the value directly (rendezvous / hand-off).
        // A remote receiver that cannot take it (non-portable pre-ship residue
        // is impossible here — checked above) never loops.
        if let Some(popped) = self.pop_live_recv(mc, receiver)? {
            self.deliver_to_recv(mc, receiver, popped, value)?;
            return Ok(());
        }
        // Otherwise buffer it if there is room.
        let buffered = receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                if ch.buffer.len() < ch.cap {
                    ch.buffer_mut().push_back(value);
                    true
                } else {
                    false
                }
            })
            .map_err(QuoinError::Other)?;
        if buffered {
            return Ok(());
        }
        // Full (or unbuffered with no receiver waiting): park as a sender.
        let me = self.sched.current_task;
        let epoch = self.current_park_epoch();
        receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                ch.push_send_waiter(me, epoch, value);
            })
            .map_err(QuoinError::Other)?;
        if let Some(t) = self.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = true;
        }
        self.set_park_info("channel send".to_string(), Some(receiver));
        match self.channel_park(mc, receiver)? {
            ParkOutcome::SendAccepted => Ok(()),
            ParkOutcome::Closed => Err(QuoinError::ValueError(
                "send on a closed channel".to_string(),
            )),
            ParkOutcome::Received(_) => Err(QuoinError::Other(
                "channel send resumed as a receive".to_string(),
            )),
        }
    }

    /// Receive a value from `receiver`, parking if none is available. Returns `None` when
    /// the channel is closed and drained (the caller renders that as nil / ends `each:`).
    fn channel_recv(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Option<Value<'gc>>, QuoinError> {
        // A buffered value (FIFO). If a sender was parked because the buffer was full,
        // move its value into the freed slot and complete it (a wake for a local
        // sender, an `Ack` frame for a remote one), keeping the buffer flowing.
        let buffered = receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| ch.buffer_mut().pop_front())
            .map_err(QuoinError::Other)?;
        if let Some(v) = buffered {
            if let Some(popped) = self.pop_live_send(mc, receiver)? {
                let sval = popped.value();
                receiver
                    .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                        ch.buffer_mut().push_back(sval)
                    })
                    .map_err(QuoinError::Other)?;
                self.complete_send(&popped);
            }
            return Ok(Some(v));
        }
        // Empty buffer: take directly from a parked sender (unbuffered rendezvous).
        if let Some(popped) = self.pop_live_send(mc, receiver)? {
            let sval = popped.value();
            self.complete_send(&popped);
            return Ok(Some(sval));
        }
        // Nothing available now: a closed channel is done; otherwise park.
        if self.channel_is_closed(receiver)? {
            return Ok(None);
        }
        let me = self.sched.current_task;
        let epoch = self.current_park_epoch();
        receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                ch.recv_waiters
                    .push_back(RecvWaiter::Local { task: me, epoch })
            })
            .map_err(QuoinError::Other)?;
        if let Some(t) = self.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = true;
        }
        self.set_park_info("channel receive".to_string(), Some(receiver));
        match self.channel_park(mc, receiver)? {
            ParkOutcome::Received(v) => Ok(Some(v)),
            ParkOutcome::Closed => Ok(None),
            ParkOutcome::SendAccepted => Err(QuoinError::Other(
                "channel receive resumed as a send".to_string(),
            )),
        }
    }

    /// Close `receiver` (idempotent) and wake everyone parked — local waiters
    /// with `ChannelClosed`, remote pending ops with `ClosedFor` frames:
    /// receivers observe a closed, drained channel; senders' pending sends
    /// fail. Buffered values remain receivable.
    pub(crate) fn channel_close(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<(), QuoinError> {
        enum Woken {
            Local(TaskId, u64),
            Remote(usize, u64),
        }
        let woken: Vec<Woken> = receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                ch.closed = true;
                let mut all = Vec::new();
                for w in ch.recv_waiters.drain(..) {
                    all.push(match w {
                        RecvWaiter::Local { task, epoch } => Woken::Local(task, epoch),
                        RecvWaiter::Remote { link, corr } => Woken::Remote(link, corr),
                    });
                }
                for w in ch.send_waiters.drain(..) {
                    all.push(match w {
                        SendWaiter::Local { task, epoch, .. } => Woken::Local(task, epoch),
                        SendWaiter::Remote { link, corr, .. } => Woken::Remote(link, corr),
                    });
                }
                all
            })
            .map_err(QuoinError::Other)?;
        for w in woken {
            match w {
                Woken::Local(id, epoch) => {
                    if self.channel_waiter_live(id, epoch) {
                        self.wake_channel_task(id, Wake::ChannelClosed);
                    }
                }
                Woken::Remote(link, corr) => {
                    self.chan_emit(link, crate::worker::ChanFrame::ClosedFor { corr });
                }
            }
        }
        Ok(())
    }

    /// Root `channel` for shipping across an isolate boundary (or bump its
    /// endpoint refcount if already shipped) and answer its hosted id
    /// (ACTOR_OBJECTS.md §6).
    pub(crate) fn channel_ship(
        &mut self,
        mc: &Mutation<'gc>,
        channel: Value<'gc>,
    ) -> Result<u64, QuoinError> {
        let already = channel
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.ship)
            .map_err(QuoinError::Other)?;
        if let Some(ship) = already {
            channel
                .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                    if let Some(s) = &mut ch.ship {
                        s.refs += 1;
                    }
                })
                .map_err(QuoinError::Other)?;
            return Ok(ship.id);
        }
        let id = self.hosted_insert(channel);
        channel
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                ch.ship = Some(ChanShip { id, refs: 1 });
            })
            .map_err(QuoinError::Other)?;
        Ok(id)
    }

    /// OWNER side of the channel relay (§6): apply one inbound frame from the
    /// worker link `link`, targeting a channel this VM rooted. Replies (acks,
    /// values, closures) go back out on the same link. Endpoint-side frames
    /// (`Ack`/`Value`/`ClosedFor`/`RecvError`) are the relay agent's own
    /// business and never reach here.
    pub(crate) fn channel_apply_owner_frame(
        &mut self,
        mc: &Mutation<'gc>,
        link: usize,
        frame: crate::worker::ChanFrame,
    ) -> Result<(), QuoinError> {
        use crate::worker::ChanFrame as F;
        match frame {
            F::Send { chan, corr, value } => {
                let Some(chv) = self.hosted_get(chan) else {
                    self.chan_emit(link, F::ClosedFor { corr });
                    return Ok(());
                };
                if self.channel_is_closed(chv)? {
                    self.chan_emit(link, F::ClosedFor { corr });
                    return Ok(());
                }
                let v = crate::runtime::extension::wire_to_value(self, mc, &value, None)?;
                // Hand off / buffer / queue — the local send flow, with the
                // remote sender acked instead of woken. The value came off the
                // wire, so a remote-receiver handoff can never bounce.
                if let Some(popped) = self.pop_live_recv(mc, chv)? {
                    self.deliver_to_recv(mc, chv, popped, v)?;
                    self.chan_emit(link, F::Ack { corr });
                    return Ok(());
                }
                let buffered = chv
                    .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                        if ch.buffer.len() < ch.cap {
                            ch.buffer_mut().push_back(v);
                            true
                        } else {
                            false
                        }
                    })
                    .map_err(QuoinError::Other)?;
                if buffered {
                    self.chan_emit(link, F::Ack { corr });
                    return Ok(());
                }
                chv.with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                    ch.push_remote_send(link, corr, v);
                })
                .map_err(QuoinError::Other)?;
                Ok(())
            }
            F::Recv { chan, corr } => {
                let Some(chv) = self.hosted_get(chan) else {
                    self.chan_emit(link, F::ClosedFor { corr });
                    return Ok(());
                };
                // Mirror channel_recv, answering with frames.
                let buffered = chv
                    .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                        ch.buffer_mut().pop_front()
                    })
                    .map_err(QuoinError::Other)?;
                let taken = if let Some(v) = buffered {
                    if let Some(popped) = self.pop_live_send(mc, chv)? {
                        let sval = popped.value();
                        chv.with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                            ch.buffer_mut().push_back(sval)
                        })
                        .map_err(QuoinError::Other)?;
                        self.complete_send(&popped);
                    }
                    Some(v)
                } else if let Some(popped) = self.pop_live_send(mc, chv)? {
                    let sval = popped.value();
                    self.complete_send(&popped);
                    Some(sval)
                } else {
                    None
                };
                match taken {
                    Some(v) => match crate::runtime::extension::value_to_wire(v, None) {
                        Ok(wire) => {
                            self.chan_emit(link, F::Value { corr, value: wire });
                        }
                        Err(e) => {
                            // Pre-shipping residue that cannot cross: fail the
                            // remote op, keep the value receivable locally.
                            self.chan_emit(
                                link,
                                F::RecvError {
                                    corr,
                                    message: format!(
                                        "the received value cannot cross to the remote \
                                         endpoint: {e}"
                                    ),
                                },
                            );
                            chv.with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                                ch.buffer_mut().push_front(v)
                            })
                            .map_err(QuoinError::Other)?;
                        }
                    },
                    None => {
                        if self.channel_is_closed(chv)? {
                            self.chan_emit(link, F::ClosedFor { corr });
                        } else {
                            chv.with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                                ch.recv_waiters.push_back(RecvWaiter::Remote { link, corr });
                            })
                            .map_err(QuoinError::Other)?;
                        }
                    }
                }
                Ok(())
            }
            F::Close { chan } => {
                if let Some(chv) = self.hosted_get(chan) {
                    self.channel_close(mc, chv)?;
                }
                Ok(())
            }
            F::Cancel { chan, corr } => {
                if let Some(chv) = self.hosted_get(chan) {
                    chv.with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                        ch.recv_waiters
                            .retain(|w| !matches!(w, RecvWaiter::Remote { link: l, corr: c } if *l == link && *c == corr));
                        ch.send_waiters
                            .retain(|w| !matches!(w, SendWaiter::Remote { link: l, corr: c, .. } if *l == link && *c == corr));
                    })
                    .map_err(QuoinError::Other)?;
                }
                Ok(())
            }
            F::Release { chan } => {
                if let Some(chv) = self.hosted_get(chan) {
                    let now_unshipped = chv
                        .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                            if let Some(s) = &mut ch.ship {
                                s.refs = s.refs.saturating_sub(1);
                                if s.refs == 0 {
                                    ch.ship = None;
                                    return true;
                                }
                            }
                            false
                        })
                        .map_err(QuoinError::Other)?;
                    if now_unshipped {
                        self.hosted_release(chan);
                    }
                }
                Ok(())
            }
            F::Return { chan, value } => {
                if let Some(chv) = self.hosted_get(chan) {
                    let v = crate::runtime::extension::wire_to_value(self, mc, &value, None)?;
                    self.channel_redeliver(mc, chv, v)?;
                }
                Ok(())
            }
            F::Ack { .. } | F::Value { .. } | F::ClosedFor { .. } | F::RecvError { .. } => Ok(()),
        }
    }

    /// `channel.each:{|v| …}` — run the block on each value until the channel is closed
    /// and drained, parking between values. A non-local exit / throw / cancel from the
    /// block propagates straight out (like `acceptLoop:`).
    fn channel_each(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        block: Gc<'gc, Block<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        loop {
            match self.channel_recv(mc, receiver)? {
                Some(value) => {
                    self.execute_block(mc, block, vec![value], None)?;
                }
                None => return Ok(self.new_nil(mc)),
            }
        }
    }
}

pub fn build_channel_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("Channel", Some("Object"))
        .construct_with("use Channel.new or Channel.buffered:")
        .class_doc(
            "A CSP-style channel for passing values between tasks. `Channel.new` is an \
             unbuffered rendezvous -- a send parks until a receiver takes the value; \
             `Channel.buffered:n` queues up to n values before sends park. `close` ends the \
             conversation: further sends raise, receives drain the buffer then answer nil, \
             and `each:` ends. Channels also CROSS ISOLATE BOUNDARIES: sent to a \
             thread-backed worker (or passed to / returned from a hosted service), the far \
             side gets a live endpoint whose operations relay here with the same semantics \
             -- values deep-copy and must be portable, backpressure crosses, close \
             propagates both ways.\n\n\
             ```\n\
             var ch = Channel.new;\n\
             Task.spawn:{ ch.send:42 };\n\
             ch.receive    \"* -> 42\n\
             ```",
        )
        // Channel.new -> an unbuffered (rendezvous) channel.
        .class_method("new", |vm, mc, _r, _a| Ok(make_channel(vm, mc, 0)))
        .doc("An unbuffered (rendezvous) channel: every send waits for its receiver.")
        // The relay-agent loop (§6): drains one worker link's inbound relay
        // lane. Spawned by qnlib's ChannelRelayBoot the first time a channel
        // crosses that link; not a user-facing surface.
        .class_method("relayAgent:", |vm, mc, _r, args| {
            let link = args[0]
                .as_i64()
                .ok_or_else(|| QuoinError::Other("Channel.relayAgent: expects a link id".into()))?
                as usize;
            crate::runtime::channel_relay::relay_agent(vm, mc, link)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Worker-link relay agent (internal): applies inbound channel-relay frames \
             for one link until it closes. Spawned automatically when a channel is \
             shipped across an isolate boundary; not meant to be called directly.",
        )
        // Channel.buffered:n -> a channel that buffers up to n values (0 == unbuffered).
        .typed_class_method("buffered:", &["Integer"], |vm, mc, _r, args| {
            let n = arg!(args, Int, 0);
            if n < 0 {
                return Err(QuoinError::ValueError(format!(
                    "Channel.buffered: capacity must be >= 0, got {n}"
                )));
            }
            Ok(make_channel(vm, mc, n as usize))
        })
        .doc(
            "A channel that buffers up to n values before sends park (0 is unbuffered).\n\n\
             ```\n\
             var ch = Channel.buffered:2;\n\
             ch.send:1; ch.send:2; ch.close;\n\
             #( ch.receive ch.receive )    \"* -> #(1 2)\n\
             ```",
        );
    b.instance_method("send:", |vm, mc, receiver, args| {
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            crate::runtime::channel_relay::relay_send(vm, mc, receiver, args[0])?;
            return Ok(vm.new_nil(mc));
        }
        vm.channel_send(mc, receiver, args[0])?;
        Ok(vm.new_nil(mc))
    })
    .doc(
        "Send a value: hand it to a waiting receiver, else buffer it if there is room, \
         else park until a receiver takes it. Raises on a closed channel. Answers nil.",
    )
    .instance_method("receive", |vm, mc, receiver, _args| {
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            return match crate::runtime::channel_relay::relay_recv(vm, mc, receiver)? {
                Some(v) => Ok(v),
                None => Ok(vm.new_nil(mc)),
            };
        }
        match vm.channel_recv(mc, receiver)? {
            Some(v) => Ok(v),
            None => Ok(vm.new_nil(mc)),
        }
    })
    .doc(
        "The next value, parking until one is available -- buffered values first (FIFO), \
         else directly from a parked sender. On a closed, drained channel answers nil.",
    )
    .instance_method("close", |vm, mc, receiver, _args| {
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            crate::runtime::channel_relay::relay_close(vm, receiver);
            return Ok(vm.new_nil(mc));
        }
        vm.channel_close(mc, receiver)?;
        Ok(vm.new_nil(mc))
    })
    .doc(
        "Close the channel (idempotent); answers nil. Parked and future sends raise; \
         buffered values remain receivable; a drained receive answers nil and `each:` \
         ends.",
    )
    .instance_method("closed?", |vm, mc, receiver, _args| {
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            return Err(QuoinError::Other(
                "closed? is not available on a remote channel endpoint (the state \
                 lives in the owning isolate); receive answers nil once closed and \
                 drained"
                    .to_string(),
            ));
        }
        Ok(vm.new_bool(mc, vm.channel_is_closed(receiver)?))
    })
    .doc("True once the channel has been closed.")
    // count -> the number of buffered (not-yet-received) values.
    .instance_method("count", |vm, mc, receiver, _args| {
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            return Err(QuoinError::Other(
                "count is not available on a remote channel endpoint (the buffer \
                 lives in the owning isolate)"
                    .to_string(),
            ));
        }
        let n = receiver
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.buffer.len())
            .map_err(QuoinError::Other)?;
        Ok(vm.new_int(mc, n as i64))
    })
    .doc("How many values are currently buffered (sent but not yet received).")
    // capacity -> the buffer capacity (0 for an unbuffered channel).
    .instance_method("capacity", |vm, mc, receiver, _args| {
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            return Err(QuoinError::Other(
                "capacity is not available on a remote channel endpoint (the buffer \
                 lives in the owning isolate)"
                    .to_string(),
            ));
        }
        let cap = receiver
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.cap)
            .map_err(QuoinError::Other)?;
        Ok(vm.new_int(mc, cap as i64))
    })
    .doc("The buffer capacity; 0 for an unbuffered (rendezvous) channel.")
    .typed_instance_method("each:", &["Block"], |vm, mc, receiver, args| {
        let block = arg!(args, Block, 0);
        if crate::runtime::channel_relay::relay_parts(receiver).is_some() {
            loop {
                match crate::runtime::channel_relay::relay_recv(vm, mc, receiver)? {
                    Some(value) => {
                        vm.execute_block(mc, block, vec![value], None)?;
                    }
                    None => return Ok(vm.new_nil(mc)),
                }
            }
        }
        vm.channel_each(mc, receiver, block)
    })
    .doc(
        "Run the block on each received value until the channel is closed and drained, \
         parking between values; answers nil.\n\n\
         ```\n\
         var ch = Channel.buffered:3;\n\
         ch.send:1; ch.send:2; ch.close;\n\
         var sum = 0;\n\
         ch.each:{ |v| sum = sum + v };\n\
         sum    \"* -> 3\n\
         ```",
    )
}
