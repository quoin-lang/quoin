use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

/// The interned name of a symbol value, or `None` if `val` isn't a symbol.
pub fn symbol_name(val: Value<'_>) -> Option<String> {
    if let Value::Object(obj) = val
        && let ObjectPayload::Symbol(s) = &obj.borrow().payload
    {
        Some((**s).clone())
    } else {
        None
    }
}

pub fn build_symbol_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Symbol", Some("Object"))
        // The plain name, without the leading `#` (e.g. `#foo.s` -> 'foo').
        .instance_method("s", |vm, mc, receiver, _args| {
            let name = symbol_name(receiver)
                .ok_or_else(|| QuoinError::Other("Symbol#s on a non-symbol".to_string()))?;
            Ok(vm.new_string(mc, name))
        })
        .instance_method("asString", |vm, mc, receiver, _args| {
            let name = symbol_name(receiver)
                .ok_or_else(|| QuoinError::Other("Symbol#asString on a non-symbol".to_string()))?;
            Ok(vm.new_string(mc, name))
        })
        .instance_method("asSymbol", |_vm, _mc, receiver, _args| Ok(receiver))
        // Symbols are interned, so equality is identity (handled by Value::eq).
        .instance_method("==:", |vm, mc, receiver, args| Ok(vm.new_bool(mc, receiver == args[0])))
}
