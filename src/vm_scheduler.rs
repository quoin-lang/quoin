//! Execution-scheduling subsystem extracted from `vm.rs`: the async task scheduler
//! (`Async.gather:` / `Task.spawn:` / `Async.timeout:`) and the guest-`Fiber` coroutine
//! machinery. Still intrinsically VM state — these are `impl VmState` methods and the
//! `Scheduler` struct embedded in `VmState` — hence the `vm_` prefix; split out only so
//! the growing `vm.rs` stays legible. See `docs/ASYNC_ARCH.md`.

use crate::error::QuoinError;
use crate::fiber::{Fiber, YieldReason};
use crate::gc;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::fiber::{FiberStatus, NativeFiberState};
use crate::runtime::task::NativeTaskHandle;
use crate::value::{Block, NativeCall, ObjectPayload, Value};
use crate::vm::{Frame, VmState};

use futures_util::future::AbortHandle;
use gc_arena::{Collect, Gc, Mutation};
use std::collections::VecDeque;

/// Index of a top-level scheduler task in [`Scheduler::tasks`]. Plain data, so it
/// crosses the yield boundary and the arena/runner boundary freely.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TaskId(pub usize);

/// A top-level **task**: an independently schedulable line of execution the runner
/// owns. Distinct from a guest `Fiber` (an asymmetric coroutine driven by explicit
/// `resume`/`yield`) — a task is scheduled by the runner, and Stage 2 overlaps the
/// I/O of several at once. Built on the same `corosensei` coroutine primitive: each
/// task has its own root coroutine and, while parked, its own stash of the
/// per-task slice of `VmState` (the live task keeps that slice in `VmState`
/// directly; see `save_task_context`/`load_task_context`, added in Stage 2a-ii).
/// See `docs/ASYNC_ARCH.md`.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Task<'gc> {
    /// This task's root coroutine (the analogue of the old single `active_fiber`).
    pub coro: Gc<'gc, Fiber<'gc>>,
    /// The root coroutine's yielder slot (per-task replacement for the old global
    /// `main_yielder`). The driver restores it into `Scheduler::yielder` before
    /// resuming this task's root, so it never dangles.
    #[collect(require_static)]
    pub root_yielder: Option<*const ()>,
    /// The block this task runs. `None` for the main task (#0), whose block is
    /// already started into the live context before the task is created. A spawned
    /// child carries its block here and starts it on first activation (`started`).
    pub block: Option<Gc<'gc, Block<'gc>>>,
    /// Whether the root coroutine has begun. The first activation of a child
    /// installs an empty live context and starts `block` (mirrors guest fibers).
    pub started: bool,
    /// Stashed per-task slice of `VmState`, valid only while this task is *not* the
    /// current one (the live task keeps these in `VmState` directly). Saved/restored
    /// across a task switch by `save_task_context`/`load_task_context`.
    pub stack: Vec<Value<'gc>>,
    pub frames: Vec<Frame<'gc>>,
    pub native_args: Vec<NativeCall<'gc>>,
    pub current_fiber: Option<Value<'gc>>,
    pub resume_stack: Vec<Option<Value<'gc>>>,
    /// The task-root execution context, stashed here while this task is parked
    /// *inside* guest-fiber execution (its root frames live in the scheduler's
    /// `main_saved_*` slot while a fiber runs). Those scheduler slots are shared,
    /// so a parked task must carry its own copy across a task switch — otherwise
    /// another task's fiber switch overwrites them, and this task's next fiber
    /// yield would load an empty (or foreign) root context: a silently-completing
    /// or frame-corrupted task.
    pub saved_root_stack: Vec<Value<'gc>>,
    pub saved_root_frames: Vec<Frame<'gc>>,
    pub saved_root_native_args: Vec<NativeCall<'gc>>,
    /// Result to deliver when this task is next resumed (an I/O result, or a gather
    /// outcome). Stashed here while parked; moved into `Scheduler::wake` by
    /// `load_task_context`, then taken by `await_io`/`await_gather`.
    pub wake: Option<Wake<'gc>>,
    /// For a gather *child*: which parent task awaits it, and the result slot index
    /// to report into. `None` for the main task and any detached task.
    #[collect(require_static)]
    pub parent: Option<(TaskId, usize)>,
    /// For a task parked in `gather`: the children it is waiting on and their
    /// results so far.
    pub gather: Option<GatherState<'gc>>,
    /// For a *detached* (`Task.spawn:`) task: its QN handle object. A running task
    /// roots its handle here, so the handle cannot vanish while the task runs; on
    /// completion `complete_detached` writes the outcome into the handle and frees
    /// this slot, after which the handle lives by normal QN reachability. `None` for
    /// gather children and the main task.
    pub handle: Option<Value<'gc>>,
    /// Tasks parked in `join` on this one, woken with the outcome on completion.
    #[collect(require_static)]
    pub waiters: Vec<TaskId>,
    /// Set by `handle.cancel`: at the task's next checkpoint (a VM step, or a park
    /// resume) it raises `QuoinError::Cancelled`, unwinding through `finally`. Copied
    /// to the live `Scheduler::cancel_current` by `load_task_context`.
    #[collect(require_static)]
    pub cancel_requested: bool,
    /// While parked on I/O: the handle that aborts the in-flight future, so `cancel`
    /// interrupts a `sleep`/read promptly. Set by the driver on park, cleared on the
    /// op's completion and on becoming current.
    #[collect(require_static)]
    pub abort_handle: Option<AbortHandle>,
    /// While parked in `join`: the task being joined, so `cancel` can dequeue this
    /// joiner from that target's waiter list and wake it. Cleared on becoming current.
    #[collect(require_static)]
    pub joining: Option<TaskId>,
    /// Park identity, stamped from the *global* `Scheduler::park_seq` each time this
    /// task becomes current (`load_task_context`). A `JoinTimed` deadline timer and a
    /// channel waiter-queue entry capture the epoch at park time; a stale capture whose
    /// epoch no longer matches is ignored. Global (not per-task) allocation is what makes
    /// this exact across slot reuse: a fresh task in a recycled slot can never repeat an
    /// epoch the previous occupant parked at, so a ghost timer/queue entry can never
    /// impersonate the new occupant.
    #[collect(require_static)]
    pub park_epoch: u64,
    /// While parked in `JoinTimed`: aborts the in-flight deadline timer, so a normal
    /// completion (or a cancel) can disarm the deadline promptly instead of letting it
    /// linger in the reactor. Set by the driver on park, taken on wake.
    #[collect(require_static)]
    pub deadline_abort: Option<AbortHandle>,
    /// While parked on a `Channel` send/receive: set before the `ChannelPark` suspend so
    /// `cancel` can make this task runnable (it matches none of the I/O/join branches),
    /// and so a counterpart can tell a live waiter from a stale ("ghost") queue entry left
    /// behind by a cancelled task. Cleared on becoming current and when a counterpart wakes
    /// it. See `src/runtime/channel.rs`.
    #[collect(require_static)]
    pub parked_on_channel: bool,
}

/// Bookkeeping for a task parked in `Async.gather:`: it resumes once `pending`
/// reaches zero, with `results` (in spawn order) or the `first_error` a child hit.
#[derive(Collect)]
#[collect(no_drop)]
pub struct GatherState<'gc> {
    pub pending: usize,
    pub results: Vec<Option<Value<'gc>>>,
    #[collect(require_static)]
    pub first_error: Option<QuoinError>,
}

/// The value the scheduler delivers to a parked task on resume. Plain I/O results
/// and gather errors carry no `Gc`; a gather's result list does. Unifies what were
/// two separate Stage-1 slots so a growing scheduler has one delivery channel.
#[derive(Collect)]
#[collect(no_drop)]
pub enum Wake<'gc> {
    Io {
        #[collect(require_static)]
        result: IoResult,
    },
    Gather {
        list: Value<'gc>,
    },
    GatherErr {
        #[collect(require_static)]
        error: QuoinError,
    },
    /// A joined task finished normally; `value` is its result.
    Joined {
        value: Value<'gc>,
    },
    /// A joined task finished with an uncaught exception; `error` is the exception
    /// value, re-raised in the joiner (a *catchable* throw — distinct from the
    /// joiner's own cancellation).
    Failed {
        error: Value<'gc>,
    },
    /// A joined task was cancelled. The joiner observes this as a *catchable* error
    /// (distinct from its own uncatchable `Cancelled`).
    JoinedCancelled,
    /// A `JoinTimed` park hit its deadline before the joined task finished. Delivered
    /// by the driver when the deadline timer wins the race (see `Async.timeout:do:`).
    TimedOut,
    /// A parked `Channel` receiver was handed `value` by a sender (`channel.rs`).
    ChannelRecv {
        value: Value<'gc>,
    },
    /// A parked `Channel` sender's value was accepted by a receiver — the send succeeds.
    ChannelSendOk,
    /// A parked `Channel` sender/receiver was woken by `close`. A receiver observes an
    /// empty, closed channel (returns nil / ends `each:`); a sender raises "send on a
    /// closed channel".
    ChannelClosed,
}

/// Classification of a finished detached task's outcome, used by `complete_detached`
/// to write the handle and wake joiners. A plain transient local (not arena state).
#[derive(Clone, Copy)]
enum DetachedOutcome<'gc> {
    Done(Value<'gc>),
    Failed(Value<'gc>),
    Cancelled,
}

/// Outcome of a `JoinTimed` park (`await_join_timed`): either the joined task finished
/// with a value before the deadline, or the deadline won the race. The error cases
/// (the child threw / was cancelled / our own cancellation) are returned as `Err`.
enum TimedJoin<'gc> {
    Completed(Value<'gc>),
    TimedOut,
}

/// Coroutine / guest-fiber scheduler state, grouped out of [`VmState`] for
/// legibility.
///
/// All of this is execution-scheduling bookkeeping: the running coroutine's
/// yielder, the top-level task table and which one is current, the active/current
/// fiber, the resume chain, the value mailbox handed across a switch, the saved
/// main-program context while a guest fiber runs, and a slot for an error raised
/// inside a fiber. Stored inline by value in `VmState` (no indirection); split
/// purely so the growing struct stays readable. Stage 1's I/O round-trip slots
/// join here.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Scheduler<'gc> {
    /// Yielder of the *currently running* coroutine. Set by the driver from the
    /// running fiber's stored slot before every resume, so it can never dangle.
    #[collect(require_static)]
    pub yielder: Option<*const ()>,
    /// Top-level task table, indexed by [`TaskId`]; a `None` slot is a finished
    /// task. The run/test driver schedules over this. (Benchmark mode uses the
    /// simpler single-root `active_fiber` path below and leaves this empty.)
    pub tasks: Vec<Option<Task<'gc>>>,
    /// Runnable task ids. `spawn`/`gather`/completion/I/O-wakeup all enqueue here;
    /// the driver pops the next task to run (FIFO, or random under `QN_SCHED_STRESS`).
    /// Lives here (not in the runner) so a native `spawn` can enqueue without parking.
    #[collect(require_static)]
    pub ready: VecDeque<TaskId>,
    /// The task whose per-task state is currently live in `VmState`.
    #[collect(require_static)]
    pub current_task: TaskId,
    /// Single root coroutine for the benchmark driver, which does not use the task
    /// table. `None` once it finishes. The run/test driver leaves this unused.
    pub active_fiber: Option<Gc<'gc, Fiber<'gc>>>,
    /// The guest `Fiber` currently executing, or `None` when the main program
    /// (fiber #0) is running. The scheduler in the driver keeps this in sync.
    pub current_fiber: Option<Value<'gc>>,
    /// Chain of resumers: each entry is whoever resumed the fiber above it
    /// (`None` == the main program). A `yield` pops this to find who to return to.
    pub resume_stack: Vec<Option<Value<'gc>>>,
    /// One-slot mailbox for the value handed across a fiber switch (the arg to
    /// `resume:`, or the value out of `yield:`). Written by the scheduler, read
    /// by the resumed coroutine.
    pub fiber_transfer: Option<Value<'gc>>,
    /// Saved execution context for the main program while a guest fiber runs.
    pub main_saved_stack: Vec<Value<'gc>>,
    pub main_saved_frames: Vec<Frame<'gc>>,
    pub main_saved_native_args: Vec<NativeCall<'gc>>,
    /// An error raised inside a guest fiber, delivered to its resumer.
    #[collect(require_static)]
    pub fiber_error: Option<QuoinError>,
    /// Delivery slot for the *current* task, loaded from its `Task::wake` just
    /// before resuming and taken by `await_io`/`await_gather` (Stage 2a). The
    /// outgoing I/O request no longer needs a slot — the driver receives it
    /// directly from the `AwaitIo` suspension.
    pub wake: Option<Wake<'gc>>,
    /// Live mirror of the current task's `cancel_requested`, checked on the hot path
    /// (a bool, not a table index — and always `false` in benchmark mode, which has
    /// no task table). Set by `load_task_context`; cleared when a checkpoint raises
    /// `Cancelled`, so the ensuing `finally` unwind is not itself re-cancelled.
    #[collect(require_static)]
    pub cancel_current: bool,
    /// Monotonic allocator for [`Task::park_epoch`]: bumped and stamped onto a task
    /// each time it becomes current. Scheduler-global so epochs are unique across all
    /// tasks and all slot reuses — never reset (not even by `reset_scheduler`), so a
    /// channel entry or deadline timer surviving a REPL line stays inert too.
    #[collect(require_static)]
    pub park_seq: u64,
}

impl<'gc> VmState<'gc> {
    // =====================================================================
    // Guest fiber support
    //
    // `fiber_resume` / `fiber_yield` run inside native `Fiber` methods (deep in
    // `step`). They bubble a `YieldReason` up to the scheduler in the driver,
    // which performs the actual context switch via the `*_switch` helpers below
    // and re-enters the appropriate coroutine. The transfer value rides in the
    // GC-rooted `fiber_transfer` slot, so nothing is held only on the suspended
    // native stack across the yield.
    // =====================================================================

    /// Resume `fiber_val`, delivering `arg`. Returns the value the fiber yields,
    /// or its final return value when it completes. Called from `f.resume[:]`.
    #[allow(no_gc_across_yield)]
    pub fn fiber_resume(
        &mut self,
        mc: &Mutation<'gc>,
        fiber_val: Value<'gc>,
        arg: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        match self.fiber_status(fiber_val)? {
            FiberStatus::Done => {
                return Err(self.raise_fiber_error(mc, "cannot resume a finished Fiber"));
            }
            FiberStatus::Failed => {
                return Err(self.raise_fiber_error(mc, "cannot resume a failed Fiber"));
            }
            _ => {}
        }
        // Within the current task, the only Running fiber is the current one and
        // ancestors are Suspended — these two checks cover the current task's chain.
        if self.sched.current_fiber == Some(fiber_val) {
            return Err(self.raise_fiber_error(mc, "a Fiber cannot resume itself"));
        }
        if self
            .sched
            .resume_stack
            .iter()
            .any(|f| *f == Some(fiber_val))
        {
            return Err(self.raise_fiber_error(
                mc,
                "cannot resume a Fiber that is currently resuming this one (would deadlock)",
            ));
        }
        // Cross-task: a fiber live inside ANOTHER task (its current fiber, or an
        // ancestor on its resume chain — that task may be parked mid-fiber on I/O,
        // or preempted) has its real context live in or stashed with that task,
        // not in its own state. Re-entering it from here would load an empty
        // context and resume its coroutine at a foreign suspend point — corrupting
        // both tasks, failing the fiber, and aborting the whole process when the
        // owning task later re-resumes the now-completed coroutine.
        if let Some(owner) = fiber_val
            .with_native_state::<NativeFiberState, _, _>(|s| s.owner)
            .map_err(QuoinError::Other)?
        {
            if owner != self.sched.current_task {
                return Err(self.raise_fiber_error(
                    mc,
                    "cannot resume a Fiber that is running in another task",
                ));
            }
        }

        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::ResumeFiber {
                fiber: fiber_val,
                arg,
            });
        } else {
            return Err(QuoinError::Other(
                "Fiber.resume called outside the VM scheduler".to_string(),
            ));
        }
        // On resume the driver has already restored `self.sched.yielder` for us.

        if let Some(err) = self.sched.fiber_error.take() {
            return Err(err);
        }
        Ok(self
            .sched
            .fiber_transfer
            .take()
            .unwrap_or_else(|| self.new_nil(mc)))
    }

    /// Suspend the running fiber, handing `value` to whoever resumed it. Returns
    /// the value passed to the next `resume:`. Called from `Fiber.yield[:]`.
    #[allow(no_gc_across_yield)]
    pub fn fiber_yield(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        if self.sched.current_fiber.is_none() {
            return Err(self.raise_fiber_error(mc, "Fiber.yield: called outside of a Fiber"));
        }

        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::YieldFiber { value });
        } else {
            return Err(QuoinError::Other(
                "Fiber.yield: called outside the VM scheduler".to_string(),
            ));
        }
        // On resume the driver has already restored `self.sched.yielder` for us.

        if let Some(err) = self.sched.fiber_error.take() {
            return Err(err);
        }
        Ok(self
            .sched
            .fiber_transfer
            .take()
            .unwrap_or_else(|| self.new_nil(mc)))
    }

    /// Consume a pending cancellation at a checkpoint: clear the live flag *and* the
    /// current task's durable `cancel_requested`, so cancellation is one-shot — the
    /// ensuing `finally` unwind (and any preempt-reload during it) is not re-cancelled.
    /// Returns the `Cancelled` error to raise.
    pub(crate) fn take_cancellation(&mut self) -> QuoinError {
        self.sched.cancel_current = false;
        if let Some(t) = self
            .sched
            .tasks
            .get_mut(self.sched.current_task.0)
            .and_then(|t| t.as_mut())
        {
            t.cancel_requested = false;
        }
        QuoinError::Cancelled
    }

    /// Suspend the running coroutine to perform async I/O, returning the result the
    /// scheduler delivers on resume. Mirrors `fiber_yield`: the request bubbles up as
    /// `YieldReason::AwaitIo`, and on resume the driver has stashed the answer in
    /// `self.sched.wake`. Only plain data crosses the yield. Works from the main
    /// program too (it runs as a task whose root coroutine has its own `root_yielder`).
    #[allow(no_gc_across_yield)]
    pub fn await_io(&mut self, req: IoRequest) -> Result<IoResult, QuoinError> {
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::AwaitIo { req });
        } else {
            return Err(QuoinError::Other(
                "I/O attempted outside the VM scheduler".to_string(),
            ));
        }
        // On resume: a pending cancel raises before consuming any result.
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        // The driver has stashed the result for us.
        match self.sched.wake.take() {
            Some(Wake::Io { result }) => Ok(result),
            _ => Err(QuoinError::Other(
                "I/O resumed without a result".to_string(),
            )),
        }
    }

    /// Suspend the running task to spawn one child task per block and wait for all of
    /// them (`Async.gather:`). The blocks bubble up as `YieldReason::Gather`; the
    /// scheduler runs the children concurrently and resumes this task with the list of
    /// results in `self.sched.wake` (or the first child error). See `docs/ASYNC_ARCH.md`.
    #[allow(no_gc_across_yield)]
    pub fn await_gather(
        &mut self,
        blocks: Vec<Gc<'gc, Block<'gc>>>,
    ) -> Result<Value<'gc>, QuoinError> {
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::Gather { blocks });
        } else {
            return Err(QuoinError::Other(
                "gather attempted outside the VM scheduler".to_string(),
            ));
        }
        // On resume: a pending cancel raises (the gather's children have finished by
        // now — see the v1 scope note in docs/ASYNC_ARCH.md).
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        // The driver has stashed the assembled result list (or an error).
        match self.sched.wake.take() {
            Some(Wake::Gather { list }) => Ok(list),
            Some(Wake::GatherErr { error }) => Err(error),
            _ => Err(QuoinError::Other(
                "gather resumed without a result".to_string(),
            )),
        }
    }

    /// Park the running task until `target` (a still-running detached task) finishes,
    /// returning its result — or re-raising its exception (a catchable throw). Called
    /// from `handle.join` only when the handle's status is `Running`; the join of an
    /// already-finished task reads the outcome straight off the handle without parking.
    #[allow(no_gc_across_yield)]
    pub fn await_join(&mut self, target: TaskId) -> Result<Value<'gc>, QuoinError> {
        // Register as a waiter on the target before suspending (same step, so the
        // target cannot complete in between); record the target so `cancel` can dequeue
        // and wake this joiner.
        let me = self.sched.current_task;
        match self.sched.tasks.get_mut(target.0).and_then(|t| t.as_mut()) {
            Some(t) => t.waiters.push(me),
            None => {
                return Err(QuoinError::Other(
                    "join target is no longer running".to_string(),
                ));
            }
        }
        if let Some(t) = self.sched.tasks[me.0].as_mut() {
            t.joining = Some(target);
        }
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::Join { task: target });
        } else {
            return Err(QuoinError::Other(
                "join attempted outside the VM scheduler".to_string(),
            ));
        }
        // On resume: a pending cancel on *this* joiner raises first.
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        // The driver has stashed the outcome.
        match self.sched.wake.take() {
            Some(Wake::Joined { value }) => Ok(value),
            Some(Wake::Failed { error }) => {
                self.exceptions.active = Some(error);
                Err(QuoinError::Thrown)
            }
            // The joined task was cancelled — a *catchable* observation, not the
            // joiner's own cancellation.
            Some(Wake::JoinedCancelled) => {
                Err(QuoinError::Other("joined task was cancelled".to_string()))
            }
            _ => Err(QuoinError::Other(
                "join resumed without a result".to_string(),
            )),
        }
    }

    /// Like [`await_join`], but parks on `target` *or* a deadline of `ms` ms — whichever
    /// fires first (the join machinery for `Async.timeout:do:`). Registers as a waiter so
    /// completion wakes us, then bubbles `JoinTimed`; the driver arms the deadline timer
    /// and resumes us with whichever won in `wake`. `Wake::TimedOut` means the deadline.
    #[allow(no_gc_across_yield)]
    fn await_join_timed(&mut self, target: TaskId, ms: u64) -> Result<TimedJoin<'gc>, QuoinError> {
        let me = self.sched.current_task;
        match self.sched.tasks.get_mut(target.0).and_then(|t| t.as_mut()) {
            Some(t) => t.waiters.push(me),
            None => {
                return Err(QuoinError::Other(
                    "timeout join target is no longer running".to_string(),
                ));
            }
        }
        if let Some(t) = self.sched.tasks[me.0].as_mut() {
            t.joining = Some(target);
        }
        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::JoinTimed { task: target, ms });
        } else {
            return Err(QuoinError::Other(
                "timeout attempted outside the VM scheduler".to_string(),
            ));
        }
        // On resume: an *outer* cancel of this task raises first (handler is skipped).
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        match self.sched.wake.take() {
            Some(Wake::Joined { value }) => Ok(TimedJoin::Completed(value)),
            Some(Wake::Failed { error }) => {
                self.exceptions.active = Some(error);
                Err(QuoinError::Thrown)
            }
            Some(Wake::JoinedCancelled) => {
                Err(QuoinError::Other("joined task was cancelled".to_string()))
            }
            Some(Wake::TimedOut) => Ok(TimedJoin::TimedOut),
            _ => Err(QuoinError::Other(
                "timeout join resumed without a result".to_string(),
            )),
        }
    }

    /// `Async.timeout:ms do:{block}` (and `… onCancel:{handler}`). Run `block` as a child
    /// task raced against a deadline of `ms` ms:
    /// - finishes first → its value (or its error/`Cancelled` propagate);
    /// - deadline first → cancel and drain the child (its `finally` runs, in-flight I/O
    ///   aborts), then run the handler — `onCancel:` returns its value; the bare form
    ///   throws a catchable `TimeoutError` (carrying the deadline `ms`);
    /// - this call is cancelled from *outside* while waiting → cancel/drain the child and
    ///   re-raise `Cancelled` (the handler does *not* run).
    ///
    /// `on_cancel` is rooted as a live argument of the calling native method, so holding
    /// it across the await is sound. See `docs/ASYNC_ARCH.md` (Stage 5a).
    pub fn await_timeout(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ms: u64,
        on_cancel: Option<Gc<'gc, Block<'gc>>>,
    ) -> Result<Value<'gc>, QuoinError> {
        // Spawn the work before any suspend, so `block` is consumed (not held across it).
        let child = self.spawn_detached_id(mc, block);
        match self.await_join_timed(child, ms) {
            Ok(TimedJoin::Completed(value)) => Ok(value),
            Ok(TimedJoin::TimedOut) => {
                self.drain_cancelled(child);
                match on_cancel {
                    Some(handler) => self.execute_block(mc, handler, vec![], None),
                    None => Err(QuoinError::Timeout { ms: ms as i64 }),
                }
            }
            // Outer cancellation: tear down the child, then propagate (handler skipped).
            Err(QuoinError::Cancelled) => {
                self.drain_cancelled(child);
                Err(QuoinError::Cancelled)
            }
            // The child finished with an error/cancel and is already gone; propagate it.
            Err(e) => Err(e),
        }
    }

    /// Cancel `child` (if still running) and wait for it to unwind, discarding the
    /// outcome — so its `finally` runs and its resources free before we move on. The
    /// `request_cancel`/`await_join` pair is atomic w.r.t. the child (single-threaded:
    /// nothing runs between them), so the child cannot vanish in the gap.
    #[allow(no_gc_across_yield)]
    fn drain_cancelled(&mut self, child: TaskId) {
        if self
            .sched
            .tasks
            .get(child.0)
            .and_then(|t| t.as_ref())
            .is_none()
        {
            return; // already finished
        }
        self.request_cancel(child);
        if self
            .sched
            .tasks
            .get(child.0)
            .and_then(|t| t.as_ref())
            .is_some()
        {
            let _ = self.await_join(child); // JoinedCancelled expected; we want the unwind
        }
    }

    /// Deliver a `JoinTimed` deadline. If `joiner` is still parked on this exact
    /// timed-join — `target`/`epoch` match and no wake is pending — disarm the join
    /// (drop it from `target`'s waiters) and wake it with `Wake::TimedOut`. A stale or
    /// superseded timer (the join already completed, or the joiner was resumed and
    /// re-parked, possibly on a reused slot id) is ignored. This, run inside the
    /// single-threaded scheduler, is the exact deadline-vs-completion race resolution.
    pub fn deliver_deadline(&mut self, joiner: TaskId, target: TaskId, epoch: u64) {
        let live = self
            .sched
            .tasks
            .get(joiner.0)
            .and_then(|t| t.as_ref())
            .is_some_and(|t| {
                t.joining == Some(target) && t.park_epoch == epoch && t.wake.is_none()
            });
        if !live {
            return;
        }
        if let Some(tt) = self.sched.tasks.get_mut(target.0).and_then(|t| t.as_mut()) {
            tt.waiters.retain(|w| *w != joiner);
        }
        let t = self.sched.tasks[joiner.0].as_mut().unwrap();
        t.joining = None;
        t.deadline_abort = None;
        t.wake = Some(Wake::TimedOut);
        self.sched.ready.push_back(joiner);
    }

    fn fiber_status(&self, fiber_val: Value<'gc>) -> Result<FiberStatus, QuoinError> {
        fiber_val
            .with_native_state::<NativeFiberState, _, _>(|s| s.status)
            .map_err(QuoinError::Other)
    }

    /// Park a structured `FiberError` in `active_exception` and return the
    /// `Thrown` signal, so fiber misuse is catchable by type in Quoin code.
    fn raise_fiber_error(&mut self, mc: &Mutation<'gc>, msg: &str) -> QuoinError {
        let err = self.make_error(mc, "FiberError", msg, None);
        self.exceptions.active = Some(err);
        QuoinError::Thrown
    }

    fn set_fiber_status(&self, mc: &Mutation<'gc>, fiber_val: Value<'gc>, status: FiberStatus) {
        let _ =
            fiber_val.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.status = status);
    }

    /// Record which task `fiber_val` is live inside (`None` = not in any task's
    /// resume chain). Set on resume, cleared on yield/completion; `fiber_resume`
    /// refuses a cross-task resume while it is set.
    fn set_fiber_owner(&self, mc: &Mutation<'gc>, fiber_val: Value<'gc>, owner: Option<TaskId>) {
        let _ = fiber_val.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.owner = owner);
    }

    /// Save the live VM execution context into the slot for `who` (`None` = main).
    fn save_fiber_context(
        &mut self,
        mc: &Mutation<'gc>,
        who: Option<Value<'gc>>,
    ) -> Result<(), QuoinError> {
        let stack = std::mem::take(&mut self.stack);
        let frames = std::mem::take(&mut self.frames);
        let native_args = std::mem::take(&mut self.active_native_args);
        match who {
            None => {
                self.sched.main_saved_stack = stack;
                self.sched.main_saved_frames = frames;
                self.sched.main_saved_native_args = native_args;
            }
            Some(f) => {
                f.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| {
                    s.set_context(stack, frames, native_args)
                })
                .map_err(QuoinError::Other)?;
            }
        }
        Ok(())
    }

    /// Load the saved context for `who` (`None` = main) into the live VM fields.
    fn load_fiber_context(
        &mut self,
        mc: &Mutation<'gc>,
        who: Option<Value<'gc>>,
    ) -> Result<(), QuoinError> {
        let (stack, frames, native_args) = match who {
            None => (
                std::mem::take(&mut self.sched.main_saved_stack),
                std::mem::take(&mut self.sched.main_saved_frames),
                std::mem::take(&mut self.sched.main_saved_native_args),
            ),
            Some(f) => f
                .with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.take_context())
                .map_err(QuoinError::Other)?,
        };
        self.stack = stack;
        self.frames = frames;
        self.active_native_args = native_args;
        Ok(())
    }

    /// Scheduler: switch from the running coroutine to `fiber_val`, delivering
    /// `arg`. Pushes the caller onto the resume stack.
    pub fn do_resume_switch(
        &mut self,
        mc: &Mutation<'gc>,
        fiber_val: Value<'gc>,
        arg: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let outgoing = self.sched.current_fiber;
        self.save_fiber_context(mc, outgoing)?;
        if let Some(of) = outgoing {
            self.set_fiber_status(mc, of, FiberStatus::Suspended);
        }
        self.sched.resume_stack.push(outgoing);
        self.sched.current_fiber = Some(fiber_val);

        let started = fiber_val
            .with_native_state::<NativeFiberState, _, _>(|s| s.started)
            .map_err(QuoinError::Other)?;

        self.load_fiber_context(mc, Some(fiber_val))?;

        if started {
            self.sched.fiber_transfer = Some(arg);
        } else {
            // First activation: bind `arg` to the block's parameters.
            let block_val = fiber_val
                .with_native_state::<NativeFiberState, _, _>(|s| s.block())
                .map_err(QuoinError::Other)?;
            let block_gc = match block_val {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::Block(b) => *b,
                    _ => return Err(QuoinError::Other("Fiber target is not a Block".to_string())),
                },
                _ => return Err(QuoinError::Other("Fiber target is not a Block".to_string())),
            };
            self.start_block(mc, block_gc, vec![arg], None, None);
            fiber_val
                .with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.started = true)
                .map_err(QuoinError::Other)?;
        }
        self.set_fiber_status(mc, fiber_val, FiberStatus::Running);
        // The incoming fiber joins this task's resume chain. The outgoing one (now
        // Suspended, an ancestor on the chain) keeps its owner — it is still live here.
        self.set_fiber_owner(mc, fiber_val, Some(self.sched.current_task));
        Ok(())
    }

    /// Scheduler: the running fiber yielded `value`; return control to its resumer.
    pub fn do_yield_switch(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let outgoing = self.sched.current_fiber;
        self.save_fiber_context(mc, outgoing)?;
        if let Some(of) = outgoing {
            self.set_fiber_status(mc, of, FiberStatus::Suspended);
            // A yield removes the fiber from this task's resume chain: its context
            // is back in its own state, so any task may legally resume it now.
            self.set_fiber_owner(mc, of, None);
        }
        let resumer = self.sched.resume_stack.pop().unwrap_or(None);
        self.sched.current_fiber = resumer;
        self.load_fiber_context(mc, resumer)?;
        if let Some(rf) = resumer {
            self.set_fiber_status(mc, rf, FiberStatus::Running);
        }
        self.sched.fiber_transfer = Some(value);
        Ok(())
    }

    /// Scheduler: the running fiber's block returned (or errored); mark it done
    /// and return control to its resumer with the result.
    pub fn do_fiber_done(
        &mut self,
        mc: &Mutation<'gc>,
        result: Result<Value<'gc>, QuoinError>,
    ) -> Result<(), QuoinError> {
        // Record the outcome on the finished fiber for `result`/`error`/`status`.
        if let Some(finished) = self.sched.current_fiber {
            match &result {
                Ok(val) => {
                    let v = *val;
                    self.set_fiber_status(mc, finished, FiberStatus::Done);
                    let _ = finished
                        .with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.set_result(v));
                }
                Err(e) => {
                    // The error value is the parked Quoin exception, or a converted
                    // internal error. Peek (don't take) so the resumer still sees it.
                    let err_val = match self.exceptions.active {
                        Some(v) => v,
                        None => self.quoinerror_to_value(mc, e),
                    };
                    self.set_fiber_status(mc, finished, FiberStatus::Failed);
                    let _ = finished.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| {
                        s.set_error(err_val)
                    });
                }
            }
            self.set_fiber_owner(mc, finished, None);
        }
        // The finished fiber's execution context is discarded.
        self.stack.clear();
        self.frames.clear();
        self.active_native_args.clear();

        let resumer = self.sched.resume_stack.pop().unwrap_or(None);
        self.sched.current_fiber = resumer;
        self.load_fiber_context(mc, resumer)?;
        if let Some(rf) = resumer {
            self.set_fiber_status(mc, rf, FiberStatus::Running);
        }
        match result {
            Ok(val) => self.sched.fiber_transfer = Some(val),
            Err(err) => self.sched.fiber_error = Some(err),
        }
        Ok(())
    }

    // =====================================================================
    // Task scheduling support (Stage 2a)
    //
    // A top-level task owns a private slice of `VmState` (the data stack, frame
    // stack, native-call stack, and its guest-fiber chain). The *current* task
    // keeps that slice live in `VmState`; every other task stashes it in its
    // `Task`. `save_task_context`/`load_task_context` swap the slice in and out at
    // a task switch — the I/O-parking analogue of `save_/load_fiber_context`. The
    // driver in `runner.rs` decides *when* to switch; these just move the state.
    // =====================================================================

    /// Discard transient execution/scheduler bookkeeping so the next top-level run starts on a
    /// clean slate. The REPL reuses one `VmState` across lines; a line that errors mid-fiber or
    /// mid-native-call can leave the run-queue, guest-fiber chain, saved main context, native
    /// args, or value/error mailboxes dirty. The task table itself is rebuilt by the caller
    /// (`install_main_task`); frames/stack are restored separately by `end_repl_line`. (The
    /// file runner aborts on the first error and never continues, so it has no use for this.)
    pub(crate) fn reset_scheduler(&mut self) {
        self.active_native_args.clear();
        self.sched.yielder = None;
        self.sched.ready.clear();
        self.sched.current_fiber = None;
        self.sched.resume_stack.clear();
        self.sched.fiber_transfer = None;
        self.sched.main_saved_stack.clear();
        self.sched.main_saved_frames.clear();
        self.sched.main_saved_native_args.clear();
        self.sched.fiber_error = None;
        self.sched.wake = None;
        self.sched.cancel_current = false;
    }

    /// Stash the live per-task context into `tasks[tid]` (the task is parking or
    /// being preempted). Mailboxes (`fiber_transfer`/`fiber_error`) are empty at
    /// every switch boundary, so they are not saved. The `main_saved_*` slots ARE
    /// saved: they hold this task's root context whenever it parks mid-fiber, and
    /// the next task's fiber switches would otherwise clobber them.
    pub fn save_task_context(&mut self, tid: TaskId) {
        let stack = std::mem::take(&mut self.stack);
        let frames = std::mem::take(&mut self.frames);
        let native_args = std::mem::take(&mut self.active_native_args);
        let current_fiber = self.sched.current_fiber.take();
        let resume_stack = std::mem::take(&mut self.sched.resume_stack);
        let saved_root_stack = std::mem::take(&mut self.sched.main_saved_stack);
        let saved_root_frames = std::mem::take(&mut self.sched.main_saved_frames);
        let saved_root_native_args = std::mem::take(&mut self.sched.main_saved_native_args);
        let t = self.sched.tasks[tid.0]
            .as_mut()
            .expect("save_task_context: task slot is empty");
        t.stack = stack;
        t.frames = frames;
        t.native_args = native_args;
        t.current_fiber = current_fiber;
        t.resume_stack = resume_stack;
        t.saved_root_stack = saved_root_stack;
        t.saved_root_frames = saved_root_frames;
        t.saved_root_native_args = saved_root_native_args;
    }

    /// Make `tid` the current task and restore its context into `VmState`. The
    /// caller must already have saved (or discarded) the outgoing task's context.
    /// On a child's first activation, installs an empty context and starts its block.
    pub fn load_task_context(&mut self, mc: &Mutation<'gc>, tid: TaskId) {
        self.sched.current_task = tid;
        {
            // Becoming current: surface any pending cancel, drop the join park-state
            // (no longer waiting on anything), stamp a fresh globally-unique park
            // epoch, and clear any deadline-timer handle (a `JoinTimed` park is over
            // once we run).
            self.sched.park_seq += 1;
            let t = self.sched.tasks[tid.0]
                .as_mut()
                .expect("load_task_context: task slot is empty");
            self.sched.cancel_current = t.cancel_requested;
            t.joining = None;
            t.parked_on_channel = false;
            t.park_epoch = self.sched.park_seq;
            t.deadline_abort = None;
        }
        let started = self.sched.tasks[tid.0].as_ref().unwrap().started;
        if started {
            let t = self.sched.tasks[tid.0].as_mut().unwrap();
            self.stack = std::mem::take(&mut t.stack);
            self.frames = std::mem::take(&mut t.frames);
            self.active_native_args = std::mem::take(&mut t.native_args);
            self.sched.current_fiber = t.current_fiber.take();
            self.sched.resume_stack = std::mem::take(&mut t.resume_stack);
            self.sched.main_saved_stack = std::mem::take(&mut t.saved_root_stack);
            self.sched.main_saved_frames = std::mem::take(&mut t.saved_root_frames);
            self.sched.main_saved_native_args = std::mem::take(&mut t.saved_root_native_args);
            self.sched.wake = t.wake.take();
        } else {
            // First activation: a fresh, empty live context, then start the block.
            self.stack = Vec::new();
            self.frames = Vec::new();
            self.active_native_args = Vec::new();
            self.sched.current_fiber = None;
            self.sched.resume_stack = Vec::new();
            self.sched.main_saved_stack = Vec::new();
            self.sched.main_saved_frames = Vec::new();
            self.sched.main_saved_native_args = Vec::new();
            self.sched.wake = None;
            let block = self.sched.tasks[tid.0]
                .as_ref()
                .unwrap()
                .block
                .expect("a spawned task must carry a block");
            self.start_block(mc, block, Vec::new(), None, None);
            self.sched.tasks[tid.0].as_mut().unwrap().started = true;
        }
    }

    /// Spawn one child task per block, parking the current task on a fresh
    /// `GatherState` and enqueueing the children as ready (in spawn order). The
    /// current task's context is saved here.
    pub fn spawn_gather(&mut self, mc: &Mutation<'gc>, blocks: Vec<Gc<'gc, Block<'gc>>>) {
        let parent = self.sched.current_task;
        self.save_task_context(parent);
        let n = blocks.len();
        if n == 0 {
            // No children means nothing will ever call `complete_child`, whose
            // pending==0 check is the only delivery path — deliver the empty result
            // now, or the parent parks forever (and the program "succeeds" silently).
            let list = self.new_list(mc, Vec::new());
            let pt = self.sched.tasks[parent.0].as_mut().unwrap();
            pt.wake = Some(Wake::Gather { list });
            self.sched.ready.push_back(parent);
            return;
        }
        self.sched.tasks[parent.0].as_mut().unwrap().gather = Some(GatherState {
            pending: n,
            results: vec![None; n],
            first_error: None,
        });
        for (slot, block) in blocks.into_iter().enumerate() {
            let task = self.new_child_task(mc, block, Some((parent, slot)), None);
            let id = self.alloc_task(task);
            self.sched.ready.push_back(id);
        }
    }

    /// Spawn a detached task running `block` and return its QN handle. The spawner is
    /// not parked — the new task is enqueued as ready and runs when scheduled; the
    /// handle (a `Task`-class object over the new id) is rooted by the task while it
    /// runs and receives the outcome on completion (see `complete_detached`).
    pub fn spawn_detached(&mut self, mc: &Mutation<'gc>, block: Gc<'gc, Block<'gc>>) -> Value<'gc> {
        let id = self.spawn_detached_id(mc, block);
        self.sched.tasks[id.0]
            .as_ref()
            .unwrap()
            .handle
            .expect("spawn_detached_id just set the handle")
    }

    /// As [`spawn_detached`], but returns the new task's [`TaskId`]. A handle is still
    /// created (`complete_detached` needs one to record the outcome and wake joiners),
    /// but the caller joins by id — used by `await_timeout`, which joins/cancels the
    /// child directly rather than through a QN handle.
    pub fn spawn_detached_id(&mut self, mc: &Mutation<'gc>, block: Gc<'gc, Block<'gc>>) -> TaskId {
        // Reserve the slot first so the handle can carry the real id, then root the
        // handle back on the task.
        let task = self.new_child_task(mc, block, None, None);
        let id = self.alloc_task(task);
        let class = self.get_or_create_builtin_class(mc, "Task");
        let handle = self.new_native_state(mc, class, NativeTaskHandle::new(id));
        self.sched.tasks[id.0].as_mut().unwrap().handle = Some(handle);
        self.sched.ready.push_back(id);
        id
    }

    /// Build a fresh, unstarted child/detached task wrapping `block`.
    fn new_child_task(
        &self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        parent: Option<(TaskId, usize)>,
        handle: Option<Value<'gc>>,
    ) -> Task<'gc> {
        let coro = Fiber::new(|yielder, ctx| crate::fiber::run_vm_loop(yielder, ctx));
        Task {
            coro: gc!(mc, coro),
            root_yielder: None,
            block: Some(block),
            started: false,
            stack: Vec::new(),
            frames: Vec::new(),
            native_args: Vec::new(),
            current_fiber: None,
            resume_stack: Vec::new(),
            saved_root_stack: Vec::new(),
            saved_root_frames: Vec::new(),
            saved_root_native_args: Vec::new(),
            wake: None,
            parent,
            gather: None,
            handle,
            waiters: Vec::new(),
            cancel_requested: false,
            abort_handle: None,
            joining: None,
            park_epoch: 0,
            deadline_abort: None,
            parked_on_channel: false,
        }
    }

    /// Install `task` in the first free slot (reusing a finished task's slot to keep
    /// the table from growing without bound), returning its id.
    fn alloc_task(&mut self, task: Task<'gc>) -> TaskId {
        if let Some(i) = self.sched.tasks.iter().position(|t| t.is_none()) {
            self.sched.tasks[i] = Some(task);
            TaskId(i)
        } else {
            self.sched.tasks.push(Some(task));
            TaskId(self.sched.tasks.len() - 1)
        }
    }

    /// A gather child `cur` finished with `result`. Record it into its parent's
    /// `GatherState` and free the child slot; once every sibling has finished, deliver
    /// the assembled result list (or the first error) to the parent and enqueue it as
    /// ready.
    pub fn complete_child(
        &mut self,
        mc: &Mutation<'gc>,
        cur: TaskId,
        result: Result<Value<'gc>, QuoinError>,
    ) {
        let (parent, slot) = self.sched.tasks[cur.0]
            .as_ref()
            .expect("complete_child: child slot is empty")
            .parent
            .expect("complete_child: child has no parent");
        self.sched.tasks[cur.0] = None; // free the child

        let done = {
            let pt = self.sched.tasks[parent.0]
                .as_mut()
                .expect("complete_child: parent slot is empty");
            let g = pt
                .gather
                .as_mut()
                .expect("complete_child: parent is not gathering");
            match result {
                Ok(val) => g.results[slot] = Some(val),
                Err(e) => {
                    if g.first_error.is_none() {
                        g.first_error = Some(e);
                    }
                }
            }
            g.pending -= 1;
            g.pending == 0
        };
        if !done {
            return;
        }

        // All children finished: assemble the wake (no borrow held across `new_list`).
        let gather = self.sched.tasks[parent.0]
            .as_mut()
            .unwrap()
            .gather
            .take()
            .unwrap();
        let wake = match gather.first_error {
            Some(error) => Wake::GatherErr { error },
            None => {
                let vals = gather
                    .results
                    .into_iter()
                    .map(|o| o.expect("a finished gather has every result"))
                    .collect();
                Wake::Gather {
                    list: self.new_list(mc, vals),
                }
            }
        };
        self.sched.tasks[parent.0].as_mut().unwrap().wake = Some(wake);
        self.sched.ready.push_back(parent);
    }

    /// A detached task `cur` finished with `result` (a value, an uncaught exception,
    /// or cancellation). Write the outcome into its handle, wake every joiner with it,
    /// and free the slot. After this the handle lives by normal QN reachability and
    /// carries the outcome for any later `join`.
    pub fn complete_detached(
        &mut self,
        mc: &Mutation<'gc>,
        cur: TaskId,
        result: Result<Value<'gc>, QuoinError>,
    ) {
        let handle = self.sched.tasks[cur.0]
            .as_ref()
            .expect("complete_detached: slot is empty")
            .handle
            .expect("complete_detached: task has no handle");
        // Classify the outcome. Cancellation is distinct from a normal failure: it sets
        // status `Cancelled` and joiners observe it as a *catchable* `JoinedCancelled`.
        // A failure carries the task's exception value (the parked Quoin exception, or a
        // converted internal error) so `join` can re-raise it.
        let outcome = match result {
            Ok(val) => DetachedOutcome::Done(val),
            Err(QuoinError::Cancelled) => DetachedOutcome::Cancelled,
            Err(ref e) => DetachedOutcome::Failed(match self.exceptions.active {
                Some(v) => v,
                None => self.quoinerror_to_value(mc, e),
            }),
        };
        let _ = handle.with_native_state_mut::<NativeTaskHandle, _, _>(mc, |h| match outcome {
            DetachedOutcome::Done(val) => h.set_done(val),
            DetachedOutcome::Failed(err) => h.set_failed(err),
            DetachedOutcome::Cancelled => h.set_cancelled(),
        });
        let waiters = std::mem::take(
            &mut self.sched.tasks[cur.0]
                .as_mut()
                .expect("complete_detached: slot is empty")
                .waiters,
        );
        self.sched.tasks[cur.0] = None; // free the slot; the handle keeps the outcome
        for w in waiters {
            let wake = match outcome {
                DetachedOutcome::Done(value) => Wake::Joined { value },
                DetachedOutcome::Failed(error) => Wake::Failed { error },
                DetachedOutcome::Cancelled => Wake::JoinedCancelled,
            };
            if let Some(t) = self.sched.tasks.get_mut(w.0).and_then(|t| t.as_mut()) {
                // A timed joiner: disarm its deadline so it doesn't linger / fire stale.
                if let Some(ah) = t.deadline_abort.take() {
                    ah.abort();
                }
                // No longer waiting on anything: clear the join park-state now (not
                // just at `load_task_context`), so a `cancel` landing before this
                // waiter runs cannot take the join branch and enqueue it a second time.
                t.joining = None;
                t.wake = Some(wake);
                self.sched.ready.push_back(w);
            }
        }
    }

    /// `handle.cancel`: request cancellation of detached task `target`. No-op if it is
    /// no longer running. Otherwise flag it (raised at its next checkpoint, unwinding
    /// `finally`) and nudge it toward that checkpoint promptly: abort its in-flight I/O
    /// future, or dequeue-and-wake it if it is parked in `join`.
    pub fn request_cancel(&mut self, target: TaskId) {
        let Some(t) = self.sched.tasks.get_mut(target.0).and_then(|t| t.as_mut()) else {
            return; // already finished
        };
        t.cancel_requested = true;
        if target == self.sched.current_task {
            self.sched.cancel_current = true;
            return;
        }
        if let Some(ah) = t.abort_handle.take() {
            ah.abort(); // parked on I/O: the aborted future wakes it
        } else if t.wake.is_some() {
            // Already woken and enqueued (a completion delivered its wake while this
            // task sat un-run in `ready`): it observes the cancel flag at its next
            // checkpoint. Enqueueing it again here would double-resume the slot — a
            // "task slot is empty" panic after it completes, or a spurious resume of
            // whatever task reuses the slot first.
        } else if let Some(joined) = t.joining.take() {
            // parked on (timed) join: disarm any deadline timer, dequeue from the
            // target's waiters, and make this task runnable so it sees the cancel.
            if let Some(ah) = t.deadline_abort.take() {
                ah.abort();
            }
            if let Some(jt) = self.sched.tasks.get_mut(joined.0).and_then(|t| t.as_mut()) {
                jt.waiters.retain(|w| *w != target);
            }
            self.sched.ready.push_back(target);
        } else if t.parked_on_channel {
            // Parked on a channel send/receive: make it runnable so it sees the cancel.
            // Its entry in the channel's waiter queue can't be reached from here (no `mc`),
            // so it lingers as a ghost and is skipped when a counterpart next pops it.
            t.parked_on_channel = false;
            self.sched.ready.push_back(target);
        }
        // Otherwise it is already ready (or parked on its own gather — see the v1 scope
        // note); it will see the flag when it next runs.
    }
}
