use crate::value::{NativeClassBuilder, Value};
use crate::{arg, gc};

use gc_arena::Gc;

pub fn build_boolean_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Boolean", Some("Object"))
        //
        .instance_method("s", |vm, mc, args| {
            let b = arg!(args, Bool, 0);
            Ok(vm.new_string(
                mc,
                if b {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ))
        })
}
