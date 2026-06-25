use crate::arg;
use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// `Base64` — encode/decode between `Bytes` and a base64 `String` (standard alphabet, padded).
pub fn build_base64_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Base64", Some("Object"))
        .typed_class_method("encode:", &["Bytes"], |vm, mc, _r, args| {
            Ok(vm.new_string(mc, BASE64.encode(arg!(args, Bytes, 0).to_vec())))
        })
        .typed_class_method("decode:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match BASE64.decode(s.as_str()) {
                Ok(bytes) => Ok(vm.new_bytes(mc, bytes)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "Base64.decode:: not valid base64: '{}'",
                    s.as_str()
                ))),
            }
        })
}

/// `Hex` — encode/decode between `Bytes` and a hex `String` (lower-case on encode,
/// case-insensitive on decode).
pub fn build_hex_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Hex", Some("Object"))
        .typed_class_method("encode:", &["Bytes"], |vm, mc, _r, args| {
            Ok(vm.new_string(mc, hex::encode(arg!(args, Bytes, 0).to_vec())))
        })
        .typed_class_method("decode:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match hex::decode(s.as_str().trim()) {
                Ok(bytes) => Ok(vm.new_bytes(mc, bytes)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "Hex.decode:: not valid hex: '{}'",
                    s.as_str()
                ))),
            }
        })
}
