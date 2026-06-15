use crate::arg;
use crate::value::{NamespacedName, NativeClassBuilder, Value};

pub fn build_class_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Class", Some("Object"))
        .instance_method("name", |vm, mc, args| {
            let clz = arg!(args, Class, 0);
            Ok(vm.new_string(mc, clz.borrow().name.to_string()))
        })
        .instance_method("class", |vm, _mc, _args| {
            let class_key = NamespacedName::new(Vec::new(), "Class".to_string());
            Ok(vm
                .globals
                .borrow()
                .get(&class_key)
                .expect("Class global not found")
                .clone())
        })
        .instance_method("parent", |vm, mc, args| {
            let clz = arg!(args, Class, 0);
            let parent = clz.borrow().parent;
            if let Some(parent) = parent {
                Ok(Value::Class(parent))
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .instance_method("mix:", |_vm, mc, args| {
            let clz = arg!(args, Class, 0);
            let mixin = arg!(args, Class, 1);
            clz.borrow_mut(mc).mixin_classes.push(mixin);
            Ok(Value::Class(mixin))
        })
        .instance_method("can:", |_vm, mc, args| {
            let clz = arg!(args, Class, 0);
            let mixin = arg!(args, Class, 1);
            clz.borrow_mut(mc).mixin_classes.push(mixin);
            Ok(Value::Class(mixin))
        })
        .instance_method("sealed!", |vm, mc, _args| {
            // TODO: implement this
            Ok(vm.new_nil(mc))
        })
        .instance_method("abstract!", |vm, mc, _args| {
            // TODO: implement this
            Ok(vm.new_nil(mc))
        })
}
