use crate::arg;
use crate::compute::{self, ComputeJob, ComputeOut};
use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult};
use crate::recv;
use crate::runtime::compress;
use crate::runtime::list::NativeListState;
use crate::value::{NativeClassBuilder, Value};
use crate::vm::VmState;

/// The `Bytes` class — immutable binary data (Stage 3a). The raw `Vec<u8>` lives in
/// `ObjectPayload::Bytes`; this is the QN-facing surface. Text crosses at the edges
/// via `String.asBytes` / `Bytes.asString`. See `docs/ASYNC_ARCH.md`.
pub fn build_bytes_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Bytes", Some("Object"))
        .construct_with("use Bytes.of: / Bytes.empty (or 'abc'.bytes)")
        // Bytes of:#(72 101 ...) -> bytes from a list of integers (each 0-255).
        .sdk_class_method("of:", |host, _receiver, args| {
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
                        return Err(QuoinError::ValueError(
                            "Bytes of: each element must be an Integer in 0..=255".to_string(),
                        ));
                    }
                    other => return Err(type_error("Integer", other)),
                }
            }
            Ok(host.new_bytes(bytes))
        })
        // Bytes empty -> a zero-length Bytes.
        .sdk_class_method("empty", |host, _receiver, _args| {
            Ok(host.new_bytes(Vec::new()))
        })
        // Bytes new -> same as `empty` (the generic fallback would mint a
        // payload-less shell; QUOIN_TODO.md).
        .sdk_class_method("new", |host, _receiver, _args| {
            Ok(host.new_bytes(Vec::new()))
        })
        // size / count -> the number of bytes.
        .sdk_instance_method("size", |host, receiver, _args| {
            Ok(host.new_int(recv!(receiver, Bytes).len() as i64))
        })
        .sdk_instance_method("count", |host, receiver, _args| {
            Ok(host.new_int(recv!(receiver, Bytes).len() as i64))
        })
        // at:i -> the byte (0..=255) at index i; out of range throws.
        .sdk_typed_instance_method("at:", &["Integer"], |host, receiver, args| {
            let b = recv!(receiver, Bytes);
            let i = arg!(args, Int, 0);
            match usize::try_from(i).ok().and_then(|i| b.get(i).copied()) {
                Some(byte) => Ok(host.new_int(byte as i64)),
                None => Err(QuoinError::IndexError {
                    index: i,
                    len: b.len() as i64,
                    msg: format!("Bytes.at:: index {} out of range (size {})", i, b.len()),
                }),
            }
        })
        // from:to: -> the slice [from, to), clamped to bounds.
        .sdk_typed_instance_method(
            "from:to:",
            &["Integer", "Integer"],
            |host, receiver, args| {
                let b = recv!(receiver, Bytes);
                let len = b.len() as i64;
                let from = arg!(args, Int, 0).clamp(0, len) as usize;
                let to = arg!(args, Int, 1).clamp(from as i64, len) as usize;
                Ok(host.new_bytes(b[from..to].to_vec()))
            },
        )
        // Concatenation: `a + b` -> `Send(a, "+:", [b])`. Bytes + Bytes only.
        .sdk_typed_instance_method("+:", &["Bytes"], |host, receiver, args| {
            let a = recv!(receiver, Bytes);
            let b = arg!(args, Bytes, 0);
            let mut out = Vec::with_capacity(a.len() + b.len());
            out.extend_from_slice(a.as_slice());
            out.extend_from_slice(b.as_slice());
            Ok(host.new_bytes(out))
        })
        // each:block -> yield each byte (as an Integer) to the block; returns receiver.
        .sdk_instance_method("each:", |host, receiver, args| {
            // Copy the bytes out first: plain data, so nothing is held across the block
            // calls (which may themselves step/park).
            let bytes = recv!(receiver, Bytes).to_vec();
            for byte in bytes {
                let v = host.new_int(byte as i64);
                // `execute_block` validates that args[0] is a block (errors otherwise).
                host.execute_block(args[0], vec![v], None)?;
            }
            Ok(receiver)
        })
        // asString -> UTF-8 decode; throws (catchable) on invalid UTF-8.
        .sdk_instance_method("asString", |host, receiver, _args| {
            let bytes = recv!(receiver, Bytes).to_vec();
            match String::from_utf8(bytes) {
                Ok(s) => Ok(host.new_string(s)),
                Err(e) => Err(QuoinError::ParseError(format!(
                    "Bytes.asString: invalid UTF-8 (valid up to byte {})",
                    e.utf8_error().valid_up_to()
                ))),
            }
        })
        // asStringLossy -> UTF-8 decode with replacement characters (never throws).
        .sdk_instance_method("asStringLossy", |host, receiver, _args| {
            let bytes = recv!(receiver, Bytes);
            Ok(host.new_string(String::from_utf8_lossy(bytes.as_slice()).into_owned()))
        })
        // Content-Encoding (de)compression — gzip + deflate (flate2/miniz_oxide) and zstd
        // decode (ruzstd), all pure Rust. Malformed input throws a catchable ParseError.
        // zstd encode is intentionally absent (no pure-Rust compressor; see compress.rs).
        // Legacy (`&mut VmState`) style rather than the SDK surface: big inputs OFFLOAD
        // to the compute pool (docs/CONCURRENCY_ARCH.md §4), which parks the task via
        // `await_io` — below `QN_COMPUTE_MIN` (or with the pool disabled) they run
        // inline exactly as before.
        .instance_method("decodeGz", |vm, mc, receiver, _args| {
            run_codec(vm, mc, receiver, "decodeGz", compress::gzip_decode)
        })
        .instance_method("encodeGz", |vm, mc, receiver, _args| {
            run_codec(vm, mc, receiver, "encodeGz", compress::gzip_encode)
        })
        .instance_method("decodeDeflate", |vm, mc, receiver, _args| {
            run_codec(vm, mc, receiver, "decodeDeflate", compress::deflate_decode)
        })
        .instance_method("encodeDeflate", |vm, mc, receiver, _args| {
            run_codec(vm, mc, receiver, "encodeDeflate", compress::deflate_encode)
        })
        .instance_method("decodeZstd", |vm, mc, receiver, _args| {
            run_codec(vm, mc, receiver, "decodeZstd", compress::zstd_decode)
        })
        // s -> the inspect string: length + a short hex preview.
        .sdk_instance_method("s", |host, receiver, _args| {
            Ok(host.new_string(format!("{}", receiver)))
        })
}

/// Run a `&[u8] -> Result<Vec<u8>, String>` codec over the receiver Bytes, returning a new
/// Bytes; a codec error becomes a catchable `ParseError` tagged with the method name.
fn run_codec<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    label: &'static str,
    f: impl Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync + 'static,
) -> Result<Value<'gc>, QuoinError> {
    // Detach the buffer FIRST: nothing borrowed (and no Gc beyond the rooted
    // receiver) is held across the park below.
    let bytes = recv!(receiver, Bytes).to_vec();
    let out = if compute::should_offload(bytes.len()) {
        // The job owns the detached buffer; the `Send + Sync` bound is what
        // enforces the eligibility rule at compile time.
        let job = ComputeJob::new(label, move || f(&bytes).map(ComputeOut::Bytes));
        match vm.await_io(IoRequest::Compute(job))? {
            IoResult::Computed(Ok(ComputeOut::Bytes(out))) => Ok(out),
            IoResult::Computed(Err(msg)) => Err(msg),
            other => {
                return Err(QuoinError::Other(format!(
                    "Bytes.{label}: unexpected compute result {other:?}"
                )));
            }
        }
    } else {
        compute::note_inline();
        f(&bytes)
    };
    match out {
        Ok(out) => Ok(vm.new_bytes(mc, out)),
        Err(msg) => Err(QuoinError::ParseError(format!("Bytes.{label}: {msg}"))),
    }
}

fn type_error(expected: &str, got: &Value) -> QuoinError {
    QuoinError::TypeError {
        expected: expected.to_string(),
        got: got.type_name().to_string(),
        msg: format!("expected a {}", expected),
    }
}
