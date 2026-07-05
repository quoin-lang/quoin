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
    /// Tasks parked in `receive` waiting for a value (FIFO), each with the park
    /// epoch captured when it enqueued — the entry is live only while the task is
    /// still parked at that exact epoch (see `channel_waiter_live`).
    pub recv_waiters: VecDeque<(TaskId, u64)>,
    /// Tasks parked in `send:` with their park epoch and the value they are trying
    /// to deliver (FIFO).
    pub send_waiters: VecDeque<(TaskId, u64, Value<'static>)>,
}

impl NativeChannelState {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            closed: false,
            buffer: VecDeque::new(),
            recv_waiters: VecDeque::new(),
            send_waiters: VecDeque::new(),
        }
    }

    pub(crate) fn buffer_mut<'gc>(&mut self) -> &mut VecDeque<Value<'gc>> {
        unsafe { transmute(&mut self.buffer) }
    }

    pub(crate) fn send_waiters_mut<'gc>(&mut self) -> &mut VecDeque<(TaskId, u64, Value<'gc>)> {
        unsafe { transmute(&mut self.send_waiters) }
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
        for (_, _, val) in &self.send_waiters {
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
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
    /// `parked_on_channel`, so this carries no payload — only plain data crosses the yield.
    #[allow(no_gc_across_yield)]
    fn channel_park(&mut self) -> Result<ParkOutcome<'gc>, QuoinError> {
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::ChannelPark);
        } else {
            return Err(QuoinError::Other(
                "channel operation attempted outside the VM scheduler".to_string(),
            ));
        }
        // On resume: a pending cancel on this task raises before consuming any result.
        if self.sched.cancel_current {
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

    /// True if `id` is still parked on a channel *at the same park epoch the entry was
    /// enqueued with* and not cancelled — i.e. a live waiter, not a stale ("ghost")
    /// queue entry left by a task that was cancelled or already woken. The epoch match
    /// is what makes this exact: `parked_on_channel` alone says the slot is parked on
    /// *some* channel, but after a cancel (or slot reuse — epochs are scheduler-global,
    /// so a recycled slot never repeats one) that can be a different channel than the
    /// one holding this entry, and delivering to it would misroute the value.
    fn channel_waiter_live(&self, id: TaskId, epoch: u64) -> bool {
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
    fn current_park_epoch(&self) -> u64 {
        self.sched
            .tasks
            .get(self.sched.current_task.0)
            .and_then(|t| t.as_ref())
            .map(|t| t.park_epoch)
            .unwrap_or(0)
    }

    /// Deliver `wake` to a parked channel task: clear its park flag and enqueue it ready.
    fn wake_channel_task(&mut self, id: TaskId, wake: Wake<'gc>) {
        if let Some(t) = self.sched.tasks.get_mut(id.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = false;
            t.wake = Some(wake);
            self.sched.ready.push_back(id);
        }
    }

    /// Pop the next *live* parked receiver, skipping ghosts. `None` if none waits.
    fn pop_live_recv(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Option<TaskId>, QuoinError> {
        loop {
            let id = receiver
                .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                    ch.recv_waiters.pop_front()
                })
                .map_err(QuoinError::Other)?;
            match id {
                None => return Ok(None),
                Some((id, epoch)) if self.channel_waiter_live(id, epoch) => return Ok(Some(id)),
                Some(_) => {} // ghost — skip
            }
        }
    }

    /// Pop the next *live* parked sender and its value, skipping ghosts (whose pending
    /// values are dropped — that sender was cancelled). `None` if none waits.
    fn pop_live_send(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Option<(TaskId, Value<'gc>)>, QuoinError> {
        loop {
            let entry = receiver
                .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                    ch.send_waiters_mut().pop_front()
                })
                .map_err(QuoinError::Other)?;
            match entry {
                None => return Ok(None),
                Some((id, epoch, value)) if self.channel_waiter_live(id, epoch) => {
                    return Ok(Some((id, value)));
                }
                Some(_) => {} // ghost — skip (its value is discarded)
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
    fn channel_send(
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
        // A waiting receiver takes the value directly (rendezvous / hand-off).
        if let Some(rid) = self.pop_live_recv(mc, receiver)? {
            self.wake_channel_task(rid, Wake::ChannelRecv { value });
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
                ch.send_waiters_mut().push_back((me, epoch, value));
            })
            .map_err(QuoinError::Other)?;
        if let Some(t) = self.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = true;
        }
        match self.channel_park()? {
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
        // move its value into the freed slot and wake it, keeping the buffer flowing.
        let buffered = receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| ch.buffer_mut().pop_front())
            .map_err(QuoinError::Other)?;
        if let Some(v) = buffered {
            if let Some((sid, sval)) = self.pop_live_send(mc, receiver)? {
                receiver
                    .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                        ch.buffer_mut().push_back(sval)
                    })
                    .map_err(QuoinError::Other)?;
                self.wake_channel_task(sid, Wake::ChannelSendOk);
            }
            return Ok(Some(v));
        }
        // Empty buffer: take directly from a parked sender (unbuffered rendezvous).
        if let Some((sid, sval)) = self.pop_live_send(mc, receiver)? {
            self.wake_channel_task(sid, Wake::ChannelSendOk);
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
                ch.recv_waiters.push_back((me, epoch))
            })
            .map_err(QuoinError::Other)?;
        if let Some(t) = self.sched.tasks.get_mut(me.0).and_then(|t| t.as_mut()) {
            t.parked_on_channel = true;
        }
        match self.channel_park()? {
            ParkOutcome::Received(v) => Ok(Some(v)),
            ParkOutcome::Closed => Ok(None),
            ParkOutcome::SendAccepted => Err(QuoinError::Other(
                "channel receive resumed as a send".to_string(),
            )),
        }
    }

    /// Close `receiver` (idempotent) and wake everyone parked: receivers observe a closed,
    /// drained channel; senders' pending sends fail. Buffered values remain receivable.
    fn channel_close(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let (recvs, sends): (Vec<(TaskId, u64)>, Vec<(TaskId, u64)>) = receiver
            .with_native_state_mut::<NativeChannelState, _, _>(mc, |ch| {
                ch.closed = true;
                let recvs = ch.recv_waiters.drain(..).collect();
                let sends = ch
                    .send_waiters
                    .drain(..)
                    .map(|(id, ep, _)| (id, ep))
                    .collect();
                (recvs, sends)
            })
            .map_err(QuoinError::Other)?;
        for (id, epoch) in recvs {
            if self.channel_waiter_live(id, epoch) {
                self.wake_channel_task(id, Wake::ChannelClosed);
            }
        }
        for (id, epoch) in sends {
            if self.channel_waiter_live(id, epoch) {
                self.wake_channel_task(id, Wake::ChannelClosed);
            }
        }
        Ok(())
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
        // Channel.new -> an unbuffered (rendezvous) channel.
        .class_method("new", |vm, mc, _r, _a| Ok(make_channel(vm, mc, 0)))
        // Channel.buffered:n -> a channel that buffers up to n values (0 == unbuffered).
        .typed_class_method("buffered:", &["Integer"], |vm, mc, _r, args| {
            let n = arg!(args, Int, 0);
            if n < 0 {
                return Err(QuoinError::ValueError(format!(
                    "Channel.buffered: capacity must be >= 0, got {n}"
                )));
            }
            Ok(make_channel(vm, mc, n as usize))
        });
    b.instance_method("send:", |vm, mc, receiver, args| {
        vm.channel_send(mc, receiver, args[0])?;
        Ok(vm.new_nil(mc))
    })
    .instance_method("receive", |vm, mc, receiver, _args| {
        match vm.channel_recv(mc, receiver)? {
            Some(v) => Ok(v),
            None => Ok(vm.new_nil(mc)),
        }
    })
    .instance_method("close", |vm, mc, receiver, _args| {
        vm.channel_close(mc, receiver)?;
        Ok(vm.new_nil(mc))
    })
    .instance_method("closed?", |vm, mc, receiver, _args| {
        Ok(vm.new_bool(mc, vm.channel_is_closed(receiver)?))
    })
    // count -> the number of buffered (not-yet-received) values.
    .instance_method("count", |vm, mc, receiver, _args| {
        let n = receiver
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.buffer.len())
            .map_err(QuoinError::Other)?;
        Ok(vm.new_int(mc, n as i64))
    })
    // capacity -> the buffer capacity (0 for an unbuffered channel).
    .instance_method("capacity", |vm, mc, receiver, _args| {
        let cap = receiver
            .with_native_state::<NativeChannelState, _, _>(|ch| ch.cap)
            .map_err(QuoinError::Other)?;
        Ok(vm.new_int(mc, cap as i64))
    })
    .typed_instance_method("each:", &["Block"], |vm, mc, receiver, args| {
        let block = arg!(args, Block, 0);
        vm.channel_each(mc, receiver, block)
    })
}
