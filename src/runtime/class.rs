use crate::arg;
use crate::recv;
use crate::value::{NamespacedName, NativeClassBuilder, Value};
use crate::vm::DeferredCall;

pub fn build_class_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Class", Some("Object"))
        .instance_method("name", |vm, mc, receiver, _args| {
            let clz = recv!(receiver, Class);
            Ok(vm.new_string(mc, clz.borrow().name.to_string()))
        })
        .instance_method("class", |vm, _mc, _receiver, _args| {
            let class_key = NamespacedName::new(Vec::new(), "Class".to_string());
            Ok(vm
                .globals
                .borrow()
                .get(&class_key)
                .expect("Class global not found")
                .clone())
        })
        .instance_method("parent", |vm, mc, receiver, _args| {
            let clz = recv!(receiver, Class);
            let parent = clz.borrow().parent;
            if let Some(parent) = parent {
                Ok(Value::Class(parent))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .instance_method("mix:", |vm, mc, receiver, args| {
            let clz = recv!(receiver, Class);
            vm.ensure_not_sealed(clz)?;
            let mixin = arg!(args, Class, 0);
            clz.borrow_mut(mc).mixin_classes.push(mixin);
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
        .instance_method("sealed!", |_vm, mc, receiver, _args| {
            // Freeze the class: no further extension or subclassing.
            let clz = recv!(receiver, Class);
            clz.borrow_mut(mc).is_sealed = true;
            Ok(Value::Class(clz))
        })
        .instance_method("abstract!", |_vm, mc, receiver, _args| {
            // Forbid instantiating this class itself (subclasses may still be).
            let clz = recv!(receiver, Class);
            clz.borrow_mut(mc).is_abstract = true;
            Ok(Value::Class(clz))
        })
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
}
