use crate::arg;
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
        .instance_method("code", |vm, mc, _args| Ok(vm.new_nil(mc)))
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
            let args_list = arg!(args, List, 1);
            let block_args = args_list.borrow().clone();
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
            let block_args = match arg_val {
                Value::Object(obj) => match &obj.borrow().payload {
                    crate::value::ObjectPayload::List(l) => l.borrow().clone(),
                    _ => vec![arg_val],
                },
                _ => vec![arg_val],
            };
            vm.execute_block(mc, block, block_args, Some(self_val))
        })
}
