use new_vm::parser::{ast, parse_building_blocks_file};
use new_vm::runtime::{
    block, boolean, class, double, integer, io, list, map, method, native, nil, object, regex,
    runtime, string, timer,
};
use new_vm::value::{Block, NativeClassBuilder};
use new_vm::vm::{VmState, VmStatus};
use new_vm::{compiler, gc};

use gc_arena::{Arena, Gc, Rootable};
use glob::glob;
use new_vm::parser::ast::Node;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionStatus {
    Running,
    Finished,
    Yeeted,
}

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    if let Some(arg) = args.get(1)
        && arg == "highlight"
    {
        let Some(path) = args.get(2) else {
            eprintln!("Usage: cargo run -- highlight FILE");
            std::process::exit(2);
        };
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", path, e);
                std::process::exit(1);
            }
        };
        print!("{}", new_vm::highlighter::highlight_to_ansi(&source));
        return;
    }

    if let Some(arg) = args.get(1)
        && arg == "test"
    {
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
            .chain(vec![{
                println!("Loading file: bblib/main.b");
                parse_building_blocks_file(&std::path::PathBuf::from("bblib/main.b"))
            }]);

        compile_and_run_asts(ast_iter);
        return;
    }

    if let Some(arg) = args.get(1)
        && arg == "benchmark"
    {
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
            .chain(vec![{
                println!("Loading file: bblib/benchmark.b");
                parse_building_blocks_file(&std::path::PathBuf::from("bblib/benchmark.b"))
            }]);

        compile_and_benchmark(ast_iter);
        return;
    }

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
        .chain(vec![{
            let script_path = args.get(1).map(|s| s.as_str()).unwrap_or("bblib/testscript.b");
            println!("Loading file: {}", script_path);
            parse_building_blocks_file(&std::path::PathBuf::from(script_path))
        }]);

    compile_and_run_asts(ast_iter);
}

fn compile_and_run_asts(ast_iter: impl Iterator<Item = Node>) {
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

        // Register placeholder classes for remaining builtin types.
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
                ast::NodeValue::Program(p) => p,
                _ => {
                    panic!("Error: Root AST node is not a ProgramNode");
                }
            };

            let mut compiler = compiler::Compiler::new();
            let program = match compiler.compile_program(program_node) {
                Ok(p) => p,
                Err(e) => {
                    panic!("Compilation error: {}", e);
                }
            };

            // Convert StaticBlock to Block in GC and start it
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

    arena.finish_cycle();
}

fn run_benchmark_iteration(
    arena: &mut Arena<Rootable![VmState<'_>]>,
    receiver_name: &str,
    selector: &str,
    arg_ints: Vec<i64>,
) -> (u128, usize, usize) {
    // 1. Start the method call
    let initial_frame_count = arena.mutate_root(|mc, vm| {
        let receiver = vm.globals.borrow().get(&new_vm::value::NamespacedName::parse(receiver_name))
            .copied()
            .unwrap_or_else(|| panic!("{} not found", receiver_name));
        let args = arg_ints.iter().map(|&i| vm.new_int(mc, i)).collect::<Vec<_>>();
        vm.start_method_call(mc, receiver, selector, args).unwrap_or_else(|e| panic!("failed to start {}.{}: {}", receiver_name, selector, e))
    });

    let alloc_before = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());

    // 2. Measure start time
    let start_time = std::time::Instant::now();

    // 3. Step loop
    let mut step_count = 0;
    loop {
        let is_done = arena.mutate_root(|mc, vm| {
            if vm.frames.len() <= initial_frame_count {
                return true;
            }
            match vm.step(mc) {
                Ok(VmStatus::Running) => {}
                Ok(VmStatus::Finished(_)) => {}
                Ok(VmStatus::Yeeted(val)) => {
                    panic!("VM execution terminated with uncaught exception: {}", val);
                }
                Err(e) => {
                    panic!("VM execution error: {}", e);
                }
            }
            false
        });

        if is_done {
            break;
        }

        step_count += 1;
        if step_count % 10 == 0 {
            arena.collect_debt();
        }
    }

    let elapsed = start_time.elapsed().as_millis();
    
    // 4. Pop the result
    arena.mutate_root(|_mc, vm| {
        let _ = vm.pop().expect("Failed to pop benchmark result");
    });

    let alloc_after = arena.mutate_root(|mc, _| mc.metrics().total_gc_allocation());

    (elapsed, alloc_before, alloc_after)
}

fn compile_and_benchmark(ast_iter: impl Iterator<Item = Node>) {
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

        // Register placeholder classes for remaining builtin types.
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
                ast::NodeValue::Program(p) => p,
                _ => {
                    panic!("Error: Root AST node is not a ProgramNode");
                }
            };

            let mut compiler = compiler::Compiler::new();
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
        ("Sieve of Eratosthenes (limit = 10000)", "Sieve", "primesUpTo:", vec![10000]),
        ("Binary Trees (depth = 10)", "TreeBenchmark", "run:", vec![10]),
    ];

    let mut averages = Vec::new();

    for &(name, receiver_name, selector, ref args) in &benchmarks {
        println!("Running: {}", name);
        let mut total_time = 0;
        let mut initial_alloc = 0;
        let mut final_alloc = 0;
        for iter in 1..=2 {
            let (elapsed, alloc_before, alloc_after) = run_benchmark_iteration(&mut arena, receiver_name, selector, args.clone());
            if iter == 1 {
                initial_alloc = alloc_before;
            }
            if iter == 2 {
                final_alloc = alloc_after;
            }
            println!("  Iteration {}: {} ms (Heap: {} KB -> {} KB)", iter, elapsed, alloc_before / 1024, alloc_after / 1024);
            total_time += elapsed;
        }
        let avg = total_time / 2;
        averages.push((name, avg));
        println!("  Average: {} ms", avg);
        println!("  Heap delta over iterations: {} KB -> {} KB (difference: {} KB)", initial_alloc / 1024, final_alloc / 1024, (final_alloc as i64 - initial_alloc as i64) / 1024);
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
