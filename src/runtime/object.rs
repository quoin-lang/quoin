use crate::gc;
use crate::value::{NativeClassBuilder, Value};

use gc_arena::Gc;

pub fn build_object_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Object", None)
        .instance_method("s", |_vm, mc, args| {
            Ok(Value::String(gc!(mc, format!("{}", args[0]))))
        })
        .instance_method("id", |_vm, mc, args| {
            let value = args[0];

            let id: Value = match value {
                Value::Object(obj) => Value::String(gc!(mc, obj.borrow().id.0.to_string())),
                _ => todo!(),
            };
            Ok(id)
        })
        .instance_method("print", |vm, mc, args| {
            let s_result = vm.call_method(mc, args[0], "s", vec![])?;

            println!(
                "{}",
                match s_result {
                    Value::String(string) => string.to_string(),
                    x => format!("{:?}", x),
                }
            );

            Ok(Value::Nil)
        })
        .instance_method("throw", |_vm, _mc, args| {
            // TODO: implement throw properly
            Err(format!("{}", args[0]).into())
        })
}
