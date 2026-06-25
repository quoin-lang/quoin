use crate::arg;
use crate::error::QuoinError;
use crate::runtime::data_value::{DataValue, data_to_value, value_to_data};
use crate::value::{NativeClassBuilder, Value};

/// `MessagePack` — compact binary serialization over the `DataValue` bridge. Unlike JSON it has a
/// native bytes type, so `Bytes` round-trips. Numbers beyond the 64-bit range (a large
/// `BigInteger`) and any `BigDecimal` serialize as their exact digits in a string (see DataValue).
pub fn build_message_pack_class() -> NativeClassBuilder {
    NativeClassBuilder::new("MessagePack", Some("Object"))
        // MessagePack.pack:value -> Bytes.
        .class_method("pack:", |vm, mc, _r, args| {
            let data = value_to_data(args[0])?;
            let bytes = rmp_serde::to_vec(&data)
                .map_err(|e| QuoinError::Other(format!("MessagePack.pack:: {e}")))?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // MessagePack.unpack:bytes -> a Quoin value.
        .typed_class_method("unpack:", &["Bytes"], |vm, mc, _r, args| {
            let bytes = arg!(args, Bytes, 0).to_vec();
            let data: DataValue = rmp_serde::from_slice(&bytes)
                .map_err(|e| QuoinError::ParseError(format!("MessagePack.unpack:: {e}")))?;
            data_to_value(&data, vm, mc)
        })
}
