//! The cooperative async driver / scheduler loop that runs a `VmState` to completion: per-task
//! resume/complete, the `DriverFrontend` abstraction (CLI vs DAP), and the `drive_*` entry points.
//! Split out of `runner.rs` (a `#[path]` child module of `runner`, so `super::*` brings the shared
//! imports and the `ReplArena` type).

use super::*;
/// What a completed background future tells the driver to do for the task that owns it.
/// The scheduler keeps these in a `FuturesUnordered`; `.next().await` is the one reactor
/// wait. Both arms are `abortable` so `cancel` (and a won race) interrupts them promptly.
enum TaskWakeup {
    /// An async I/O op finished (`Ok`), or was aborted by `cancel` (`Err(Aborted)`).
    Io(Result<IoResult, Aborted>),
    /// A `JoinTimed` deadline timer elapsed. Carries the joined `target` and the park
    /// `epoch` captured at park time, so `deliver_deadline` can ignore a stale firing.
    Deadline { target: TaskId, epoch: u64 },
}

/// A boxed, single-threaded background future tagged with the task that is waiting on it.
type IoTaskFuture = Pin<Box<dyn Future<Output = (TaskId, TaskWakeup)>>>;

/// A tiny deterministic PRNG (SplitMix64) for `QN_SCHED_STRESS`. Seeded so a
/// randomized scheduling failure can be replayed exactly. Not used outside stress.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `0..n` (caller ensures `n > 0`).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// What the current task did when the run/test scheduler resumed it once.
enum RunStep {
    /// Suspended at a cooperative-yield boundary (between VM steps). Mailboxes are
    /// empty here, so this is the one point it is safe to *preempt* the task — the
    /// scheduler stress mode does exactly that. Without stress, it just keeps running.
    Yielded,
    /// Made progress mid-work — a block call or a guest-fiber switch — and is still
    /// the current task. Not a safe preemption point (a fiber switch leaves a value
    /// in the `fiber_transfer` mailbox the target has not consumed yet), so the
    /// driver always keeps running it.
    Running,
    /// Parked on async I/O. Its context is already saved; the driver fulfills `req`
    /// and resumes it later with the result.
    ParkedIo(IoRequest),
    /// Parked waiting on other tasks — a `gather` batch, or a `join` — which were
    /// already wired up (children/waiters enqueued, context saved) inside the resume.
    /// The driver just picks the next ready task; the wakeup comes from a completion.
    Parked,
    /// Parked in `JoinTimed` on `target` with a deadline of `ms` ms: like `Parked`, but
    /// the driver must also arm a deadline timer that wakes this task if `target` has not
    /// finished in time (`Async.timeout:do:`). The joiner is already a waiter on `target`.
    ParkedJoinTimed { target: TaskId, ms: u64 },
    /// A non-main task finished; its waker(s) were already enqueued to `ready`.
    Done,
    /// The main task (#0) finished — the program is done; its result is on the stack.
    Finished,
    /// An interactive debug session hit a breakpoint/step. The driver runs the `$`-command
    /// loop (which reads commands outside the arena and applies them inside it), then
    /// re-resumes this same task in place. Only produced when `debug.interactive` is set.
    DebugPaused,
}

/// Resume the current task's coroutine once and classify what happened. The guest
/// `Fiber` switches (`ResumeFiber`/`YieldFiber`) and the GC-cooperative yield stay
/// internal to the task; only I/O, gather, and completion surface to the driver.
fn resume_current_task<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
) -> Result<RunStep, QuoinError> {
    // Resume the coroutine of the currently-running fiber: a guest `Fiber` if one is
    // active in this task, otherwise the task's own root coroutine.
    let coro_holder = match vm.sched.current_fiber {
        None => match vm
            .sched
            .tasks
            .get(vm.sched.current_task.0)
            .and_then(|t| t.as_ref())
        {
            Some(task) => task.coro,
            None => return Ok(RunStep::Finished),
        },
        Some(fv) => fv
            .with_native_state::<fiber_class::NativeFiberState, _, _>(|s| s.coro())
            .map_err(QuoinError::Other)?,
    };

    // Point `vm.sched.yielder` at the coroutine we're about to run, sourced from its
    // own GC-rooted slot, so it never dangles.
    vm.sched.yielder = vm.current_fiber_yielder();

    let ctx = VMContext {
        vm: vm as *mut _,
        mc: mc as *const _,
    };
    let res = {
        let mut opt = coro_holder.coroutine.borrow_mut();
        let coro = opt.as_mut().expect("Coroutine already finished");
        coro.resume(ctx)
    };

    match res {
        CoroutineResult::Yield(YieldReason::CooperativeYield) => Ok(RunStep::Yielded),
        CoroutineResult::Yield(YieldReason::CallBlock { .. }) => Ok(RunStep::Running),
        CoroutineResult::Yield(YieldReason::ResumeFiber { fiber, arg }) => {
            vm.do_resume_switch(mc, fiber, arg)?;
            Ok(RunStep::Running)
        }
        CoroutineResult::Yield(YieldReason::YieldFiber { value }) => {
            vm.do_yield_switch(mc, value)?;
            Ok(RunStep::Running)
        }
        CoroutineResult::Yield(YieldReason::AwaitIo { req }) => {
            // Park: stash this task's context so another can run while I/O is in flight.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::ParkedIo(req))
        }
        CoroutineResult::Yield(YieldReason::Gather { blocks }) => {
            // Park the parent on its gather; children are enqueued inside spawn_gather.
            vm.spawn_gather(mc, blocks);
            Ok(RunStep::Parked)
        }
        CoroutineResult::Yield(YieldReason::Join { .. }) => {
            // The joiner already added itself to the target's waiter list in await_join;
            // park its context until the target completes and wakes it.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::Parked)
        }
        CoroutineResult::Yield(YieldReason::JoinTimed { task, ms }) => {
            // Like Join (the joiner is already a waiter on `task`), but the driver also
            // arms a deadline timer — carry the target and `ms` up to it.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::ParkedJoinTimed { target: task, ms })
        }
        CoroutineResult::Yield(YieldReason::ChannelPark) => {
            // The task already enqueued itself in the channel's waiter queue (in
            // `channel_send`/`channel_recv`); park its context until a counterpart or
            // `close` sets its `wake` and re-enqueues it to `ready`.
            vm.save_task_context(vm.sched.current_task);
            Ok(RunStep::Parked)
        }
        CoroutineResult::Yield(YieldReason::DebugBreak) => {
            // A breakpoint/step paused this task. Interactive sessions bubble up to the
            // driver's `$`-command loop (where the line editor lives); non-interactive ones
            // (tests / scripted runs) apply the next scripted action in place. Either way the
            // VM stays stopped — no park — and the coroutine resumes past the suspend point in
            // `debug_checkpoint` and dispatches the instruction.
            if vm
                .instrumentation
                .debug
                .as_ref()
                .is_some_and(|d| d.interactive)
            {
                Ok(RunStep::DebugPaused)
            } else {
                vm.debug_on_pause();
                Ok(RunStep::Running)
            }
        }
        CoroutineResult::Yield(YieldReason::Return(val)) => complete_current_task(vm, mc, Ok(val)),
        CoroutineResult::Return(res) => {
            if vm.sched.current_fiber.is_some() {
                // A guest fiber's block returned; hand the result back to its resumer
                // and keep running this same task.
                vm.do_fiber_done(mc, res)?;
                Ok(RunStep::Running)
            } else {
                complete_current_task(vm, mc, res)
            }
        }
    }
}

/// The current task's root coroutine completed with `result`. Dispatch by kind: a
/// gather child reports into its parent's batch; a detached task writes its outcome to
/// its handle and wakes joiners; the main task ends the program, leaving its result on
/// the stack. The first two enqueue any woken task to `ready` themselves.
fn complete_current_task<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    result: Result<Value<'gc>, QuoinError>,
) -> Result<RunStep, QuoinError> {
    let cur = vm.sched.current_task;
    let task = vm.sched.tasks[cur.0]
        .as_ref()
        .expect("completing task slot is empty");
    if task.parent.is_some() {
        vm.complete_child(mc, cur, result);
        Ok(RunStep::Done)
    } else if task.handle.is_some() {
        vm.complete_detached(mc, cur, result);
        Ok(RunStep::Done)
    } else {
        vm.sched.tasks[cur.0] = None;
        match result {
            Ok(val) => {
                vm.push(val);
                Ok(RunStep::Finished)
            }
            Err(err) => Err(err),
        }
    }
}

/// Install the already-started top-level block as scheduler task #0: wrap `run_vm_loop` in
/// a fiber, make it the sole task, and mark it current. The block must already be live on
/// `vm`'s frames (via `start_block` or `push_repl_frame`); the task is pre-started, so its
/// context lives in `VmState` until it parks. Shared by the file runner and the REPL so
/// every top-level unit runs under the scheduler.
pub(crate) fn install_main_task<'gc>(mc: &Mutation<'gc>, vm: &mut VmState<'gc>) {
    let fiber = Fiber::new(|yielder, ctx| run_vm_loop(yielder, ctx));
    // Stamp a fresh park epoch: the main task runs (and can park) before its first
    // `load_task_context`, and the REPL reinstalls task #0 every line — a fixed epoch
    // here could collide with a ghost waiter entry left in a channel that outlived
    // the previous line. `park_seq` itself survives reinstalls, so epochs stay unique.
    vm.sched.park_seq += 1;
    let epoch = vm.sched.park_seq;
    vm.sched.tasks = vec![Some(Task {
        coro: gc!(mc, fiber),
        root_yielder: None,
        block: None,
        started: true,
        stack: Vec::new(),
        frames: Vec::new(),
        native_args: Vec::new(),
        current_fiber: None,
        resume_stack: Vec::new(),
        saved_root_stack: Vec::new(),
        saved_root_frames: Vec::new(),
        saved_root_native_args: Vec::new(),
        wake: None,
        parent: None,
        gather: None,
        handle: None,
        waiters: Vec::new(),
        cancel_requested: false,
        abort_handle: None,
        joining: None,
        park_epoch: epoch,
        deadline_abort: None,
        parked_on_channel: false,
        native_reentry_depth: 0,
        aot_fuel: 0,
        aot_depth: 0,
        aot_enclosing_env: None,
        aot_home_frame_id: None,
        aot_frame_marks: Vec::new(),
        aot_nlr_target: None,
    })];
    vm.sched.current_task = TaskId(0);
}

/// Drive the scheduler until the main task (#0) — already installed via `install_main_task`
/// — finishes. Background I/O overlaps on a fresh `SmolBackend`; the single
/// `futures.next().await` is the one reactor wait. The runnable set is `vm.sched.ready` (so a
/// native `spawn` enqueues directly); `QN_SCHED_STRESS` preempts at every cooperative yield
/// and picks ready tasks at random, otherwise the scheduler is run-to-block. On a clean
/// finish the unit's result is on the VM stack (pushed by `complete_current_task`); a runtime
/// error is returned (already source-annotated by `step`). Shared by the file runner, the
/// REPL, `-e`, and `~/.quoinrc` so every top-level run gets async I/O, sleep, tasks, fibers.
/// Outcome of servicing a debug pause: resume the paused task, or stop the session.
pub(crate) enum DebugFlow {
    Resume,
    Quit,
}

/// The frontend the driver consults at debug touchpoints — the interactive CLI (`CliFrontend`)
/// or the DAP adapter. One scheduler loop ([`drive_with_frontend`]) serves both; a non-debug run
/// uses `CliFrontend` and simply never pauses.
pub(crate) trait DriverFrontend {
    /// Run once before the task starts. Return `false` to abort without running. (DAP: the
    /// `initialize`/`setBreakpoints`/`configurationDone` handshake; CLI: nothing.)
    fn configure(&mut self, _arena: &mut ReplArena) -> Result<bool, QuoinError> {
        Ok(true)
    }
    /// Called after each step to surface program output captured since the last call (DAP:
    /// `output` events). No-op when not capturing.
    fn on_output(&mut self, _arena: &mut ReplArena) -> Result<(), QuoinError> {
        Ok(())
    }
    /// A `DebugPaused`: service the frontend until the user resumes or quits.
    fn on_pause(&mut self, arena: &mut ReplArena) -> Result<DebugFlow, QuoinError>;
    /// The task finished (`err` = `None`) or escaped with an uncaught error.
    fn on_finished(
        &mut self,
        _arena: &mut ReplArena,
        _err: Option<&QuoinError>,
    ) -> Result<(), QuoinError> {
        Ok(())
    }
}

/// The interactive `$`-command frontend (`qn debug`), and the default for every non-debug run
/// (where it never pauses). The rustyline editor is built lazily on the first pause.
#[derive(Default)]
struct CliFrontend {
    editor: Option<crate::debug_cli::DebugFrontend>,
}

impl DriverFrontend for CliFrontend {
    fn on_pause(&mut self, arena: &mut ReplArena) -> Result<DebugFlow, QuoinError> {
        // The whole VM is stopped. Run the `$`-command loop: read each line with the editor
        // (outside the arena, so history persists), then execute it against the live paused VM
        // inside `mutate_root`. Loop until a continue/step verb (`Resume`) or `$quit`.
        use crate::debug_cli::{CommandOutcome, DebugFrontend, announce_pause, exec_command};
        use rustyline::error::ReadlineError;
        if self.editor.is_none() {
            self.editor = DebugFrontend::new();
        }
        let Some(editor) = self.editor.as_mut() else {
            // No usable editor — degrade to "continue" so the run still completes.
            arena.mutate_root(|_mc, vm| vm.apply_debug_action(crate::debug::DebugAction::Continue));
            return Ok(DebugFlow::Resume);
        };
        arena.mutate_root(|_mc, vm| announce_pause(vm));
        loop {
            let line = match editor.readline() {
                Ok(l) => l,
                Err(ReadlineError::Interrupted) => continue, // Ctrl-C: re-prompt
                Err(ReadlineError::Eof) => "$quit".to_string(), // Ctrl-D: quit
                Err(e) => {
                    eprintln!("debug: input error: {e}");
                    "$quit".to_string()
                }
            };
            match arena.mutate_root(|mc, vm| exec_command(vm, mc, &line)) {
                CommandOutcome::Stay => continue,
                CommandOutcome::Resume => return Ok(DebugFlow::Resume),
                CommandOutcome::Quit => return Ok(DebugFlow::Quit),
            }
        }
    }
}

/// The interactive/normal driver: a [`CliFrontend`] over the shared scheduler loop. Used by
/// `qn debug`, plain `qn <file>`, the REPL, and the debug fixtures.
/// `QN_AOT_STATS=1`: dump the speculative-AOT observation summary after the
/// main task finishes (S0 has no other observable surface).
fn maybe_print_spec_stats(arena: &mut ReplArena) {
    if std::env::var("QN_AOT_STATS").map(|v| v != "0") != Ok(true) {
        return;
    }
    arena.mutate_root(|_mc, vm| {
        eprintln!("{}", vm.aot_spec_stats());
    });
}

pub(crate) fn drive_main_task(arena: &mut ReplArena) -> Result<(), QuoinError> {
    let result = drive_with_frontend(arena, &mut CliFrontend::default());
    maybe_print_spec_stats(arena);
    result
}

/// The cooperative scheduler loop, parameterized by a [`DriverFrontend`] for the debug
/// touchpoints (configuration, program output, pause, completion). Resumes the current task,
/// services background I/O / deadlines via the reactor, and hands a `DebugPaused` to the
/// frontend. Shared by the CLI debugger, normal/REPL runs, and the DAP adapter.
pub(crate) fn drive_with_frontend<F: DriverFrontend>(
    arena: &mut ReplArena,
    frontend: &mut F,
) -> Result<(), QuoinError> {
    // The session's persistent I/O backend (an `Rc` to the shared `StreamId -> fd` registry), not a
    // fresh one — so a long-lived resource opened on a previous driver run survives. This matters
    // for the REPL, which drives each line through its own `drive_with_frontend`: an extension
    // socket spawned on one line, or a file/connection opened on it, must still be reachable on the
    // next. (A single file/`-e` run drives once, so this is equivalent to a fresh backend there.)
    let backend = arena.mutate_root(|_mc, vm| vm.io.backend.clone());
    let mut futures: FuturesUnordered<IoTaskFuture> = FuturesUnordered::new();
    let mut rng = crate::tuning::sched_stress().map(SplitMix64::new);
    // Announce the seed once per process so a failing run is reproducible with the same
    // `QN_SCHED_STRESS=<seed>`.
    if let Some(seed) = crate::tuning::sched_stress() {
        static ANNOUNCED: Once = Once::new();
        ANNOUNCED.call_once(|| eprintln!("scheduler stress enabled (seed={seed})"));
    }
    // Task #0 starts current and already live; nothing to load on first resume.
    let mut current: Option<TaskId> = Some(TaskId(0));
    let mut needs_load = false;

    block_on(async {
        // Pre-run configuration (the DAP handshake + breakpoints; a no-op for the CLI). Abort
        // cleanly if the frontend declines to run.
        if !frontend.configure(arena)? {
            return Ok(());
        }
        let mut step_count = 0;
        loop {
            // Acquire a task to run after the previous one parked or finished: pick from
            // `ready` (random under stress); if none are ready but I/O is in flight, await a
            // completion, which feeds `ready`, and retry.
            if current.is_none() {
                let picked = arena.mutate_root(|_mc, vm| {
                    let n = vm.sched.ready.len();
                    if n == 0 {
                        None
                    } else {
                        let idx = rng.as_mut().map(|r| r.below(n)).unwrap_or(0);
                        Some(vm.sched.ready.remove(idx).expect("idx within ready"))
                    }
                });
                match picked {
                    Some(tid) => {
                        current = Some(tid);
                        needs_load = true;
                    }
                    None => {
                        if futures.is_empty() {
                            // Nothing ready and nothing in flight. A finished main
                            // task already broke out via `RunStep::Finished`, so if
                            // its slot is still occupied it is parked — on a channel,
                            // a join, or a gather that can never complete: a global
                            // deadlock. Surface it as an error; the old silent
                            // `break` exited 0 with the rest of the program
                            // unexecuted, indistinguishable from success.
                            let main_parked = arena.mutate_root(|_mc, vm| {
                                vm.sched.tasks.first().is_some_and(|t| t.is_some())
                            });
                            if main_parked {
                                let e = QuoinError::Other(
                                    "deadlock: every task is parked with no I/O in \
                                     flight (e.g. a receive with no sender, or a join \
                                     cycle); the program cannot make progress"
                                        .to_string(),
                                );
                                frontend.on_finished(arena, Some(&e))?;
                                return Err(e);
                            }
                            break; // nothing ready and nothing in flight
                        }
                        // About to go idle: flush pending fd closes FIRST. A parked peer may
                        // be waiting on exactly one of these closes (its read's EOF) for the
                        // wakeup we are about to sleep for to ever arrive — leaving the reap
                        // to the periodic (`step_count % 10`) drain below would deadlock an
                        // idle scheduler whose only in-flight I/O waits on a closed-but-
                        // unreaped fd.
                        let reaped: Vec<StreamId> = arena.mutate_root(|_mc, vm| {
                            vm.io.socket_reap.borrow_mut().drain(..).collect()
                        });
                        for id in reaped {
                            backend.close(id);
                        }
                        // The single reactor wait: park until some background future (I/O op
                        // or deadline timer) lands.
                        let (tid, wakeup) = futures.next().await.expect("futures is non-empty");
                        arena.mutate_root(|_mc, vm| match wakeup {
                            TaskWakeup::Io(result) => {
                                {
                                    let t = vm.sched.tasks[tid.0]
                                        .as_mut()
                                        .expect("woken task slot is empty");
                                    t.abort_handle = None; // the future is done
                                    // On `Err(Aborted)` the task was cancelled: leave `wake`
                                    // unset — `await_io` raises `Cancelled` instead.
                                    if let Ok(io_result) = result {
                                        t.wake = Some(Wake::Io { result: io_result });
                                    }
                                }
                                vm.sched.ready.push_back(tid);
                            }
                            // A deadline elapsed: `deliver_deadline` resolves the race and
                            // enqueues the joiner if it won.
                            TaskWakeup::Deadline { target, epoch } => {
                                vm.deliver_deadline(tid, target, epoch);
                            }
                        });
                        continue;
                    }
                }
            }
            let cur = current.expect("current task set above");
            if needs_load {
                arena.mutate_root(|mc, vm| vm.load_task_context(mc, cur));
                needs_load = false;
            }

            let step = arena.mutate_root(|mc, vm| resume_current_task(vm, mc));
            // Surface any program output this step produced before reacting to the step.
            frontend.on_output(arena)?;
            match step {
                Ok(RunStep::Yielded) => {
                    // A clean cooperative-yield boundary. Under stress, preempt: stash and
                    // requeue so the save/load round-trip runs every step and ordering varies.
                    if rng.is_some() {
                        arena.mutate_root(|_mc, vm| {
                            vm.save_task_context(cur);
                            vm.sched.ready.push_back(cur);
                        });
                        current = None;
                    }
                }
                Ok(RunStep::Running) => {}
                Ok(RunStep::ParkedIo(req)) => {
                    // Hand the op to the backend; the future is tagged with the parked task so
                    // its result routes back, and wrapped in `abortable` so `cancel` can
                    // interrupt it. Stash the abort handle for `request_cancel`.
                    let (fut, abort_handle) = abortable(backend.perform(req));
                    arena.mutate_root(|_mc, vm| {
                        vm.sched.tasks[cur.0]
                            .as_mut()
                            .expect("parked task slot is empty")
                            .abort_handle = Some(abort_handle);
                    });
                    futures.push(Box::pin(async move { (cur, TaskWakeup::Io(fut.await)) }));
                    current = None;
                }
                Ok(RunStep::ParkedJoinTimed { target, ms }) => {
                    // Arm the deadline alongside the join: a `Sleep` timer tagged with this
                    // joiner + the park epoch, wrapped in `abortable` so a normal completion /
                    // cancel can disarm it. `deliver_deadline` ignores a stale firing.
                    let (fut, abort_handle) = abortable(backend.perform(IoRequest::Sleep { ms }));
                    let epoch = arena.mutate_root(|_mc, vm| {
                        let t = vm.sched.tasks[cur.0]
                            .as_mut()
                            .expect("timed-join parked task slot is empty");
                        t.deadline_abort = Some(abort_handle);
                        t.park_epoch
                    });
                    futures.push(Box::pin(async move {
                        let _ = fut.await; // resolved (Slept) or aborted; either way
                        (cur, TaskWakeup::Deadline { target, epoch })
                    }));
                    current = None;
                }
                // Parked on a gather batch or a join, or finished: any task that became
                // runnable was already enqueued to `ready` in the resume.
                Ok(RunStep::Parked) | Ok(RunStep::Done) => {
                    current = None;
                }
                Ok(RunStep::Finished) => {
                    frontend.on_finished(arena, None)?;
                    break;
                }
                Ok(RunStep::DebugPaused) => match frontend.on_pause(arena)? {
                    // Re-resume the same task: its context is live (it parked nothing).
                    DebugFlow::Resume => {}
                    DebugFlow::Quit => return Ok(()),
                },
                Err(e) => {
                    frontend.on_finished(arena, Some(&e))?;
                    return Err(e);
                }
            }
            step_count += 1;
            if crate::tuning::gc_stress() || step_count % 10 == 0 {
                arena.collect_debt();
                // Reap fds whose handle was closed or collected — both enqueue on
                // `socket_reap`; close them now, outside the arena borrow.
                let reaped: Vec<StreamId> =
                    arena.mutate_root(|_mc, vm| vm.io.socket_reap.borrow_mut().drain(..).collect());
                for id in reaped {
                    backend.close(id);
                }
                // Bulk-release the host-value handles of any dropped extension (its `Drop`
                // enqueued its `ext_id`), so they stop rooting host Values.
                arena.mutate_root(|_mc, vm| {
                    let ext_ids: Vec<u64> = vm.io.ext_handle_reap.borrow_mut().drain(..).collect();
                    for ext_id in ext_ids {
                        vm.handle_table.release_for_ext(ext_id);
                    }
                });
            }
        }
        Ok(())
    })
}
