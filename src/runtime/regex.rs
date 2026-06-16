use crate::value::NativeClassBuilder;

pub fn build_regex_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Regex", Some("Object"))
}
