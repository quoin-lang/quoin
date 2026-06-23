use crate::arg;
use crate::error::QuoinError;
use crate::recv;
use crate::runtime::list::NativeListState;
use crate::value::{NativeClassBuilder, Value};

/// The `Bytes` class — immutable binary data (Stage 3a). The raw `Vec<u8>` lives in
/// `ObjectPayload::Bytes`; this is the QN-facing surface. Text crosses at the edges
/// via `String.asBytes` / `Bytes.asString`. See `docs/ASYNC_ARCH.md`.
pub fn build_bytes_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Bytes", Some("Object"))
        // Bytes of:#(72 101 ...) -> bytes from a list of integers (each 0-255).
        .class_method("of:", |vm, mc, _receiver, args| {
            let list_val = *args
                .first()
                .ok_or_else(|| QuoinError::Other("Bytes of: expects a list".to_string()))?;
            let elems = list_val
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
                .map_err(|_| type_error("List", &list_val))?;
            let mut bytes = Vec::with_capacity(elems.len());
            for el in &elems {
                match el {
                    Value::Int(i) if (0..=255).contains(i) => bytes.push(*i as u8),
                    Value::Int(_) => {
                        return Err(QuoinError::Other(
                            "Bytes of: each element must be an Integer in 0..=255".to_string(),
                        ));
                    }
                    other => return Err(type_error("Integer", other)),
                }
            }
            Ok(vm.new_bytes(mc, bytes))
        })
        // Bytes empty -> a zero-length Bytes.
        .class_method("empty", |vm, mc, _receiver, _args| {
            Ok(vm.new_bytes(mc, Vec::new()))
        })
        // size / count -> the number of bytes.
        .instance_method("size", |vm, mc, receiver, _args| {
            Ok(vm.new_int(mc, recv!(receiver, Bytes).len() as i64))
        })
        .instance_method("count", |vm, mc, receiver, _args| {
            Ok(vm.new_int(mc, recv!(receiver, Bytes).len() as i64))
        })
        // at:i -> the byte (0..=255) at index i; out of range throws.
        .typed_instance_method("at:", &["Integer"], |vm, mc, receiver, args| {
            let b = recv!(receiver, Bytes);
            let i = arg!(args, Int, 0);
            match usize::try_from(i).ok().and_then(|i| b.get(i).copied()) {
                Some(byte) => Ok(vm.new_int(mc, byte as i64)),
                None => Err(QuoinError::IndexError {
                    index: i,
                    len: b.len() as i64,
                    msg: format!("Bytes.at:: index {} out of range (size {})", i, b.len()),
                }),
            }
        })
        // from:to: -> the slice [from, to), clamped to bounds.
        .typed_instance_method(
            "from:to:",
            &["Integer", "Integer"],
            |vm, mc, receiver, args| {
                let b = recv!(receiver, Bytes);
                let len = b.len() as i64;
                let from = arg!(args, Int, 0).clamp(0, len) as usize;
                let to = arg!(args, Int, 1).clamp(from as i64, len) as usize;
                Ok(vm.new_bytes(mc, b[from..to].to_vec()))
            },
        )
        // Concatenation: `a + b` -> `Send(a, "+:", [b])`. Bytes + Bytes only.
        .typed_instance_method("+:", &["Bytes"], |vm, mc, receiver, args| {
            let a = recv!(receiver, Bytes);
            let b = arg!(args, Bytes, 0);
            let mut out = Vec::with_capacity(a.len() + b.len());
            out.extend_from_slice(a.as_slice());
            out.extend_from_slice(b.as_slice());
            Ok(vm.new_bytes(mc, out))
        })
        // each:block -> yield each byte (as an Integer) to the block; returns receiver.
        .instance_method("each:", |vm, mc, receiver, args| {
            // Copy the bytes out first: plain data, so nothing is held across the block
            // calls (which may themselves step/park).
            let bytes = recv!(receiver, Bytes).to_vec();
            let block = arg!(args, Block, 0);
            for byte in bytes {
                let v = vm.new_int(mc, byte as i64);
                vm.execute_block(mc, block, vec![v], None)?;
            }
            Ok(receiver)
        })
        // asString -> UTF-8 decode; throws (catchable) on invalid UTF-8.
        .instance_method("asString", |vm, mc, receiver, _args| {
            let bytes = recv!(receiver, Bytes).to_vec();
            match String::from_utf8(bytes) {
                Ok(s) => Ok(vm.new_string(mc, s)),
                Err(e) => {
                    let msg = vm.new_string(
                        mc,
                        format!(
                            "Bytes.asString: invalid UTF-8 (valid up to byte {})",
                            e.utf8_error().valid_up_to()
                        ),
                    );
                    vm.active_exception = Some(msg);
                    Err(QuoinError::Thrown)
                }
            }
        })
        // asStringLossy -> UTF-8 decode with replacement characters (never throws).
        .instance_method("asStringLossy", |vm, mc, receiver, _args| {
            let bytes = recv!(receiver, Bytes);
            Ok(vm.new_string(mc, String::from_utf8_lossy(bytes.as_slice()).into_owned()))
        })
        // s -> the inspect string: length + a short hex preview.
        .instance_method("s", |vm, mc, receiver, _args| {
            Ok(vm.new_string(mc, format!("{}", receiver)))
        })
}

fn type_error(expected: &str, got: &Value) -> QuoinError {
    QuoinError::TypeError {
        expected: expected.to_string(),
        got: got.type_name().to_string(),
        msg: format!("expected a {}", expected),
    }
}
