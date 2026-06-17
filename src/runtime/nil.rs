use crate::value::NativeClassBuilder;

pub fn build_nil_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Nil", Some("Object"))
        .instance_method("==:", |vm, mc, args| {
            Ok(vm.new_bool(mc, args[1].is_nil()))
        })
}
