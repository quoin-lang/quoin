use crate::value::{NativeClassBuilder, Value};
use crate::arg;

pub fn build_class_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Class", Some("Object"))
        .instance_method("name", |vm, mc, args| {
            let clz = arg!(args, Class, 0);
            Ok(vm.new_string(mc, clz.borrow().name.clone()))
        })
        .instance_method("class", |vm, _mc, _args| {
            Ok(vm
                .globals
                .borrow()
                .get("Class")
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
        .instance_method("sealed!", |vm, mc, _args| {
            Ok(vm.new_nil(mc))
        })
}
