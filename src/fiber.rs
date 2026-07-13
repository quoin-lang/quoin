use crate::error::QuoinError;
use crate::io_backend::IoRequest;
use crate::value::{Block, Value};
use crate::vm::{TaskId, VmState, VmStatus};

use corosensei::stack::{DefaultStack, Stack};
use gc_arena::{Collect, Gc, Mutation};
use std::cell::{Cell, RefCell};

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
    /// the result in `Scheduler::wake`. See `docs/internal/ASYNC_ARCH.md`.
    AwaitIo {
        #[collect(require_static)]
        req: IoRequest,
    },
    /// The running task is spawning one child task per block and parking until all
    /// of them complete (`Async.gather:`). The blocks carry `Gc` just like
    /// `CallBlock`; the scheduler spawns the children and resumes the parent with the
    /// list of results in `Scheduler::wake`. See `docs/internal/ASYNC_ARCH.md` (Stage 2a).
    Gather {
        blocks: Vec<Gc<'gc, Block<'gc>>>,
    },
    /// The running task is parking in `join` on another (detached) task. The plain
    /// `TaskId` bubbles to the scheduler; the joiner was already added to the target's
    /// waiter list, and is resumed with the outcome in `Scheduler::wake` when the
    /// target completes. See `docs/internal/ASYNC_ARCH.md` (Stage 2b).
    Join {
        #[collect(require_static)]
        task: TaskId,
    },
    /// Like `Join`, but with a deadline: the running task parks on `task` *or* a timer
    /// of `ms` milliseconds, whichever fires first (`Async.timeout:do:`). The scheduler
    /// arms a deadline timer alongside the join; the first to resolve wins and the loser
    /// is disarmed. Resumes with the join outcome, or `Wake::TimedOut` on the deadline.
    /// See `docs/internal/ASYNC_ARCH.md` (Stage 5a).
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

    // Batch instructions per cooperative yield (Slice 2g), so the coroutine switch + driver
    // round-trip + GC pacing amortize over many steps instead of being paid every step.
    let batch: u32 = crate::tuning::step_batch();
    let stats = crate::tuning::batch_stats();
    loop {
        // Per-batch tuning stats (QN_BATCH_STATS): wall time + GC bytes allocated over the
        // batch (no collection runs mid-batch, so the live-allocation delta = bytes allocated).
        let mark = if stats {
            let (_, mc) = unsafe { ctx.get() };
            Some((
                std::time::Instant::now(),
                mc.metrics().total_gc_allocation(),
            ))
        } else {
            None
        };
        {
            let (vm, mc) = unsafe { ctx.get() };
            match vm.run_dispatch(mc, batch) {
                // Budget spent — fall through to the cooperative yield below.
                Ok(VmStatus::Running) => {}
                Ok(VmStatus::Finished(val)) => {
                    if stats {
                        report_batch_stats(batch);
                    }
                    return Ok(val);
                }
                Ok(VmStatus::Yeeted(val)) => {
                    if stats {
                        report_batch_stats(batch);
                    }
                    return Err(QuoinError::Other(format!("Uncaught exception: {}", val)));
                }
                Err(err) => {
                    // Break-on-throw: an exception is about to escape the task uncaught — its
                    // frames are still live (propagation doesn't pop them), so pause here if a
                    // debug session is watching its type, then let it propagate (task ends).
                    if vm.has_break_on_throw() {
                        vm.debug_check_throw(mc, &err);
                    }
                    if stats {
                        report_batch_stats(batch);
                    }
                    return Err(err);
                }
            }
        }
        if let Some((t0, a0)) = mark {
            let (_, mc) = unsafe { ctx.get() };
            let dt = t0.elapsed().as_nanos();
            let db = mc.metrics().total_gc_allocation().saturating_sub(a0) as u128;
            BATCH_ACC.with(|c| {
                let (n, sn, sb) = c.get();
                c.set((n + 1, sn + dt, sb + db));
            });
        }
        ctx = yielder.suspend(YieldReason::CooperativeYield);
    }
}

thread_local! {
    /// (full batches, sum wall-ns, sum GC bytes allocated) for the `QN_BATCH_STATS` harness.
    static BATCH_ACC: std::cell::Cell<(u64, u128, u128)> = const { std::cell::Cell::new((0, 0, 0)) };
}

/// Emit the accumulated per-batch stats to stderr (the `QN_BATCH_STATS` harness). One
/// full batch = exactly `batch` instructions, so total steps = batches * batch.
fn report_batch_stats(batch: u32) {
    let (n, sum_ns, sum_bytes) = BATCH_ACC.with(|c| c.get());
    if n == 0 {
        return;
    }
    let steps = n as u128 * batch as u128;
    eprintln!(
        "[batch-stats] batch={:>7} batches={:>9} time/batch={:>9.3}us alloc/batch={:>9}B per_instr={:>6.3}ns",
        batch,
        n,
        sum_ns as f64 / n as f64 / 1000.0,
        sum_bytes / n as u128,
        sum_ns as f64 / steps as f64,
    );
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
    // Returning `&mut` from `&self` is the deliberate purpose of this raw-pointer
    // wrapper: it hands the VM state across the coroutine boundary without a
    // lifetime tying it to the wrapper. Safety is the caller's contract above.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> (&mut VmState<'gc>, &Mutation<'gc>) {
        unsafe { (&mut *self.vm, &*self.mc) }
    }
}

pub struct Fiber<'gc> {
    pub coroutine: RefCell<Option<VMCoroutine<'gc>>>,
    /// Sticky: this fiber has entered AOT-compiled code at least once, so its
    /// suspended stack may hold Cranelift frames — which corosensei's forced
    /// unwind cannot cross (compiled code carries no unwind tables; the walk
    /// aborts the process). Set by `codegen`'s entry gate, read by `Drop`.
    pub ran_compiled: Cell<bool>,
    /// Lowest usable address of this coroutine's stack (stacks grow *down*), captured at
    /// construction. The driver copies it into `VmState::stack_limit` before resuming this
    /// coroutine, which is how `execute_block` knows how much room is left — see
    /// `VmState::ensure_stack_headroom`.
    pub stack_limit: usize,
}

/// Teardown for an abandoned fiber (a generator dropped mid-iteration by
/// `take:`, or still suspended at program exit). Corosensei's own `Drop`
/// force-unwinds a suspended coroutine with a panic that walks the fiber's
/// stack — fine for pure-interpreted fibers (their Rust frames unwind
/// normally, exactly as before), fatal across Cranelift frames. If this
/// fiber ever entered compiled code, LEAK the suspended stack instead:
/// `force_reset` marks the coroutine complete, so its `Drop` frees the
/// stack mapping without the unwind walk.
///
/// Safety of the leak: by the pinned suspension invariants, nothing on a
/// suspended coroutine stack owns a `Gc` value (tests/gc_across_yield.rs)
/// or holds a `RefCell` borrow (tests/borrow_across_yield.rs) — the guest
/// context lives in the swapped `stack`/`frames`/`aot` slices, which the
/// fiber's native state owns and drops normally. What leaks is the
/// Rust-frame residue of one suspended resume chain (interpreter locals'
/// heap buffers), bounded per abandoned fiber — the price of admitting
/// compiled frames into fibers at all.
impl<'gc> Drop for Fiber<'gc> {
    fn drop(&mut self) {
        if !self.ran_compiled.get() {
            return; // pre-existing behavior: corosensei's forced unwind
        }
        if let Ok(mut slot) = self.coroutine.try_borrow_mut()
            && let Some(coro) = slot.as_mut()
            && coro.started()
            && !coro.done()
        {
            // SAFETY: leaks the suspended stack instead of unwinding
            // it; the invariants above guarantee nothing on it
            // requires Drop for soundness.
            unsafe { coro.force_reset() };
        }
    }
}

unsafe impl<'gc> Collect<'gc> for Fiber<'gc> {
    const NEEDS_TRACE: bool = false;
}

impl<'gc> Fiber<'gc> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnOnce(&VMYielder<'gc>, VMContext<'gc>) -> Result<Value<'gc>, QuoinError> + 'gc,
    {
        // 16 MiB virtual (lazily committed): compiled<->interpreted
        // alternations nest real Rust frames per level (bounded by
        // spec::MAX_OUTCALL_NESTING), and `dispatch_one`'s frame is large —
        // the corosensei default (1 MiB) overflowed under S1 promotion.
        let stack = DefaultStack::new(16 * 1024 * 1024).expect("coroutine stack");
        // Read the limit before `with_stack` consumes the stack: it is the only handle we
        // get on this coroutine's extent, and `execute_block` needs it to bound re-entry.
        let stack_limit = stack.limit().get();
        let coroutine = VMCoroutine::with_stack(stack, move |yielder, ctx| f(yielder, ctx));
        Self {
            coroutine: RefCell::new(Some(coroutine)),
            ran_compiled: Cell::new(false),
            stack_limit,
        }
    }
}
