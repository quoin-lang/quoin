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
        .construct_with("use symbol literals (#name)")
        .class_doc(
            "An interned identifier -- the type of `#name` literals (`#'quoted form'` for \
             names with spaces or punctuation). Two symbols with the same name are the \
             same object, so comparing them is a cheap identity check. Symbols name \
             things -- selectors, classes, keys -- where a String would carry text.\n\n\
             ```\n\
             #foo == #foo     \"* -> true\n\
             #foo.s           \"* -> foo\n\
             ```",
        )
        // The plain name, without the leading `#` (e.g. `#foo.s` -> 'foo').
        .sdk_instance_method("s", |host, receiver, _args| {
            let name = symbol_name(receiver)
                .ok_or_else(|| QuoinError::Other("Symbol#s on a non-symbol".to_string()))?;
            Ok(host.new_string(name))
        })
        .doc(
            "The plain name as a String, without the leading `#`.\n\n\
             ```\n\
             #foo.s     \"* -> foo\n\
             ```",
        )
        .sdk_instance_method("asString", |host, receiver, _args| {
            let name = symbol_name(receiver)
                .ok_or_else(|| QuoinError::Other("Symbol#asString on a non-symbol".to_string()))?;
            Ok(host.new_string(name))
        })
        .doc("The name as a String -- the same text `s` returns.")
        .sdk_instance_method("asSymbol", |_host, receiver, _args| Ok(receiver))
        .doc(
            "The receiver itself: a Symbol is already a Symbol, so code can normalize a \
             name-or-symbol value without checking its type first.",
        )
        // Symbols are interned, so equality is identity (handled by Value::eq).
        .sdk_instance_method("==:", |host, receiver, args| {
            Ok(host.new_bool(receiver == args[0]))
        })
        .doc(
            "Whether the argument is the same symbol. Symbols are interned, so this is an \
             identity comparison; a non-Symbol is simply unequal.\n\n\
             ```\n\
             #foo == #bar     \"* -> false\n\
             ```",
        )
}
