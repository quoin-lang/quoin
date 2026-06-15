use crate::value::{NativeClassBuilder, ObjectPayload, Value};

pub fn build_object_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Object", None)
        .instance_method("s", |vm, mc, args| {
            Ok(vm.new_string(mc, format!("{}", args[0])))
        })
        .instance_method("id", |vm, mc, args| {
            let id_str = match args[0] {
                Value::Object(obj) => obj.borrow().id.0.to_string(),
                Value::Class(c) => format!("Class({})", c.borrow().name),
                Value::ClassMeta(c) => format!("ClassMeta({})", c.borrow().name),
            };
            Ok(vm.new_string(mc, id_str))
        })
        .instance_method("class", |vm, _mc, args| {
            let receiver = args[0];
            if let Some(c) = vm.get_class_for_lookup(receiver) {
                Ok(Value::Class(c))
            } else {
                Err(crate::error::BBError::Other(format!(
                    "Class not found for type {}",
                    receiver.type_name()
                )))
            }
        })
        .instance_method("~:", |vm, mc, args| {
            vm.call_method(mc, args[0], "==:", vec![args[1]])
        })
        .instance_method("==:", |vm, mc, args| {
            let lhs = args[0];
            let rhs = args[1];
            Ok(vm.new_bool(mc, lhs == rhs))
        })
        .instance_method("!=:", |vm, mc, args| {
            let lhs = args[0];
            let rhs = args[1];

            let eq_result = vm.call_method(mc, lhs, "==:", vec![rhs])?;
            let false_val = vm.new_bool(mc, false);
            Ok(vm.new_bool(mc, eq_result == false_val))
        })
        // TODO: call #init in #new/#new:
        .instance_method("init", |_vm, _mc, args| Ok(args[0]))
        .instance_method("print", |vm, mc, args| {
            let s_result = vm.call_method(mc, args[0], "s", vec![])?;

            println!(
                "{}",
                match s_result {
                    Value::Object(obj) => match &obj.borrow().payload {
                        ObjectPayload::String(string) => string.to_string(),
                        _ => format!("{}", s_result),
                    },
                    x => format!("{}", x),
                }
            );

            Ok(vm.new_nil(mc))
        })
        // TODO: implement throw properly
        .instance_method("throw", |_vm, _mc, args| Err(format!("{}", args[0]).into()))
}
