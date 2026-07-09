use crate::recv;
use crate::value::{NativeClassBuilder, Value};

pub fn build_boolean_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Boolean", Some("Object"))
        .construct_with("use the literals true / false")
        //
        .sdk_instance_method("s", |host, receiver, _args| {
            let b = recv!(receiver, Bool);
            Ok(host.new_string(if b {
                "true".to_string()
            } else {
                "false".to_string()
            }))
        })
        .sdk_instance_method("==:", |host, receiver, args| {
            Ok(host.new_bool(receiver == args[0]))
        })
}
