use crate::recv;
use crate::value::{NativeClassBuilder, Value};

pub fn build_boolean_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Boolean", Some("Object"))
        //
        .instance_method("s", |vm, mc, receiver, _args| {
            let b = recv!(receiver, Bool);
            Ok(vm.new_string(
                mc,
                if b {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver == args[0]))
        })
}
