use crate::arg;
use crate::runtime::list::NativeListState;
use crate::value::{NativeClassBuilder, Value};

pub fn build_block_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Block", Some("Object"))
        .instance_method("arity", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            Ok(vm.new_int(mc, block.param_names.len() as i64))
        })
        .instance_method("args", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            Ok(vm.new_list(
                mc,
                block
                    .param_names
                    .iter()
                    .map(|s| vm.new_string(mc, s.clone()))
                    .collect(),
            ))
        })
        .instance_method("name", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            if let Some(name) = &block.name {
                Ok(vm.new_string(mc, name.clone()))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .instance_method("code", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            if let Some(source_info) = &block.source_info
                && let Some(text) = &source_info.source_text
            {
                Ok(vm.new_string(mc, text.clone()))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        // .instance_method("value", |vm, mc, args| {
        //     let block = arg!(args, Block, 0);
        //     vm.execute_block(mc, block, Vec::new(), None)
        // })
        // .instance_method("value:", |vm, mc, args| {
        //     let block = arg!(args, Block, 0);
        //     let val = args[1];
        //     vm.execute_block(mc, block, vec![val], None)
        // })
        .instance_method("valueWithArgs:", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            let block_args =
                args[1].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())?;
            vm.execute_block(mc, block, block_args, None)
        })
        .instance_method("valueWithSelf:", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            let self_val = args[1];
            vm.execute_block(mc, block, Vec::new(), Some(self_val))
        })
        .instance_method("value:withSelf:", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            let arg_val = args[1];
            let self_val = args[2];
            let block_args = match arg_val
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
            {
                Ok(vec) => vec,
                Err(_) => vec![arg_val],
            };
            vm.execute_block(mc, block, block_args, Some(self_val))
        })
        .instance_method("valueWithSelfOrArg:", |vm, mc, args| {
            let block = arg!(args, Block, 0);
            let arg_val = args[1];
            vm.execute_block(mc, block, vec![arg_val], Some(arg_val))
        })
        .instance_method("==:", |vm, mc, args| {
            Ok(vm.new_bool(mc, args[0] == args[1]))
        })
        .instance_method("catch:", |vm, mc, args| {
            let receiver_block = arg!(args, Block, 0);
            let catch_block = arg!(args, Block, 1);
            
            let initial_frame_count = vm.frames.len();
            match vm.execute_block(mc, receiver_block, Vec::new(), None) {
                Ok(val) => Ok(val),
                Err(e) => {
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    let exception_val = if let Some(val) = vm.active_exception.take() {
                        val
                    } else {
                        vm.new_string(mc, format!("{}", e))
                    };
                    
                    vm.execute_block(mc, catch_block, vec![exception_val], None)
                }
            }
        })
        .instance_method("catch:finally:", |vm, mc, args| {
            let receiver_block = arg!(args, Block, 0);
            let catch_block = arg!(args, Block, 1);
            let finally_block = arg!(args, Block, 2);
            
            let initial_frame_count = vm.frames.len();
            let res = vm.execute_block(mc, receiver_block, Vec::new(), None);
            match res {
                Ok(val) => {
                    vm.execute_block(mc, finally_block, Vec::new(), None)?;
                    Ok(val)
                }
                Err(e) => {
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    let exception_val = if let Some(val) = vm.active_exception.take() {
                        val
                    } else {
                        vm.new_string(mc, format!("{}", e))
                    };
                    
                    let catch_res = vm.execute_block(mc, catch_block, vec![exception_val], None);
                    while vm.frames.len() > initial_frame_count {
                        vm.frames.pop();
                    }
                    vm.execute_block(mc, finally_block, Vec::new(), None)?;
                    catch_res
                }
            }
        })
}
