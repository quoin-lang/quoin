use crate::arg;
use crate::error::QuoinError;
use crate::runtime::data_value::{DataValue, data_to_value, value_to_data};
use crate::value::{NativeClassBuilder, Value};

/// `MessagePack` — compact binary serialization over the `DataValue` bridge. Unlike JSON it has a
/// native bytes type, so `Bytes` round-trips. Numbers beyond the 64-bit range (a large
/// `BigInteger`) and any `BigDecimal` serialize as their exact digits in a string (see DataValue).
pub fn build_message_pack_class() -> NativeClassBuilder {
    NativeClassBuilder::new("MessagePack", Some("Object"))
        .abstract_class()
        .class_doc(
            "Compact binary serialization: `pack:` a value to `Bytes`, `unpack:` it back.\n\n\
             The same value mapping as `JSON` — and unlike JSON, MessagePack has a native \
             bytes type, so `Bytes` round-trips without a Base64 detour. Numbers beyond the \
             64-bit range (a large BigInteger) and any BigDecimal serialize as their exact \
             digits in a string.\n\n\
             ```\n\
             MessagePack.pack:#{'a': 1}                        \"* -> Bytes[4] 81 a1 61 01\n\
             MessagePack.unpack:(MessagePack.pack:#{'a': 1})   \"* -> #{'a': 1}\n\
             ```",
        )
        // MessagePack.pack:value -> Bytes.
        .class_method("pack:", |vm, mc, _r, args| {
            let data = value_to_data(vm, mc, args[0])?;
            let bytes = rmp_serde::to_vec(&data)
                .map_err(|e| QuoinError::Other(format!("MessagePack.pack:: {e}")))?;
            Ok(vm.new_bytes(mc, bytes))
        })
        .doc(
            "Serialize a value (Maps, Lists, Strings, numbers, Booleans, Bytes, nil) to \
             MessagePack Bytes.\n\n\
             ```\n\
             MessagePack.pack:#(1 2 3)     \"* -> Bytes[4] 93 01 02 03\n\
             ```",
        )
        // MessagePack.unpack:bytes -> a Quoin value.
        .sdk_typed_class_method("unpack:", &["Bytes"], |host, _r, args| {
            let bytes = arg!(args, Bytes, 0).to_vec();
            let data: DataValue = rmp_serde::from_slice(&bytes)
                .map_err(|e| QuoinError::ParseError(format!("MessagePack.unpack:: {e}")))?;
            data_to_value(&data, host)
        })
        .doc(
            "Deserialize MessagePack Bytes back into Quoin values — the inverse of `pack:`. \
             Malformed input throws a ParseError.\n\n\
             ```\n\
             (MessagePack.unpack:(MessagePack.pack:#{'a': 1})) == #{'a': 1}     \"* -> true\n\
             ```",
        )
}
