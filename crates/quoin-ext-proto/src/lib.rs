//! Shared FlatBuffers control-plane types for the Quoin out-of-process extension
//! protocol (Tier 1; see `docs/FUTURE_EXT_ARCH.md`). The canonical schema is
//! `schema/ext.fbs`; the Rust bindings in `generated.rs` are produced with planus
//! (pure-Rust, no `flatc`):
//!
//! ```text
//! planus rust -o crates/quoin-ext-proto/src/generated.rs crates/quoin-ext-proto/schema/ext.fbs
//! ```
//!
//! Both the VM (host) and the `quoin-ext` extension SDK depend on this crate. Rather
//! than touch the planus accessor API directly, they use the owned [`Msg`] enum plus
//! [`encode`] / [`decode_envelope`]: one frame is one `Msg`, and the direction (host->ext
//! vs ext->host) is implicit in which side reads it. Other-language SDKs code-generate
//! from the same `.fbs` independently.

// planus emits a full builder API (the `*Builder` structs) alongside the create API we
// use; the unused half trips dead-code lints. It is machine-generated — don't hand-edit.
#[allow(dead_code, unused)]
mod generated;

use generated::quoin_ext_proto as g;
use planus::{Builder, ReadAsRoot};

/// A single control-plane frame, in either direction. Mirrors the `Message` union in
/// `ext.fbs`. Encode with [`encode`]; decode a received frame with [`decode_envelope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// host -> ext: invoke `op` with the scalar argument `arg`.
    Call { op: String, arg: String },
    /// ext -> host: the originating call is finished; `result` is the scalar return.
    CallReturn { result: String },
    /// ext -> host (re-entrant): make a host String, return a handle to it.
    MakeString { value: String },
    /// ext -> host (re-entrant): read a String-handle back into a scalar string.
    HandleToString { handle: u64 },
    /// ext -> host (re-entrant): promote a call-local handle to retained (global).
    Retain { handle: u64 },
    /// ext -> host (re-entrant): release retained handles (batched).
    Release { handles: Vec<u64> },
    /// host -> ext: the reply to any re-entrant host-op. `handle` is set for `MakeString`,
    /// `str` for `HandleToString`, neither for an ack; `error` is `Some` iff the op failed.
    HostOpReturn {
        handle: u64,
        str: Option<String>,
        error: Option<String>,
    },
}

/// Encode one `Msg` as a complete FlatBuffers `Envelope` buffer (no length prefix —
/// the transport frames it).
pub fn encode(msg: &Msg) -> Vec<u8> {
    let message = match msg {
        Msg::Call { op, arg } => g::Message::Call(Box::new(g::Call {
            op: Some(op.clone()),
            arg: Some(arg.clone()),
        })),
        Msg::CallReturn { result } => g::Message::CallReturn(Box::new(g::CallReturn {
            result: Some(result.clone()),
        })),
        Msg::MakeString { value } => g::Message::MakeString(Box::new(g::MakeString {
            value: Some(value.clone()),
        })),
        Msg::HandleToString { handle } => {
            g::Message::HandleToString(Box::new(g::HandleToString { handle: *handle }))
        }
        Msg::Retain { handle } => g::Message::Retain(Box::new(g::Retain { handle: *handle })),
        Msg::Release { handles } => g::Message::Release(Box::new(g::Release {
            handles: Some(handles.clone()),
        })),
        Msg::HostOpReturn { handle, str, error } => {
            g::Message::HostOpReturn(Box::new(g::HostOpReturn {
                handle: *handle,
                str: str.clone(),
                error: error.clone(),
            }))
        }
    };
    let mut builder = Builder::new();
    let env = g::Envelope { msg: Some(message) };
    builder.finish(&env, None).to_vec()
}

/// Decode one received `Envelope` frame into an owned [`Msg`]. The error is a
/// human-readable string (a malformed buffer or an `Envelope` with no `msg`); both the
/// host and the extension SDK wrap it in their own error type.
pub fn decode_envelope(bytes: &[u8]) -> Result<Msg, String> {
    decode_inner(bytes)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "extension protocol: Envelope had no `msg`".to_string())
}

/// The planus-fallible core of [`decode_envelope`]; `Ok(None)` means the `msg` union was
/// absent (kept separate so the accessor `?`s stay on `planus::Error`).
fn decode_inner(bytes: &[u8]) -> Result<Option<Msg>, planus::Error> {
    let env = g::EnvelopeRef::read_as_root(bytes)?;
    let Some(msg) = env.msg()? else {
        return Ok(None);
    };
    Ok(Some(match msg {
        g::MessageRef::Call(c) => Msg::Call {
            op: c.op()?.unwrap_or_default().to_string(),
            arg: c.arg()?.unwrap_or_default().to_string(),
        },
        g::MessageRef::CallReturn(c) => Msg::CallReturn {
            result: c.result()?.unwrap_or_default().to_string(),
        },
        g::MessageRef::MakeString(m) => Msg::MakeString {
            value: m.value()?.unwrap_or_default().to_string(),
        },
        g::MessageRef::HandleToString(h) => Msg::HandleToString {
            handle: h.handle()?,
        },
        g::MessageRef::Retain(r) => Msg::Retain {
            handle: r.handle()?,
        },
        g::MessageRef::Release(r) => Msg::Release {
            handles: match r.handles()? {
                Some(v) => v.iter().collect(),
                None => Vec::new(),
            },
        },
        g::MessageRef::HostOpReturn(h) => Msg::HostOpReturn {
            handle: h.handle()?,
            str: h.str()?.map(str::to_string),
            error: h.error()?.map(str::to_string),
        },
    }))
}
