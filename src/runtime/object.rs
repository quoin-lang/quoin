use crate::error::QuoinError;
use crate::runtime::pretty;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

pub fn build_object_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Object", None)
        // The default `.s` for a value with no intrinsic human form: fall back to the
        // structural `.pp`. The Rust Display impl is for Rust-level debugging only — no Quoin
        // `.s` routes through it. (Types with an intrinsic form — Integer, String, Error, … —
        // override this.)
        .instance_method("s", |vm, mc, receiver, _args| {
            let width = vm.options.console_width.map(|w| w as usize).unwrap_or(80);
            // Methods return plain text; color is a REPL display concern (the `=>` path).
            Ok(vm.new_string(mc, pretty::render(receiver, width, false)))
        })
        // `pp` — a structural, canonical dump of the value graph for debugging/inspection
        // (escaped strings, instance vars, intrinsic collections). Width-aware: defaults to the
        // console width; `pp:` takes an explicit width. Never calls `.s`.
        .instance_method("pp", |vm, mc, receiver, _args| {
            let width = vm.options.console_width.map(|w| w as usize).unwrap_or(80);
            // Methods return plain text; color is a REPL display concern (the `=>` path).
            Ok(vm.new_string(mc, pretty::render(receiver, width, false)))
        })
        .instance_method("pp:", |vm, mc, receiver, args| {
            let width = match args.first() {
                Some(Value::Int(w)) if *w > 0 => *w as usize,
                _ => 80,
            };
            // Methods return plain text; color is a REPL display concern (the `=>` path).
            Ok(vm.new_string(mc, pretty::render(receiver, width, false)))
        })
        .instance_method("sealed!", |vm, mc, receiver, _args| {
            // Seal an instance: get-or-create its eigenclass and freeze it, so further
            // `<--` on this instance is refused. (`Class#sealed!` handles class
            // receivers; for a value type this targets the type's shared class, matching
            // how `value <-- {…}` extends it.)
            let tc = vm
                .get_target_class_for_def(mc, receiver)
                .map_err(QuoinError::Other)?;
            tc.borrow_mut(mc).is_sealed = true;
            Ok(receiver)
        })
        .instance_method("class", |vm, _mc, receiver, _args| {
            if let Some(c) = vm.get_class_for_lookup(receiver) {
                Ok(Value::Class(c))
            } else {
                Err(QuoinError::Other(format!(
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
        .instance_method("can?:", |vm, mc, receiver, args| {
            let cap = args[0];
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
                let name = name.ok_or_else(|| QuoinError::TypeError {
                    expected: "Symbol, String, or Class".to_string(),
                    got: cap.type_name().to_string(),
                    msg: "can?: expects a selector (symbol or string) or a class".to_string(),
                })?;
                match receiver {
                    Value::Class(c) => vm.lookup_in_class_hierarchy(c, &name, false).is_some(),
                    Value::ClassMeta(c) => vm.lookup_in_class_hierarchy(c, &name, true).is_some(),
                    // Object + immediate value types: dispatch via the derived class.
                    _ => match vm.get_class_for_lookup(receiver) {
                        Some(class) => vm.lookup_in_class_hierarchy(class, &name, false).is_some(),
                        None => false,
                    },
                }
            };
            Ok(vm.new_bool(mc, responds))
        })
        .instance_method("~:", |vm, mc, receiver, args| {
            vm.call_method(mc, receiver, "==:", vec![args[0]])
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs = receiver;
            let rhs = args[0];
            Ok(vm.new_bool(mc, lhs == rhs))
        })
        .instance_method("!=:", |vm, mc, receiver, args| {
            let lhs = receiver;
            let rhs = args[0];

            let eq_result = vm.call_method(mc, lhs, "==:", vec![rhs])?;
            let false_val = vm.new_bool(mc, false);
            Ok(vm.new_bool(mc, eq_result == false_val))
        })
        .instance_method("init", |_vm, _mc, receiver, _args| Ok(receiver))
        .instance_method("print", |vm, mc, receiver, _args| {
            let s_result = vm.call_method(mc, receiver, "s", vec![])?;
            let text = match s_result {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::String(string) => string.to_string(),
                    _ => format!("{}", s_result),
                },
                x => format!("{}", x),
            };
            // Route through the VM's stdout sink (not `println!`) so the DAP adapter can capture
            // program output as `output` events instead of it hitting fd 1/2 directly.
            vm.write_std(crate::vm::StdStream::Out, format!("{text}\n").as_bytes())
                .map_err(|e| QuoinError::Other(e.to_string()))?;
            Ok(vm.new_nil(mc))
        })
        .instance_method("throw", |vm, _mc, receiver, _args| {
            vm.active_exception = Some(receiver);
            Err(QuoinError::Thrown)
        })
}
