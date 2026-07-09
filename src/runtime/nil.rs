use crate::value::NativeClassBuilder;

pub fn build_nil_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Nil", Some("Object"))
        .construct_with("use nil")
        .sdk_instance_method("==:", |host, _receiver, args| {
            Ok(host.new_bool(args[0].is_nil()))
        })
}
