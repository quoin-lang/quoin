use new_vm::error::BBError;
use new_vm::parser::{ast_visitor, parser};
use new_vm::runtime::{block, boolean, class, io, list, native, object, runtime, method, timer};
use new_vm::value::{Block, NativeClassBuilder, ObjectPayload, Value};
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
            println!("Loading file: bblib/testscript.b");
            parser::parse_building_blocks_file(&std::path::PathBuf::from("bblib/testscript.b"))
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

        // Register placeholder classes for all of the builtin types.
        for t in [
            "Nil",
            "Integer",
            "Double",
            "String",
            "Dictionary",
            "Regex",
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
            } else if t == "String" {
                let class_builder = NativeClassBuilder::new(t, Some("Object"))
                    .instance_method("replace:with:", |vm, mc, args| {
                        if args.len() < 3 {
                            return Err(BBError::Other("replace:with: expects receiver, pattern, and replacement".to_string()));
                        }
                        let receiver = args[0];
                        let from_val = args[1];
                        let to_val = args[2];

                        let s_borrow = match receiver {
                            Value::Object(obj) => match &obj.borrow().payload {
                                ObjectPayload::String(s) => s.clone(),
                                _ => return Err(BBError::Other("replace:with: expected String receiver".to_string())),
                            },
                            _ => return Err(BBError::Other("replace:with: expected String receiver".to_string())),
                        };

                        let to_str = match to_val {
                            Value::Object(obj) => match &obj.borrow().payload {
                                ObjectPayload::String(s) => s.to_string(),
                                _ => return Err(BBError::Other("replace:with: expected String replacement".to_string())),
                            },
                            _ => return Err(BBError::Other("replace:with: expected String replacement".to_string())),
                        };

                        if let Value::Object(obj) = from_val
                            && let ObjectPayload::Regex(r) = &obj.borrow().payload
                        {
                            let result = r.0.replace_all(&*s_borrow, &to_str).to_string();
                            return Ok(vm.new_string(mc, result));
                        }

                        if let Value::Object(obj) = from_val
                            && let ObjectPayload::String(s) = &obj.borrow().payload
                        {
                            let result = s_borrow.replace(&**s, &to_str);
                            return Ok(vm.new_string(mc, result));
                        }

                        Err(BBError::TypeError {
                            expected: "Regex or String".to_string(),
                            got: from_val.type_name().to_string(),
                            msg: "replace:with: expected Regex or String pattern".to_string(),
                        })
                    });
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
                    source_info: program.source_info.clone(),
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
