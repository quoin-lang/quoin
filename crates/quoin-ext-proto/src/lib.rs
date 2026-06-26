//! Shared FlatBuffers control-plane types for the Quoin out-of-process extension
//! protocol (Tier 1; see `docs/FUTURE_EXT_ARCH.md`). The canonical schema is
//! `schema/ext.fbs`; the Rust bindings in `generated.rs` are produced with planus
//! (pure-Rust, no `flatc`):
//!
//! ```text
//! planus rust -o crates/quoin-ext-proto/src/generated.rs crates/quoin-ext-proto/schema/ext.fbs
//! ```
//!
//! Both the VM (host) and the `quoin-ext` extension SDK depend on this crate and use
//! the `encode_*` / `decode_*` helpers, so the planus API lives in exactly one place.
//! Other-language SDKs code-generate from the same `.fbs` independently.

mod generated;

pub use generated::quoin_ext_proto::{Request, RequestRef, Response, ResponseRef};

use planus::{Builder, ReadAsRoot};

/// Encode a `Request` (op name + scalar argument) as a FlatBuffers buffer.
pub fn encode_request(op: &str, arg: &str) -> Vec<u8> {
    let mut builder = Builder::new();
    let req = Request {
        op: Some(op.to_string()),
        arg: Some(arg.to_string()),
    };
    builder.finish(&req, None).to_vec()
}

/// Decode a `Request` buffer into `(op, arg)`.
pub fn decode_request(bytes: &[u8]) -> Result<(String, String), planus::Error> {
    let r = RequestRef::read_as_root(bytes)?;
    Ok((
        r.op()?.unwrap_or_default().to_string(),
        r.arg()?.unwrap_or_default().to_string(),
    ))
}

/// Encode a `Response` (scalar result) as a FlatBuffers buffer.
pub fn encode_response(result: &str) -> Vec<u8> {
    let mut builder = Builder::new();
    let resp = Response {
        result: Some(result.to_string()),
    };
    builder.finish(&resp, None).to_vec()
}

/// Decode a `Response` buffer into its result string.
pub fn decode_response(bytes: &[u8]) -> Result<String, planus::Error> {
    let r = ResponseRef::read_as_root(bytes)?;
    Ok(r.result()?.unwrap_or_default().to_string())
}
