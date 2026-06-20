use crate::error::BBError;
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
                Err(BBError::Other(format!(
                    "Class not found for type {}",
                    receiver.type_name()
                )))
            }
        })
        // `can?:` is overloaded by argument:
        //   - a Symbol or String selector -> does the receiver implement that method?
        //     (instance methods for an instance or class receiver; class-side
        //     methods for a metaclass receiver)
        //   - a Class -> is the receiver an instance of / does it mix in that class?
        .instance_method("can?:", |vm, mc, args| {
            let receiver = args[0];
            let cap = args[1];
            let responds = if let Value::Class(c) = cap {
                vm.is_instance_of(receiver, c)
            } else {
                let name = match cap {
                    Value::Object(obj) => match &obj.borrow().payload {
                        ObjectPayload::Symbol(s) | ObjectPayload::String(s) => Some((**s).clone()),
                        _ => None,
                    },
                    _ => None,
                };
                let name = name.ok_or_else(|| BBError::TypeError {
                    expected: "Symbol, String, or Class".to_string(),
                    got: cap.type_name().to_string(),
                    msg: "can?: expects a selector (symbol or string) or a class".to_string(),
                })?;
                match receiver {
                    Value::Object(obj) => {
                        let class = obj.borrow().class;
                        vm.lookup_in_class_hierarchy(class, &name, false).is_some()
                    }
                    Value::Class(c) => vm.lookup_in_class_hierarchy(c, &name, false).is_some(),
                    Value::ClassMeta(c) => vm.lookup_in_class_hierarchy(c, &name, true).is_some(),
                }
            };
            Ok(vm.new_bool(mc, responds))
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
        .instance_method("throw", |vm, _mc, args| {
            vm.active_exception = Some(args[0]);
            Err(BBError::Thrown)
        })
}
