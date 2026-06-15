use crate::value::{NativeClassBuilder, Value};
use crate::{arg, gc};

use gc_arena::Gc;

pub fn build_class_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Class", Some("Object"))
        //
        .instance_method("name", |_vm, mc, args| {
            let clz = arg!(args, Class, 0);
            Ok(Value::String(gc!(mc, clz.borrow().name.clone())))
        })
}
