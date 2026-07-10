use crate::arg;
use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// `Base64` — encode/decode between `Bytes` and a base64 `String` (standard alphabet, padded).
pub fn build_base64_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Base64", Some("Object"))
        .abstract_class()
        .class_doc(
            "Base64 encoding between `Bytes` and text — the standard alphabet, with `=` \
             padding.\n\n\
             The way to carry binary data through text-only formats (JSON has no bytes \
             type). `String#toBase64` / `String#fromBase64` wrap these for the common \
             string-to-string round trip.\n\n\
             ```\n\
             Base64.encode:'hello'.asBytes            \"* -> 'aGVsbG8='\n\
             (Base64.decode:'aGVsbG8=').asString      \"* -> 'hello'\n\
             ```",
        )
        .sdk_typed_class_method("encode:", &["Bytes"], |host, _r, args| {
            Ok(host.new_string(BASE64.encode(arg!(args, Bytes, 0).to_vec())))
        })
        .doc(
            "The base64 String for some Bytes (standard alphabet, padded).\n\n\
             ```\n\
             Base64.encode:'hello'.asBytes     \"* -> 'aGVsbG8='\n\
             ```",
        )
        .sdk_typed_class_method("decode:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match BASE64.decode(s.as_str()) {
                Ok(bytes) => Ok(host.new_bytes(bytes)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "Base64.decode:: not valid base64: '{}'",
                    s.as_str()
                ))),
            }
        })
        .doc(
            "The Bytes a base64 String encodes — the inverse of `encode:`. Input that is not \
             valid base64 throws a ValueError.\n\n\
             ```\n\
             Base64.decode:'aGVsbG8='     \"* -> Bytes[5] 68 65 6c 6c 6f\n\
             ```",
        )
}

/// `Hex` — encode/decode between `Bytes` and a hex `String` (lower-case on encode,
/// case-insensitive on decode).
pub fn build_hex_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Hex", Some("Object"))
        .abstract_class()
        .class_doc(
            "Hexadecimal encoding between `Bytes` and text: two hex digits per byte, \
             lower-case on encode, either case accepted on decode.\n\n\
             `Bytes#toHex` / `String#fromHex` wrap these as instance-side conveniences.\n\n\
             ```\n\
             Hex.encode:'hi'.asBytes            \"* -> '6869'\n\
             (Hex.decode:'6869').asString       \"* -> 'hi'\n\
             ```",
        )
        .sdk_typed_class_method("encode:", &["Bytes"], |host, _r, args| {
            Ok(host.new_string(hex::encode(arg!(args, Bytes, 0).to_vec())))
        })
        .doc(
            "The lower-case hex String for some Bytes (two digits per byte).\n\n\
             ```\n\
             Hex.encode:'hi'.asBytes     \"* -> '6869'\n\
             ```",
        )
        .sdk_typed_class_method("decode:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match hex::decode(s.as_str().trim()) {
                Ok(bytes) => Ok(host.new_bytes(bytes)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "Hex.decode:: not valid hex: '{}'",
                    s.as_str()
                ))),
            }
        })
        .doc(
            "The Bytes a hex String encodes — case-insensitive, surrounding whitespace \
             ignored. Anything else throws a ValueError.\n\n\
             ```\n\
             Hex.decode:'6869'     \"* -> Bytes[2] 68 69\n\
             ```",
        )
}
