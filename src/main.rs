use new_vm::error::BBError;
use new_vm::parser::{ast_visitor, parser};
use new_vm::runtime::{block, boolean, class, io, native, object};
use new_vm::value::{Block, NativeClassBuilder, ObjectPayload, Value};
use new_vm::vm::{VmState, VmStatus};
use new_vm::{compiler, gc};
use std::fs::read_to_string;

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
        && arg == "load"
    {
        println!("Loading bblib/*.b...");

        let ast_iter = glob("bblib/*.b").unwrap().filter_map(|p| {
            let path_buf = p.unwrap();
            let path_s = path_buf.display().to_string();
            if !path_s.starts_with("bblib/test") && !path_s.ends_with("main.b") {
                println!("Loading file: {}", path_s);
                let node = parser::parse_building_blocks_file(&path_buf);
                Some(node)
            } else {
                None
            }
        });
        compile_and_run_asts(ast_iter);
    } else {
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
                println!("Loading file: bblib/testscript.b");
                parser::parse_building_blocks_string(&read_to_string("bblib/testscript.b").unwrap())
            }]);

        compile_and_run_asts(ast_iter);
    }

    // if let Some(arg) = args.get(1)
    //     && arg == "test"
    // {
    //     println!("Loading bblib/*.b...");
    //
    //     let ast_iter = glob("bblib/*.b")
    //         .unwrap()
    //         .filter_map(|p| {
    //             let path_buf = p.unwrap();
    //             let path_s = path_buf.display().to_string();
    //             if path_s == "bblib/test.b"
    //                 || (!path_s.starts_with("bblib/test") && !path_s.ends_with("main.b"))
    //             {
    //                 println!("Loading file: {}", path_s);
    //                 let node = parser::parse_building_blocks_file(&path_buf);
    //                 Some(node)
    //             } else {
    //                 None
    //             }
    //         })
    //         .chain(vec![{
    //             println!("Loading file: bblib/main.b");
    //             parser::parse_building_blocks_string(&read_to_string("bblib/main.b").unwrap())
    //         }]);
    //     compile_and_run_asts(ast_iter);
    //     return;
    // }
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

        // Register placeholder classes for all of the builtin types.
        for t in [
            "Nil",
            "Integer",
            "Double",
            "String",
            "List",
            "Dictionary",
            "Regex",
            "Method",
            "Native",
        ] {
            if t == "Double" || t == "Integer" {
                let class_builder = NativeClassBuilder::new(t, Some("Object")).instance_method(
                    "sqrt",
                    |vm, mc, args| {
                        if args.is_empty() {
                            return Err(BBError::Other("sqrt expects a receiver".to_string()));
                        }
                        let payload = match args[0] {
                            Value::Object(obj) => &obj.borrow().payload,
                            _ => {
                                return Err(BBError::Other(format!(
                                    "sqrt expected number, got {:?}",
                                    args[0]
                                )));
                            }
                        };
                        match payload {
                            ObjectPayload::Double(f) => Ok(vm.new_double(mc, f.sqrt())),
                            ObjectPayload::Int(i) => Ok(vm.new_double(mc, (*i as f64).sqrt())),
                            _ => Err(BBError::Other(format!(
                                "sqrt expected number, got {:?}",
                                args[0]
                            ))),
                        }
                    },
                );
                vm.register_native_class(mc, class_builder);
            } else {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }
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
            let main_block = gc!(
                mc,
                Block {
                    name: program.name.clone(),
                    is_nested_block: program.is_nested_block,
                    param_names: program.param_names.clone(),
                    bytecode: program.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                }
            );
            vm.start_block(mc, main_block, Vec::new());
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
