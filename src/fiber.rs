use crate::error::QuoinError;
use crate::io_backend::IoRequest;
use crate::value::{Block, Value};
use crate::vm::{TaskId, VmState, VmStatus};

use corosensei::stack::DefaultStack;
use gc_arena::{Collect, Gc, Mutation};
use std::cell::RefCell;

pub type VMCoroutine<'gc> = corosensei::ScopedCoroutine<
    'gc,
    VMContext<'gc>,
    YieldReason<'gc>,
    Result<Value<'gc>, QuoinError>,
    DefaultStack,
>;
pub type VMYielder<'gc> = corosensei::Yielder<VMContext<'gc>, YieldReason<'gc>>;

#[derive(Collect)]
#[collect(no_drop)]
pub enum YieldReason<'gc> {
    CallBlock {
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
    },
    CooperativeYield,
    Return(Value<'gc>),
    /// A guest fiber is resuming another guest fiber. Bubbles to the scheduler,
    /// which switches execution contexts to `fiber`, delivering `arg`.
    ResumeFiber {
        fiber: Value<'gc>,
        arg: Value<'gc>,
    },
    /// The running guest fiber is suspending and handing `value` back to whoever
    /// resumed it. Bubbles to the scheduler.
    YieldFiber {
        value: Value<'gc>,
    },
    /// A fiber is suspending to perform async I/O. The plain-data request bubbles to
    /// the scheduler, which fulfills it via the `IoBackend` and resumes the fiber with
    /// the result in `Scheduler::wake`. See `docs/ASYNC_ARCH.md`.
    AwaitIo {
        #[collect(require_static)]
        req: IoRequest,
    },
    /// The running task is spawning one child task per block and parking until all
    /// of them complete (`Async.gather:`). The blocks carry `Gc` just like
    /// `CallBlock`; the scheduler spawns the children and resumes the parent with the
    /// list of results in `Scheduler::wake`. See `docs/ASYNC_ARCH.md` (Stage 2a).
    Gather {
        blocks: Vec<Gc<'gc, Block<'gc>>>,
    },
    /// The running task is parking in `join` on another (detached) task. The plain
    /// `TaskId` bubbles to the scheduler; the joiner was already added to the target's
    /// waiter list, and is resumed with the outcome in `Scheduler::wake` when the
    /// target completes. See `docs/ASYNC_ARCH.md` (Stage 2b).
    Join {
        #[collect(require_static)]
        task: TaskId,
    },
    /// Like `Join`, but with a deadline: the running task parks on `task` *or* a timer
    /// of `ms` milliseconds, whichever fires first (`Async.timeout:do:`). The scheduler
    /// arms a deadline timer alongside the join; the first to resolve wins and the loser
    /// is disarmed. Resumes with the join outcome, or `Wake::TimedOut` on the deadline.
    /// See `docs/ASYNC_ARCH.md` (Stage 5a).
    JoinTimed {
        #[collect(require_static)]
        task: TaskId,
        #[collect(require_static)]
        ms: u64,
    },
    /// The running task is parking on a `Channel` send/receive rendezvous. Pure in-VM
    /// coordination (no I/O backend): the task already registered itself in the channel's
    /// waiter queue, so this carries no payload — the driver just parks its context. A
    /// counterpart (or `close`) sets this task's `wake` and enqueues it to `ready`. See
    /// `src/runtime/channel.rs`.
    ChannelPark,
    /// A debug session paused execution (a breakpoint hit, or a single-step landed). Carries
    /// no payload — the driver reads the paused state directly off `VmState`, runs the
    /// command loop, and resumes the task in place (no parking, so the VM stays stopped).
    /// Suspended deep in the step loop, so it bubbles straight to the driver like `AwaitIo`.
    /// See `src/debug.rs`.
    DebugBreak,
}

/// The standard VM driver loop, shared by the main program and every guest
/// `Fiber`. Each runs as its own `corosensei` coroutine; this body just steps
/// the VM over the *current* execution context and cooperatively suspends so
/// the scheduler can run the GC. Fiber resume/yield happen deeper in `step`
/// (inside the native `Fiber` methods) and bubble up as `YieldReason`s, so they
/// are transparent here.
pub fn run_vm_loop<'gc>(
    yielder: &VMYielder<'gc>,
    mut ctx: VMContext<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    // Record this coroutine's yielder in its fiber's slot (and make it live).
    // From here on the driver restores `vm.yielder` from that slot before every
    // resume, so it can never be left pointing at a different/freed coroutine.
    let yptr = yielder as *const _ as *const ();
    {
        let (vm, mc) = unsafe { ctx.get() };
        vm.register_yielder(mc, yptr);
    }

    loop {
        let (vm, mc) = unsafe { ctx.get() };
        match vm.step(mc) {
            Ok(VmStatus::Running) => {
                ctx = yielder.suspend(YieldReason::CooperativeYield);
            }
            Ok(VmStatus::Finished(val)) => return Ok(val),
            Ok(VmStatus::Yeeted(val)) => {
                return Err(QuoinError::Other(format!("Uncaught exception: {}", val)));
            }
            Err(err) => return Err(err),
        }
    }
}

/// A wrapper around raw pointers to VMState and Mutation contexts.
/// This allows passing execution contexts into and out of coroutines
/// without lifetime conflicts.
pub struct VMContext<'gc> {
    pub vm: *mut VmState<'gc>,
    pub mc: *const Mutation<'gc>,
}

impl<'gc> VMContext<'gc> {
    /// # Safety
    /// The caller must ensure that the pointers are valid and that no other
    /// borrows of the VM state or mutation context exist during the call.
    pub unsafe fn get(&self) -> (&mut VmState<'gc>, &Mutation<'gc>) {
        unsafe { (&mut *self.vm, &*self.mc) }
    }
}

pub struct Fiber<'gc> {
    pub coroutine: RefCell<Option<VMCoroutine<'gc>>>,
}

unsafe impl<'gc> Collect<'gc> for Fiber<'gc> {
    const NEEDS_TRACE: bool = false;
}

impl<'gc> Fiber<'gc> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnOnce(&VMYielder<'gc>, VMContext<'gc>) -> Result<Value<'gc>, QuoinError> + 'gc,
    {
        let coroutine = VMCoroutine::new(move |yielder, ctx| f(yielder, ctx));
        Self {
            coroutine: RefCell::new(Some(coroutine)),
        }
    }
}
