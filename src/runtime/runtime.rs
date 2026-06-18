use crate::arg;
use crate::compiler::Compiler;
use crate::error::BBError;
use crate::parser::ast::NodeValue;
use crate::parser::{parse_building_blocks_file, parse_building_blocks_string};
use crate::value::{Block, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::{Gc, Mutation};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn build_runtime_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Runtime", Some("Object"))
        .class_method("eval:", |vm, mc, args| {
            let code = arg!(args, String, 1);
            eval_string(vm, mc, &code, "<string>", None)
        })
        .class_method("eval:self:", |vm, mc, args| {
            let code = arg!(args, String, 1);
            let self_val = args[2];
            eval_string(vm, mc, &code, "<string>", Some(self_val))
        })
        .class_method("evalFile:", |vm, mc, args| {
            let filename = arg!(args, String, 1);
            eval_file(vm, mc, &filename, None)
        })
        .class_method("evalFile:self:", |vm, mc, args| {
            let filename = arg!(args, String, 1);
            let self_val = args[2];
            eval_file(vm, mc, &filename, Some(self_val))
        })
        .class_method("arguments", |vm, mc, _args| {
            let args_list = vm
                .options
                .arguments
                .iter()
                .map(|s| vm.new_string(mc, s.clone()))
                .collect::<Vec<_>>();
            Ok(vm.new_list(mc, args_list))
        })
        .class_method("options", |vm, mc, _args| {
            let mut map = HashMap::new();
            let args_list = vm
                .options
                .arguments
                .iter()
                .map(|s| vm.new_string(mc, s.clone()))
                .collect::<Vec<_>>();
            map.insert("arguments".to_string(), vm.new_list(mc, args_list));
            let supports_color_val = vm.new_bool(mc, vm.options.supports_color);
            map.insert("supports_color".to_string(), supports_color_val);
            Ok(vm.new_map(mc, map))
        })
        .class_method("supportsColor", |vm, mc, _args| {
            Ok(vm.new_bool(mc, vm.options.supports_color))
        })
}

fn eval_string<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    code: &str,
    _filename: &str,
    self_val: Option<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    let ast = parse_building_blocks_string(code);

    let mut compiler = Compiler::new();
    let program_node = match &ast.value {
        NodeValue::Program(p) => p,
        _ => {
            return Err(BBError::Other(
                "Expected Program node from parser".to_string(),
            ));
        }
    };

    let static_block = compiler
        .compile_program(program_node)
        .map_err(|e| BBError::Other(format!("Compilation error: {}", e)))?;

    let decl_block = static_block.decl_block.as_ref().map(|db| {
        crate::gc!(
            mc,
            Block {
                source_info: db.source_info.clone(),
                name: db.name.clone(),
                is_nested_block: db.is_nested_block,
                param_names: db.param_names.clone(),
                param_types: db.param_types.clone(),
                bytecode: db.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: db.source_map.clone(),
            }
        )
    });

    let block = crate::gc!(
        mc,
        Block {
            source_info: static_block.source_info.clone(),
            name: static_block.name.clone(),
            is_nested_block: static_block.is_nested_block,
            param_names: static_block.param_names.clone(),
            param_types: static_block.param_types.clone(),
            bytecode: static_block.bytecode.clone(),
            parent_env: None,
            enclosing_method_id: None,
            decl_block,
            source_map: static_block.source_map.clone(),
        }
    );

    vm.execute_block(mc, block, Vec::new(), self_val)
}

fn eval_file<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    filename: &str,
    self_val: Option<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    let path = PathBuf::from(filename);
    if !path.exists() {
        return Err(BBError::Other(format!("File not found: {}", filename)));
    }

    let ast = parse_building_blocks_file(&path);

    let mut compiler = Compiler::new();
    let program_node = match &ast.value {
        NodeValue::Program(p) => p,
        _ => {
            return Err(BBError::Other(
                "Expected Program node from parser".to_string(),
            ));
        }
    };

    let static_block = compiler
        .compile_program(program_node)
        .map_err(|e| BBError::Other(format!("Compilation error: {}", e)))?;

    let decl_block = static_block.decl_block.as_ref().map(|db| {
        crate::gc!(
            mc,
            Block {
                source_info: db.source_info.clone(),
                name: db.name.clone(),
                is_nested_block: db.is_nested_block,
                param_names: db.param_names.clone(),
                param_types: db.param_types.clone(),
                bytecode: db.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: db.source_map.clone(),
            }
        )
    });

    let block = crate::gc!(
        mc,
        Block {
            source_info: static_block.source_info.clone(),
            name: static_block.name.clone(),
            is_nested_block: static_block.is_nested_block,
            param_names: static_block.param_names.clone(),
            param_types: static_block.param_types.clone(),
            bytecode: static_block.bytecode.clone(),
            parent_env: None,
            enclosing_method_id: None,
            decl_block,
            source_map: static_block.source_map.clone(),
        }
    );

    vm.execute_block(mc, block, Vec::new(), self_val)
}
