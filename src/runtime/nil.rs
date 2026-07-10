use crate::value::NativeClassBuilder;

pub fn build_nil_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Nil", Some("Object"))
        .construct_with("use nil")
        .class_doc(
            "The class of `nil`, the single 'no value' value -- what a missing Map key or \
             a search that finds nothing returns. nil is not a Boolean and does not \
             answer `if:else:`; test for it with `== nil` or `defined?` (which is false \
             only for nil).\n\n\
             ```\n\
             nil == nil       \"* -> true\n\
             nil.defined?     \"* -> false\n\
             ```",
        )
        .sdk_instance_method("==:", |host, _receiver, args| {
            Ok(host.new_bool(args[0].is_nil()))
        })
        .doc(
            "Whether the argument is also nil -- nil equals only itself.\n\n\
             ```\n\
             nil == 5     \"* -> false\n\
             ```",
        )
}
