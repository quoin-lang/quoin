use crate::value::NativeClassBuilder;

pub fn build_nil_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Nil", Some("Object"))
}
