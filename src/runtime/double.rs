use crate::arg;
use crate::error::BBError;
use crate::value::{NativeClassBuilder, Value};

pub fn build_double_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Double", Some("Object"))
        .instance_method("sqrt", |vm, mc, args| {
            if args.is_empty() {
                return Err(BBError::Other("sqrt expects a receiver".to_string()));
            }
            let val = arg!(args, Double, 0);
            Ok(vm.new_double(mc, val.sqrt()))
        })
        .instance_method(
            "==:",
            |vm, mc, args| Ok(vm.new_bool(mc, args[0] == args[1])),
        )
}
