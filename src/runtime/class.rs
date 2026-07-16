use crate::arg;
use crate::error::QuoinError;
use crate::recv;
use crate::value::{NamespacedName, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::DeferredCall;

/// The text of a `Symbol` or `String` argument — `#Point` and `'Point'` both name a class.
fn name_text(v: Value<'_>) -> Option<String> {
    match v {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::Symbol(s) => Some((**s).clone()),
            ObjectPayload::String(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

pub fn build_class_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Class", Some("Object"))
        .construct_with("define classes with Name <- { … }")
        .class_doc(
            "The class of classes: `Integer`, `List`, and every user-defined class are \
             instances of Class. Define one with `Name <- { ... }` (a subclass with \
             `Parent <- Name <- { ... }`), reopen one with `Name <-- { ... }`, and reflect \
             with `name` / `parent` / `class`. Note that on classes `==` is the SUBTYPE \
             test, and `Pattern ~ x` is instance-of.",
        )
        // Ask whether a class exists *by name*, rather than by reading the name and seeing
        // whether it came back nil — reading an unbound name is a `NameError`. Namespaced
        // classes need a quoted symbol: `Class.exists?:#'[ADBC]Database'`.
        .class_method("exists?:", |vm, mc, _receiver, args| {
            let Some(text) = name_text(args[0]) else {
                return Err(QuoinError::ValueError(
                    "Class.exists?: expects a Symbol or String, e.g. `Class.exists?:#Point`"
                        .to_string(),
                ));
            };
            let key = NamespacedName::parse(&text);
            let found = matches!(vm.globals.borrow().get(&key), Some(Value::Class(_)));
            Ok(vm.new_bool(mc, found))
        })
        .returns("Boolean")
        .doc(
            "Whether a class named by the Symbol or String argument is defined -- \
             `Class.exists?:#Point`, or `Class.exists?:#'[IO]File'` for a namespaced class. \
             The way to ask without reading the name, which would raise a NameError if it is \
             unbound.",
        )
        .instance_method("name", |vm, mc, receiver, _args| {
            let clz = recv!(receiver, Class);
            Ok(vm.new_string(mc, clz.borrow().name.to_string()))
        })
        .doc(
            "The class's name as a String, including any namespace ('[IO]File').\n\n\
             ```\n\
             Integer.name    \"* -> Integer\n\
             ```",
        )
        .instance_method("class", |vm, _mc, _receiver, _args| {
            let class_key = NamespacedName::new(Vec::new(), "Class".to_string());
            Ok(*vm
                .globals
                .borrow()
                .get(&class_key)
                .expect("Class global not found"))
        })
        .doc(
            "The class `Class` itself -- every class is an instance of Class.\n\n\
             ```\n\
             Integer.class.name    \"* -> Class\n\
             ```",
        )
        .instance_method("parent", |vm, mc, receiver, _args| {
            let clz = recv!(receiver, Class);
            let parent = clz.borrow().parent;
            if let Some(parent) = parent {
                Ok(Value::Class(parent))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .doc(
            "The superclass, or nil for Object (the root).\n\n\
             ```\n\
             Integer.parent.name    \"* -> Object\n\
             ```",
        )
        .instance_method("mix:", |vm, mc, receiver, args| {
            let clz = recv!(receiver, Class);
            vm.ensure_not_sealed(clz)?;
            let mixin = arg!(args, Class, 0);
            clz.borrow_mut(mc).mixin_classes.push(mixin);
            // A mixin changes what dispatch resolves (its methods can shadow a
            // PARENT's) and what instantiation runs (its ivars and init join
            // the chain): stale cached resolutions, inline caches, compiled
            // direct-self entries, and instantiation plans must all self-evict
            // — the same pairing every DefineMethod arm does. (mix: previously
            // invalidated NOTHING — a latent staleness bug on its own.)
            vm.invalidate_method_cache();
            crate::codegen::bump_redef_epoch();
            // Defer the mixin's requirement check to the end of the current block
            // (the class-definition body), when the host class is fully defined.
            // Only mixins that define a class-side assertMeetsRequirements: take part.
            if vm
                .lookup_in_class_hierarchy(mixin, "assertMeetsRequirements:", true)
                .is_some()
                && let Some(frame) = vm.frames.last_mut()
            {
                frame.defers.push(DeferredCall {
                    receiver: Value::Class(mixin),
                    selector: "assertMeetsRequirements:".to_string(),
                    args: vec![Value::Class(clz)],
                });
            }
            Ok(Value::Class(mixin))
        })
        .doc(
            "Mix a mixin class into the receiver: the mixin's methods join dispatch (they \
             can shadow a parent's), and its instance variables and `init` join \
             instantiation. If the mixin defines a class-side `assertMeetsRequirements:`, \
             that check runs at the end of the enclosing class body, once the host class is \
             fully defined. Answers the mixin.\n\n\
             ```\n\
             Mixin <- M <- { hi -> { 'hi' } };\n\
             A <- { .mix:M };\n\
             A.new.hi    \"* -> hi\n\
             ```",
        )
        .instance_method("sealed!", |_vm, mc, receiver, _args| {
            // Freeze the class: no further extension or subclassing.
            let clz = recv!(receiver, Class);
            clz.borrow_mut(mc).is_sealed = true;
            Ok(Value::Class(clz))
        })
        .doc(
            "Freeze the class: further extension (`Name <-- { ... }`, `mix:`) and \
             subclassing are refused with an error. Answers the class.",
        )
        .instance_method("abstract!", |_vm, mc, receiver, _args| {
            // Forbid instantiating this class itself (subclasses may still be).
            let clz = recv!(receiver, Class);
            clz.borrow_mut(mc).is_abstract = true;
            Ok(Value::Class(clz))
        })
        .doc(
            "Forbid instantiating this class itself: `new` on it raises, while subclasses \
             may still be instantiated. Answers the class.",
        )
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs = receiver;
            let rhs = args[0];
            let res = match (lhs, rhs) {
                (Value::Class(l), Value::Class(r)) => vm.is_subclass_of_clz(l, r),
                (Value::ClassMeta(l), Value::ClassMeta(r)) => vm.is_subclass_of_clz(l, r),
                (Value::Class(l), Value::ClassMeta(r)) => vm.is_subclass_of_clz(l, r),
                (Value::ClassMeta(l), Value::Class(r)) => vm.is_subclass_of_clz(l, r),
                _ => lhs == rhs,
            };
            Ok(vm.new_bool(mc, res))
        })
        .doc(
            "On classes, `==` is the SUBTYPE test: `A == B` is true when A is B or a \
             descendant of B (metaclasses compare the same way) -- so it is deliberately \
             not symmetric.\n\n\
             ```\n\
             Integer == Object    \"* -> true\n\
             Object == Integer    \"* -> false\n\
             ```",
        )
}
