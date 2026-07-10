use crate::recv;
use crate::value::{NativeClassBuilder, Value};

pub fn build_boolean_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Boolean", Some("Object"))
        .construct_with("use the literals true / false")
        .class_doc(
            "The two truth values, written as the literals `true` and `false`. Control \
             flow in Quoin is messaging a Boolean: `if:`/`else:` take blocks and run the \
             matching one, so there is no statement-level if.\n\n\
             ```\n\
             (1 < 2).if:{ 'yes' } else:{ 'no' }     \"* -> yes\n\
             ```\n\n\
             Only `true` and `false` are Booleans -- nil is not a Boolean and does not \
             answer `if:`.",
        )
        //
        .sdk_instance_method("s", |host, receiver, _args| {
            let b = recv!(receiver, Bool);
            Ok(host.new_string(if b {
                "true".to_string()
            } else {
                "false".to_string()
            }))
        })
        .doc(
            "'true' or 'false'.\n\n\
             ```\n\
             false.s     \"* -> false\n\
             ```",
        )
        .sdk_instance_method("==:", |host, receiver, args| {
            Ok(host.new_bool(receiver == args[0]))
        })
        .doc(
            "Whether the argument is the same truth value; a non-Boolean is simply \
             unequal, never an error.\n\n\
             ```\n\
             true == false     \"* -> false\n\
             ```",
        )
}
