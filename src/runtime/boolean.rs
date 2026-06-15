use crate::value::{NativeClassBuilder, Value};
use crate::{arg, gc};

use gc_arena::Gc;

pub fn build_boolean_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Boolean", Some("Object"))
        //
        .instance_method("s", |_vm, mc, args| {
            let b = arg!(args, Bool, 0);
            Ok(Value::String(if b {
                gc!(mc, "true".to_string())
            } else {
                gc!(mc, "false".to_string())
            }))
        })
}
