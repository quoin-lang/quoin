use new_vm::parser::{ast_visitor, parser};
use new_vm::runtime::{
    block, boolean, class, double, integer, io, list, map, method, native, nil, object, regex,
    runtime, string, timer,
};
use new_vm::value::{Block, NativeClassBuilder};
use new_vm::vm::{VmState, VmStatus};
use new_vm::{compiler, gc};

use gc_arena::{Arena, Gc, Rootable};
use glob::glob;
use new_vm::parser::ast_visitor::Node;

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
                    || (!path_s.starts_with("bblib/test") && !path_s.ends_with("main.b"))
                {
                    println!("Loading file: {}", path_s);
                    let node = parser::parse_building_blocks_file(&path_buf);
                    Some(node)
                } else {
                    None
                }
            })
            .chain(vec![{
                println!("Loading file: bblib/main.b");
                parser::parse_building_blocks_file(&std::path::PathBuf::from("bblib/main.b"))
            }]);

        compile_and_run_asts(ast_iter);
        return;
    }

    println!("Loading bblib/*.b...");
    let ast_iter = glob("bblib/*.b")
        .unwrap()
        .filter_map(|p| {
            let path_buf = p.unwrap();
            let path_s = path_buf.display().to_string();
            if !path_s.starts_with("bblib/test") && !path_s.ends_with("main.b") {
                println!("Loading file: {}", path_s);
                let node = parser::parse_building_blocks_file(&path_buf);
                Some(node)
            } else {
                None
            }
        })
        .chain(vec![{
            let script_path = args.get(1).map(|s| s.as_str()).unwrap_or("bblib/testscript.b");
            println!("Loading file: {}", script_path);
            parser::parse_building_blocks_file(&std::path::PathBuf::from(script_path))
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
                ast_visitor::NodeValue::Program(p) => p,
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
