use crate::compiler::Compiler;
use crate::error::BBError;
use crate::fiber::{Fiber, VMContext, YieldReason};
use crate::gc;
use crate::highlighter::highlight_to_ansi;
use crate::parser::ast::Node;
use crate::parser::{parse_building_blocks_file, NodeValue};
use crate::runtime::{
    block, boolean, class, double, integer, io, list, map, method, native, nil, object, regex,
    runtime, string, timer,
};
use crate::value::{Block, NamespacedName, NativeClassBuilder};
use crate::vm::{VmState, VmStatus};

use corosensei::CoroutineResult;
use gc_arena::{Arena, Gc, Rootable};
use glob::glob;
use std::fs::read_to_string;
use std::iter::once_with;
use std::path::PathBuf;
use std::process::exit;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionStatus {
    Running,
    Finished,
    Yeeted,
}

pub struct VmRunnerOptions {
    pub mode: VmRunnerMode,
    pub target_path: Option<String>,
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
        if let Some(arg) = args.get(1) {
            if arg == "highlight" {
                return Self {
                    mode: VmRunnerMode::Highlight,
                    target_path: args.get(2).cloned(),
                };
            } else if arg == "test" {
                return Self {
                    mode: VmRunnerMode::Test,
                    target_path: None,
                };
            } else if arg == "benchmark" {
                return Self {
                    mode: VmRunnerMode::Benchmark,
                    target_path: None,
                };
            }
        }
        Self {
            mode: VmRunnerMode::Run,
            target_path: args.get(1).cloned(),
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

    pub fn run(&self) -> Result<(), BBError> {
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
                println!("Loading bblib/*.b...");
                let ast_iter = glob("bblib/*.b")
                    .unwrap()
                    .filter_map(|p| {
                        let path_buf = p.unwrap();
                        let path_s = path_buf.display().to_string();
                        if path_s == "bblib/test.b"
                            || (!path_s.starts_with("bblib/test")
                                && !path_s.ends_with("main.b")
                                && !path_s.ends_with("benchmark.b"))
                        {
                            println!("Loading file: {}", path_s);
                            let node = parse_building_blocks_file(&path_buf);
                            Some(node)
                        } else {
                            None
                        }
                    })
                    .chain(once_with(|| {
                        println!("Loading file: bblib/main.b");
                        parse_building_blocks_file(&PathBuf::from("bblib/main.b"))
                    }));

                self.compile_and_run_asts(ast_iter);
                Ok(())
            }
            VmRunnerMode::Benchmark => {
                println!("Loading bblib/*.b...");
                let ast_iter = glob("bblib/*.b")
                    .unwrap()
                    .filter_map(|p| {
                        let path_buf = p.unwrap();
                        let path_s = path_buf.display().to_string();
                        if !path_s.starts_with("bblib/test")
                            && !path_s.ends_with("main.b")
                            && !path_s.ends_with("benchmark.b")
                        {
                            println!("Loading file: {}", path_s);
                            let node = parse_building_blocks_file(&path_buf);
                            Some(node)
                        } else {
                            None
                        }
                    })
                    .chain(once_with(|| {
                        println!("Loading file: bblib/benchmark.b");
                        parse_building_blocks_file(&PathBuf::from("bblib/benchmark.b"))
                    }));

                self.compile_and_benchmark(ast_iter);
                Ok(())
            }
            VmRunnerMode::Run => {
                println!("Loading bblib/*.b...");
                let ast_iter = glob("bblib/*.b")
                    .unwrap()
                    .filter_map(|p| {
                        let path_buf = p.unwrap();
                        let path_s = path_buf.display().to_string();
                        if !path_s.starts_with("bblib/test")
                            && !path_s.ends_with("main.b")
                            && !path_s.ends_with("benchmark.b")
                        {
                            println!("Loading file: {}", path_s);
                            let node = parse_building_blocks_file(&path_buf);
                            Some(node)
                        } else {
                            None
                        }
                    })
                    .chain(once_with(|| {
                        let script_path = self
                            .options
                            .target_path
                            .as_deref()
                            .unwrap_or("bblib/testscript.b");
                        println!("Loading file: {}", script_path);
                        parse_building_blocks_file(&PathBuf::from(script_path))
                    }));

                self.compile_and_run_asts(ast_iter);
                Ok(())
            }
        }
    }

    fn compile_and_run_asts(&self, ast_iter: impl Iterator<Item = Node>) {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc);

            native::register_native_funcs(&mut vm, mc);

            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, boolean::build_boolean_class());
            vm.register_native_class(mc, block::build_block_class());
            vm.register_native_class(mc, io::build_io_folder_class());
            vm.register_native_class(mc, io::build_io_file_class());
            vm.register_native_class(mc, io::build_io_handle_class());
            vm.register_native_class(mc, list::build_list_class());
            vm.register_native_class(mc, runtime::build_runtime_class());
            vm.register_native_class(mc, method::build_method_class());
            vm.register_native_class(mc, timer::build_timer_class());
            vm.register_native_class(mc, double::build_double_class());
            vm.register_native_class(mc, integer::build_integer_class());
            vm.register_native_class(mc, string::build_string_class());
            vm.register_native_class(mc, nil::build_nil_class());
            vm.register_native_class(mc, map::build_map_class());
            vm.register_native_class(mc, map::build_key_value_pair_class());
            vm.register_native_class(mc, regex::build_regex_class());

            for t in ["Native"] {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }

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
                            param_names: db.param_names.clone(),
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
                        param_names: program.param_names.clone(),
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

                let fiber = Fiber::new(move |yielder, mut ctx| {
                    let (vm, _mc) = unsafe { ctx.get() };
                    vm.yielder = Some(yielder as *const _ as *const ());

                    loop {
                        let (vm, _mc) = unsafe { ctx.get() };
                        match vm.step(_mc) {
                            Ok(VmStatus::Running) => {
                                vm.yielder = None;
                                ctx = yielder.suspend(YieldReason::CooperativeYield);
                                let (vm, _mc) = unsafe { ctx.get() };
                                vm.yielder = Some(yielder as *const _ as *const ());
                            }
                            Ok(VmStatus::Finished(val)) => {
                                vm.yielder = None;
                                return Ok(val);
                            }
                            Ok(VmStatus::Yeeted(val)) => {
                                vm.yielder = None;
                                return Err(BBError::Other(format!("Uncaught exception: {}", val)));
                            }
                            Err(err) => {
                                vm.yielder = None;
                                return Err(err);
                            }
                        }
                    }
                });
                vm.active_fiber = Some(gc!(mc, fiber));
            });

            let mut step_count = 0;
            loop {
                let status = arena.mutate_root(|mc, vm| {
                    let Some(fiber) = vm.active_fiber else {
                        return Ok(ExecutionStatus::Finished);
                    };

                    let mut opt = fiber.coroutine.borrow_mut();
                    let coro = opt.as_mut().expect("Coroutine already finished");

                    let ctx = VMContext {
                        vm: vm as *mut _,
                        mc: mc as *const _,
                    };

                    match coro.resume(ctx) {
                        CoroutineResult::Yield(YieldReason::CooperativeYield) => {
                            Ok(ExecutionStatus::Running)
                        }
                        CoroutineResult::Yield(YieldReason::CallBlock { .. }) => {
                            Ok(ExecutionStatus::Running)
                        }
                        CoroutineResult::Yield(YieldReason::Return(val)) => {
                            vm.active_fiber = None;
                            vm.push(val);
                            Ok(ExecutionStatus::Finished)
                        }
                        CoroutineResult::Return(res) => {
                            vm.active_fiber = None;
                            match res {
                                Ok(val) => {
                                    vm.push(val);
                                    Ok(ExecutionStatus::Finished)
                                }
                                Err(err) => Err(err),
                            }
                        }
                    }
                });
                match status {
                    Ok(ExecutionStatus::Running) => {
                        step_count += 1;
                        if step_count % 10 == 0 {
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

        arena.finish_cycle();
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
                vm.yielder = Some(yielder as *const _ as *const ());

                loop {
                    let (vm, _mc) = unsafe { ctx.get() };
                    match vm.step(_mc) {
                        Ok(VmStatus::Running) => {
                            vm.yielder = None;
                            ctx = yielder.suspend(YieldReason::CooperativeYield);
                            let (vm, _mc) = unsafe { ctx.get() };
                            vm.yielder = Some(yielder as *const _ as *const ());
                        }
                        Ok(VmStatus::Finished(val)) => {
                            vm.yielder = None;
                            return Ok(val);
                        }
                        Ok(VmStatus::Yeeted(val)) => {
                            vm.yielder = None;
                            return Err(BBError::Other(format!("Uncaught exception: {}", val)));
                        }
                        Err(err) => {
                            vm.yielder = None;
                            return Err(err);
                        }
                    }
                }
            });
            vm.active_fiber = Some(gc!(mc, fiber));
        });

        let alloc_before = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());
        let start_time = Instant::now();

        let mut step_count = 0;
        loop {
            let is_done = arena.mutate_root(|mc, vm| {
                let Some(fiber) = vm.active_fiber else {
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
                    CoroutineResult::Yield(YieldReason::Return(val)) => {
                        vm.active_fiber = None;
                        vm.push(val);
                        Ok(true)
                    }
                    CoroutineResult::Return(res) => {
                        vm.active_fiber = None;
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
                    if step_count % 10 == 0 {
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
            let mut vm = VmState::new(mc);

            native::register_native_funcs(&mut vm, mc);

            vm.register_native_class(mc, object::build_object_class());
            vm.register_native_class(mc, class::build_class_class());
            vm.register_native_class(mc, boolean::build_boolean_class());
            vm.register_native_class(mc, block::build_block_class());
            vm.register_native_class(mc, io::build_io_folder_class());
            vm.register_native_class(mc, io::build_io_file_class());
            vm.register_native_class(mc, io::build_io_handle_class());
            vm.register_native_class(mc, list::build_list_class());
            vm.register_native_class(mc, runtime::build_runtime_class());
            vm.register_native_class(mc, method::build_method_class());
            vm.register_native_class(mc, timer::build_timer_class());
            vm.register_native_class(mc, double::build_double_class());
            vm.register_native_class(mc, integer::build_integer_class());
            vm.register_native_class(mc, string::build_string_class());
            vm.register_native_class(mc, nil::build_nil_class());
            vm.register_native_class(mc, map::build_map_class());
            vm.register_native_class(mc, map::build_key_value_pair_class());
            vm.register_native_class(mc, regex::build_regex_class());

            for t in ["Native"] {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }

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
                            param_names: db.param_names.clone(),
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
                        param_names: program.param_names.clone(),
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
                        if step_count % 10 == 0 {
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
