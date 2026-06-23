use crate::compiler::Compiler;
use crate::error::QuoinError;
use crate::fiber::{Fiber, VMContext, YieldReason, run_vm_loop};
use crate::gc;
use crate::highlighter::highlight_to_ansi;
use crate::io_backend::{IoBackend, IoRequest, IoResult, SmolBackend, StreamId};
use crate::parser::ast::Node;
use crate::parser::{NodeValue, parse_quoin_file};
use crate::runtime::{
    async_rt, block, boolean, bytes, class, double, fiber as fiber_class, http, integer, io, list,
    map, method, nil, object, regex, runtime, set, sockets, string, symbol, task, timer,
};
use crate::value::{Block, NamespacedName, Value};
use crate::vm::{Task, TaskId, VmOptions, VmState, VmStatus, Wake};

use corosensei::CoroutineResult;
use futures_lite::StreamExt;
use futures_lite::future::block_on;
use futures_util::future::{Aborted, abortable};
use futures_util::stream::FuturesUnordered;
use gc_arena::{Arena, Gc, Mutation, Rootable};
use std::fs::read_to_string;
use std::future::Future;
use std::iter::once_with;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::exit;
use std::sync::Once;
use std::time::Instant;

/// The prelude AST: a single `qnlib/prelude.qn` whose `use core/*` loads the core
/// stdlib (00-bootstrap … 06-io) in sorted order. Every runner mode loads this first,
/// so the prelude composition lives in Quoin rather than a hardcoded glob here.
fn prelude_asts() -> impl Iterator<Item = Node> {
    once_with(|| parse_quoin_file(&PathBuf::from("qnlib/prelude.qn")))
}

/// Step status for the benchmark driver, which runs a single fiber to completion
/// with no async I/O (the run/test driver uses `RunStep` and the task scheduler).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionStatus {
    Running,
    Finished,
    Yeeted,
}

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

pub struct VmRunnerOptions {
    pub mode: VmRunnerMode,
    pub target_path: Option<String>,
    pub vm_options: VmOptions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmRunnerMode {
    Highlight,
    Test,
    Benchmark,
    Run,
}

impl VmRunnerOptions {
    pub fn parse(args: &[String]) -> Self {
        let mut mode = VmRunnerMode::Run;
        let mut target_path = None;
        let mut vm_args = Vec::new();

        if let Some(arg) = args.get(1) {
            if arg == "highlight" {
                mode = VmRunnerMode::Highlight;
                target_path = args.get(2).cloned();
                if args.len() > 3 {
                    vm_args = args[3..].to_vec();
                }
            } else if arg == "test" {
                mode = VmRunnerMode::Test;
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
            } else if arg == "benchmark" {
                mode = VmRunnerMode::Benchmark;
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
            } else {
                mode = VmRunnerMode::Run;
                target_path = Some(arg.clone());
                if args.len() > 2 {
                    vm_args = args[2..].to_vec();
                }
            }
        }

        Self {
            mode,
            target_path,
            vm_options: VmOptions {
                arguments: vm_args,
                supports_color: false,
                console_width: None,
            },
        }
    }
}

pub struct VmRunner {
    options: VmRunnerOptions,
}

impl VmRunner {
    pub fn new(options: VmRunnerOptions) -> Self {
        Self { options }
    }

    pub fn run(&self) -> Result<(), QuoinError> {
        match self.options.mode {
            VmRunnerMode::Highlight => {
                let Some(ref path) = self.options.target_path else {
                    eprintln!("Usage: cargo run -- highlight FILE");
                    exit(2);
                };
                let source = match read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error reading {}: {}", path, e);
                        exit(1);
                    }
                };
                print!("{}", highlight_to_ansi(&source));
                Ok(())
            }
            VmRunnerMode::Test => {
                // prelude, then the test entry — main.qn `use`s the framework + suites.
                let ast_iter = prelude_asts().chain(once_with(|| {
                    parse_quoin_file(&PathBuf::from("qnlib/main.qn"))
                }));

                if !self.compile_and_run_asts(ast_iter) {
                    exit(1);
                }
                Ok(())
            }
            VmRunnerMode::Benchmark => {
                let ast_iter = prelude_asts().chain(once_with(|| {
                    parse_quoin_file(&PathBuf::from("qnlib/benchmark.qn"))
                }));

                self.compile_and_benchmark(ast_iter);
                Ok(())
            }
            VmRunnerMode::Run => {
                let script_path = self
                    .options
                    .target_path
                    .clone()
                    .unwrap_or_else(|| "qnlib/testscript.qn".to_string());
                let ast_iter = prelude_asts().chain(once_with(move || {
                    parse_quoin_file(&PathBuf::from(&script_path))
                }));

                self.compile_and_run_asts(ast_iter);
                Ok(())
            }
        }
    }

    /// Runs each program AST in turn. Returns `true` if the run completed without a
    /// VM error and the last program's result value was truthy. For `qn test` that
    /// last value is main.qn's `results.none?:{…}` boolean (true iff every suite
    /// passed), so the Test driver can gate the process exit code on it.
    fn compile_and_run_asts(&self, ast_iter: impl Iterator<Item = Node>) -> bool {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());

            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, boolean::build_boolean_class());
            vm.register_native_class(mc, block::build_block_class());
            vm.register_native_class(mc, bytes::build_bytes_class());
            vm.register_native_class(mc, sockets::build_tcp_socket_class());
            vm.register_native_class(mc, sockets::build_tls_socket_class());
            vm.register_native_class(mc, http::build_http_parser_class());
            vm.register_native_class(mc, io::build_io_folder_class());
            vm.register_native_class(mc, io::build_io_file_class());
            vm.register_native_class(mc, io::build_io_handle_class());
            vm.register_native_class(mc, list::build_list_class());
            vm.register_native_class(mc, set::build_set_class());
            vm.register_native_class(mc, runtime::build_runtime_class());
            vm.register_native_class(mc, async_rt::build_async_class());
            vm.register_native_class(mc, task::build_task_class());
            vm.register_native_class(mc, method::build_method_class());
            vm.register_native_class(mc, timer::build_timer_class());
            vm.register_native_class(mc, double::build_double_class());
            vm.register_native_class(mc, integer::build_integer_class());
            vm.register_native_class(mc, string::build_string_class());
            vm.register_native_class(mc, symbol::build_symbol_class());
            vm.register_native_class(mc, nil::build_nil_class());
            vm.register_native_class(mc, map::build_map_class());
            vm.register_native_class(mc, map::build_key_value_pair_class());
            vm.register_native_class(mc, regex::build_regex_class());
            vm.register_native_class(mc, fiber_class::build_fiber_class());

            vm
        });

        let mut aborted = false;
        for ast in ast_iter {
            if aborted {
                break;
            }

            arena.mutate_root(|mc, vm| {
                let program_node = match &ast.value {
                    NodeValue::Program(p) => p,
                    _ => {
                        panic!("Error: Root AST node is not a ProgramNode");
                    }
                };

                let mut compiler = Compiler::new();
                let program = match compiler.compile_program(program_node) {
                    Ok(p) => p,
                    Err(e) => {
                        panic!("Compilation error: {}", e);
                    }
                };

                let decl_block = program.decl_block.as_ref().map(|db| {
                    gc!(
                        mc,
                        Block {
                            name: db.name.clone(),
                            is_nested_block: db.is_nested_block,
                            param_syms: db.param_syms.clone(),
                            param_types: db.param_types.clone(),
                            bytecode: db.bytecode.clone(),
                            parent_env: None,
                            enclosing_method_id: None,
                            source_info: db.source_info.clone(),
                            decl_block: None,
                            source_map: db.source_map.clone(),
                        }
                    )
                });
                let main_block = gc!(
                    mc,
                    Block {
                        name: program.name.clone(),
                        is_nested_block: program.is_nested_block,
                        param_syms: program.param_syms.clone(),
                        param_types: program.param_types.clone(),
                        bytecode: program.bytecode.clone(),
                        parent_env: None,
                        enclosing_method_id: None,
                        source_info: program.source_info.clone(),
                        decl_block,
                        source_map: program.source_map.clone(),
                    }
                );
                vm.start_block(mc, main_block, Vec::new(), None, None);

                // The main program runs as task #0; the run/test driver schedules
                // over the task table (benchmark mode uses `active_fiber` instead).
                // Its block is already started into the live context above, so this
                // task is pre-started and its context lives in `VmState` until it parks.
                let fiber = Fiber::new(|yielder, ctx| run_vm_loop(yielder, ctx));
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
                    wake: None,
                    parent: None,
                    gather: None,
                    handle: None,
                    waiters: Vec::new(),
                    cancel_requested: false,
                    abort_handle: None,
                    joining: None,
                    park_epoch: 0,
                    deadline_abort: None,
                })];
                vm.sched.current_task = TaskId(0);
            });

            // The scheduler: run tasks until each parks on I/O or finishes, overlapping
            // their I/O via a `FuturesUnordered`. The runnable set is `vm.sched.ready`
            // (so a native `spawn` can enqueue directly); the backend lives outside the
            // arena and is the only async code; the await on `futures` is the single
            // reactor wait. `QN_SCHED_STRESS` turns on a seeded PRNG that preempts at
            // every cooperative yield and picks ready tasks at random — exercising the
            // per-task state swap and a wide range of interleavings. Without it the
            // scheduler is run-to-block.
            let backend = SmolBackend::new();
            let mut futures: FuturesUnordered<IoTaskFuture> = FuturesUnordered::new();
            let mut rng = crate::tuning::sched_stress().map(SplitMix64::new);
            // Announce the seed once per process so a failing run is reproducible with
            // the same `QN_SCHED_STRESS=<seed>` (this driver runs once per program AST).
            if let Some(seed) = crate::tuning::sched_stress() {
                static ANNOUNCED: Once = Once::new();
                ANNOUNCED.call_once(|| eprintln!("scheduler stress enabled (seed={seed})"));
            }
            // Task #0 starts current and already live; nothing to load on first resume.
            let mut current: Option<TaskId> = Some(TaskId(0));
            let mut needs_load = false;

            block_on(async {
                let mut step_count = 0;
                loop {
                    // Acquire a task to run after the previous one parked or finished:
                    // pick from `ready` (random under stress); if none are ready but I/O
                    // is in flight, await a completion, which feeds `ready`, and retry.
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
                                    break; // nothing ready and nothing in flight
                                }
                                // The single reactor wait: park until some background
                                // future (I/O op or deadline timer) lands.
                                let (tid, wakeup) =
                                    futures.next().await.expect("futures is non-empty");
                                arena.mutate_root(|_mc, vm| match wakeup {
                                    TaskWakeup::Io(result) => {
                                        {
                                            let t = vm.sched.tasks[tid.0]
                                                .as_mut()
                                                .expect("woken task slot is empty");
                                            t.abort_handle = None; // the future is done
                                            // On `Err(Aborted)` the task was cancelled:
                                            // leave `wake` unset — `await_io` raises
                                            // `Cancelled` from `cancel_current` instead.
                                            if let Ok(io_result) = result {
                                                t.wake = Some(Wake::Io { result: io_result });
                                            }
                                        }
                                        vm.sched.ready.push_back(tid);
                                    }
                                    // A deadline elapsed: `deliver_deadline` resolves the
                                    // race (wakes the joiner with `TimedOut`, or ignores a
                                    // stale/superseded firing) and enqueues it if it won.
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
                    match step {
                        Ok(RunStep::Yielded) => {
                            // A clean cooperative-yield boundary. Under scheduler stress,
                            // preempt: stash this task and requeue it, so the save/load
                            // round-trip runs every step and task ordering varies. Without
                            // stress, keep running it (run-to-block).
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
                            // Hand the op to the backend; the future is tagged with the
                            // parked task so its result routes back on completion, and
                            // wrapped in `abortable` so `cancel` can interrupt it. Stash
                            // the abort handle on the task for `request_cancel`.
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
                            // Arm the deadline alongside the join: a `Sleep` timer tagged
                            // with this joiner + the park epoch, wrapped in `abortable` so a
                            // normal completion / cancel can disarm it. The first of {target
                            // completes, deadline fires} wins; `deliver_deadline` ignores a
                            // stale firing via the epoch. (See the `JoinTimed` race notes.)
                            let (fut, abort_handle) =
                                abortable(backend.perform(IoRequest::Sleep { ms }));
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
                        // Parked on a gather batch or a join, or finished: any task that
                        // became runnable was already enqueued to `ready` in the resume.
                        Ok(RunStep::Parked) | Ok(RunStep::Done) => {
                            current = None;
                        }
                        Ok(RunStep::Finished) => break,
                        Err(e) => {
                            eprintln!("VM execution error: {}", e);
                            aborted = true;
                            break;
                        }
                    }
                    step_count += 1;
                    if crate::tuning::gc_stress() || step_count % 10 == 0 {
                        arena.collect_debt();
                        // Reap fds whose handle was closed (explicit/scope) or collected
                        // (GC Drop) — both enqueue on `socket_reap`; close them now,
                        // outside the arena borrow (no task context needed).
                        let reaped: Vec<StreamId> = arena
                            .mutate_root(|_mc, vm| vm.socket_reap.borrow_mut().drain(..).collect());
                        for id in reaped {
                            backend.close(id);
                        }
                    }
                }
            });
        }

        // The last program run leaves its result on top of the stack. Treat a VM
        // error (abort) as failure too, so callers can gate purely on the return.
        let passed = !aborted
            && arena.mutate_root(|_mc, vm| vm.stack.last().map(|v| v.is_truthy()).unwrap_or(false));

        arena.finish_cycle();
        passed
    }

    fn run_benchmark_iteration(
        &self,
        arena: &mut Arena<Rootable![VmState<'_>]>,
        receiver_name: &str,
        selector: &str,
        arg_ints: Vec<i64>,
    ) -> (u128, usize, usize) {
        let _initial_frame_count = arena.mutate_root(|mc, vm| {
            let receiver = vm
                .globals
                .borrow()
                .get(&NamespacedName::parse(receiver_name))
                .copied()
                .unwrap_or_else(|| panic!("{} not found", receiver_name));
            let args = arg_ints
                .iter()
                .map(|&i| vm.new_int(mc, i))
                .collect::<Vec<_>>();
            vm.start_method_call(mc, receiver, selector, args)
                .unwrap_or_else(|e| panic!("failed to start {}.{}: {}", receiver_name, selector, e))
        });

        arena.mutate_root(|mc, vm| {
            let fiber = Fiber::new(move |yielder, mut ctx| {
                let (vm, _mc) = unsafe { ctx.get() };
                vm.sched.yielder = Some(yielder as *const _ as *const ());

                loop {
                    let (vm, _mc) = unsafe { ctx.get() };
                    match vm.step(_mc) {
                        Ok(VmStatus::Running) => {
                            vm.sched.yielder = None;
                            ctx = yielder.suspend(YieldReason::CooperativeYield);
                            let (vm, _mc) = unsafe { ctx.get() };
                            vm.sched.yielder = Some(yielder as *const _ as *const ());
                        }
                        Ok(VmStatus::Finished(val)) => {
                            vm.sched.yielder = None;
                            return Ok(val);
                        }
                        Ok(VmStatus::Yeeted(val)) => {
                            vm.sched.yielder = None;
                            return Err(QuoinError::Other(format!("Uncaught exception: {}", val)));
                        }
                        Err(err) => {
                            vm.sched.yielder = None;
                            return Err(err);
                        }
                    }
                }
            });
            vm.sched.active_fiber = Some(gc!(mc, fiber));
        });

        let alloc_before = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());
        let start_time = Instant::now();

        let mut step_count = 0;
        loop {
            let is_done = arena.mutate_root(|mc, vm| {
                let Some(fiber) = vm.sched.active_fiber else {
                    return Ok(true);
                };

                let mut opt = fiber.coroutine.borrow_mut();
                let coro = opt.as_mut().expect("Coroutine already finished");

                let ctx = VMContext {
                    vm: vm as *mut _,
                    mc: mc as *const _,
                };

                match coro.resume(ctx) {
                    CoroutineResult::Yield(YieldReason::CooperativeYield) => Ok(false),
                    CoroutineResult::Yield(YieldReason::CallBlock { .. }) => Ok(false),
                    // Guest fibers are not used by the benchmark harness.
                    CoroutineResult::Yield(YieldReason::ResumeFiber { .. })
                    | CoroutineResult::Yield(YieldReason::YieldFiber { .. }) => {
                        panic!("guest fibers are not supported in benchmark mode")
                    }
                    CoroutineResult::Yield(YieldReason::AwaitIo { .. })
                    | CoroutineResult::Yield(YieldReason::Gather { .. })
                    | CoroutineResult::Yield(YieldReason::Join { .. })
                    | CoroutineResult::Yield(YieldReason::JoinTimed { .. }) => {
                        panic!("async I/O is not supported in benchmark mode")
                    }
                    CoroutineResult::Yield(YieldReason::Return(val)) => {
                        vm.sched.active_fiber = None;
                        vm.push(val);
                        Ok(true)
                    }
                    CoroutineResult::Return(res) => {
                        vm.sched.active_fiber = None;
                        match res {
                            Ok(val) => {
                                vm.push(val);
                                Ok(true)
                            }
                            Err(err) => Err(err),
                        }
                    }
                }
            });

            match is_done {
                Ok(true) => break,
                Ok(false) => {
                    step_count += 1;
                    if crate::tuning::gc_stress() || step_count % 10 == 0 {
                        arena.collect_debt();
                    }
                }
                Err(e) => {
                    panic!("VM execution error: {}", e);
                }
            }
        }

        let elapsed = start_time.elapsed().as_millis();

        arena.mutate_root(|_mc, vm| {
            let _ = vm.pop().expect("Failed to pop benchmark result");
        });

        let alloc_after = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());

        (elapsed, alloc_before, alloc_after)
    }

    fn compile_and_benchmark(&self, ast_iter: impl Iterator<Item = Node>) {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, self.options.vm_options.clone());

            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, boolean::build_boolean_class());
            vm.register_native_class(mc, block::build_block_class());
            vm.register_native_class(mc, bytes::build_bytes_class());
            vm.register_native_class(mc, sockets::build_tcp_socket_class());
            vm.register_native_class(mc, sockets::build_tls_socket_class());
            vm.register_native_class(mc, http::build_http_parser_class());
            vm.register_native_class(mc, io::build_io_folder_class());
            vm.register_native_class(mc, io::build_io_file_class());
            vm.register_native_class(mc, io::build_io_handle_class());
            vm.register_native_class(mc, list::build_list_class());
            vm.register_native_class(mc, set::build_set_class());
            vm.register_native_class(mc, runtime::build_runtime_class());
            vm.register_native_class(mc, async_rt::build_async_class());
            vm.register_native_class(mc, task::build_task_class());
            vm.register_native_class(mc, method::build_method_class());
            vm.register_native_class(mc, timer::build_timer_class());
            vm.register_native_class(mc, double::build_double_class());
            vm.register_native_class(mc, integer::build_integer_class());
            vm.register_native_class(mc, string::build_string_class());
            vm.register_native_class(mc, symbol::build_symbol_class());
            vm.register_native_class(mc, nil::build_nil_class());
            vm.register_native_class(mc, map::build_map_class());
            vm.register_native_class(mc, map::build_key_value_pair_class());
            vm.register_native_class(mc, regex::build_regex_class());
            vm.register_native_class(mc, fiber_class::build_fiber_class());

            vm
        });

        let mut aborted = false;
        for ast in ast_iter {
            if aborted {
                break;
            }

            arena.mutate_root(|mc, vm| {
                let program_node = match &ast.value {
                    NodeValue::Program(p) => p,
                    _ => {
                        panic!("Error: Root AST node is not a ProgramNode");
                    }
                };

                let mut compiler = Compiler::new();
                let program = match compiler.compile_program(program_node) {
                    Ok(p) => p,
                    Err(e) => {
                        panic!("Compilation error: {}", e);
                    }
                };

                let decl_block = program.decl_block.as_ref().map(|db| {
                    gc!(
                        mc,
                        Block {
                            name: db.name.clone(),
                            is_nested_block: db.is_nested_block,
                            param_syms: db.param_syms.clone(),
                            param_types: db.param_types.clone(),
                            bytecode: db.bytecode.clone(),
                            parent_env: None,
                            enclosing_method_id: None,
                            source_info: db.source_info.clone(),
                            decl_block: None,
                            source_map: db.source_map.clone(),
                        }
                    )
                });
                let main_block = gc!(
                    mc,
                    Block {
                        name: program.name.clone(),
                        is_nested_block: program.is_nested_block,
                        param_syms: program.param_syms.clone(),
                        param_types: program.param_types.clone(),
                        bytecode: program.bytecode.clone(),
                        parent_env: None,
                        enclosing_method_id: None,
                        source_info: program.source_info.clone(),
                        decl_block,
                        source_map: program.source_map.clone(),
                    }
                );
                vm.start_block(mc, main_block, Vec::new(), None, None);
            });

            let mut step_count = 0;
            loop {
                let status = arena.mutate_root(|mc, vm| match vm.step(mc) {
                    Ok(VmStatus::Running) => Ok(ExecutionStatus::Running),
                    Ok(VmStatus::Finished(_)) => Ok(ExecutionStatus::Finished),
                    Ok(VmStatus::Yeeted(val)) => {
                        println!("VM execution terminated with uncaught exception: {}", val);
                        Ok(ExecutionStatus::Yeeted)
                    }
                    Err(e) => Err(e),
                });
                match status {
                    Ok(ExecutionStatus::Running) => {
                        step_count += 1;
                        if crate::tuning::gc_stress() || step_count % 10 == 0 {
                            arena.collect_debt();
                        }
                    }
                    Ok(ExecutionStatus::Finished) => {
                        break;
                    }
                    Ok(ExecutionStatus::Yeeted) => {
                        aborted = true;
                        break;
                    }
                    Err(e) => {
                        eprintln!("VM execution error: {}", e);
                        aborted = true;
                        break;
                    }
                }
            }
        }

        if aborted {
            println!("Initialization aborted. Cannot run benchmarks.");
            return;
        }

        println!("==================================================");
        println!("RUST-TIMED BENCHMARK RUNNER (WITH GC)");
        println!("==================================================");

        let benchmarks = vec![
            ("Fibonacci (n = 20)", "Fib", "value:", vec![20]),
            (
                "Sieve of Eratosthenes (limit = 10000)",
                "Sieve",
                "primesUpTo:",
                vec![10000],
            ),
            (
                "Binary Trees (depth = 10)",
                "TreeBenchmark",
                "run:",
                vec![10],
            ),
        ];

        let mut averages = Vec::new();

        for &(name, receiver_name, selector, ref args) in &benchmarks {
            println!("Running: {}", name);
            let mut total_time = 0;
            let mut initial_alloc = 0;
            let mut final_alloc = 0;
            for iter in 1..=2 {
                let (elapsed, alloc_before, alloc_after) =
                    self.run_benchmark_iteration(&mut arena, receiver_name, selector, args.clone());
                if iter == 1 {
                    initial_alloc = alloc_before;
                }
                if iter == 2 {
                    final_alloc = alloc_after;
                }
                println!(
                    "  Iteration {}: {} ms (Heap: {} KB -> {} KB)",
                    iter,
                    elapsed,
                    alloc_before / 1024,
                    alloc_after / 1024
                );
                total_time += elapsed;
            }
            let avg = total_time / 2;
            averages.push((name, avg));
            println!("  Average: {} ms", avg);
            println!(
                "  Heap delta over iterations: {} KB -> {} KB (difference: {} KB)",
                initial_alloc / 1024,
                final_alloc / 1024,
                (final_alloc as i64 - initial_alloc as i64) / 1024
            );
            println!("--------------------------------------------------");
        }

        println!();
        println!("==================================================");
        println!("BENCHMARK SUMMARY (RUST-TIMED)");
        println!("==================================================");
        for &(name, avg) in &averages {
            println!("{:<38} {} ms", name.to_string() + ":", avg);
        }
        println!("==================================================");

        arena.finish_cycle();
    }
}
