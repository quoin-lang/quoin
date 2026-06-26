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

pub use generated::quoin_ext_proto::ArrowDType;

/// A bulk numeric column — the data plane (§6/§7). Owned mirror of the `ArrowArray` table: a dtype
/// plus the contiguous little-endian value buffer (Arrow non-nullable primitive layout). `length`
/// is the element count (the host sets it; derivable from `data` for these fixed-width types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowArray {
    pub dtype: ArrowDType,
    pub length: u64,
    pub data: Vec<u8>,
}

/// A single control-plane frame, in either direction. Mirrors the `Message` union in
/// `ext.fbs`. Encode with [`encode`]; decode a received frame with [`decode_envelope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// host -> ext: invoke `op` with the scalar argument `arg`, plus typed arguments. `handles`
    /// are host-value handle ids (a block is one of these); `resources` are ext-side resource ids
    /// passed back as args; `releases` are ext-side resource ids the host dropped and the extension
    /// should free at the top of the call (the batched reap); `arrays` are bulk columns (data plane).
    Call {
        op: String,
        arg: String,
        handles: Vec<u64>,
        resources: Vec<u64>,
        releases: Vec<u64>,
        arrays: Vec<ArrowArray>,
    },
    /// ext -> host: the originating call is finished; `result` is the scalar return.
    CallReturn { result: String },
    /// ext -> host: the call returns an ext-side resource the host will hold as an opaque token
    /// (reaped on drop). `resource` is the extension-assigned id.
    CallReturnResource { resource: u64 },
    /// ext -> host: the call returns a bulk `Array` (the data plane).
    CallReturnArray { array: ArrowArray },
    /// ext -> host (re-entrant): make a host String, return a handle to it.
    MakeString { value: String },
    /// ext -> host (re-entrant): read a String-handle back into a scalar string.
    HandleToString { handle: u64 },
    /// ext -> host (re-entrant): promote a call-local handle to retained (global).
    Retain { handle: u64 },
    /// ext -> host (re-entrant): release retained handles (batched).
    Release { handles: Vec<u64> },
    /// ext -> host (re-entrant): send `selector` to the value behind `receiver` with the
    /// values behind `args`, returning a handle to the result.
    CallMethodOnHandle {
        receiver: u64,
        selector: String,
        args: Vec<u64>,
    },
    /// ext -> host (re-entrant): invoke the host block behind `block` once per tuple in
    /// `batches`, in one round-trip. Each tuple is one invocation's argument handles.
    InvokeBlock { block: u64, batches: Vec<Vec<u64>> },
    /// host -> ext: the reply to `InvokeBlock` — one result handle per tuple, or `error`.
    InvokeBlockReturn {
        results: Vec<u64>,
        error: Option<String>,
    },
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
        Msg::Call {
            op,
            arg,
            handles,
            resources,
            releases,
            arrays,
        } => g::Message::Call(Box::new(g::Call {
            op: Some(op.clone()),
            arg: Some(arg.clone()),
            handles: Some(handles.clone()),
            resources: Some(resources.clone()),
            releases: Some(releases.clone()),
            arrays: Some(arrays.iter().map(encode_arrow).collect()),
        })),
        Msg::CallReturn { result } => g::Message::CallReturn(Box::new(g::CallReturn {
            result: Some(result.clone()),
        })),
        Msg::CallReturnResource { resource } => {
            g::Message::CallReturnResource(Box::new(g::CallReturnResource {
                resource: *resource,
            }))
        }
        Msg::CallReturnArray { array } => {
            g::Message::CallReturnArray(Box::new(g::CallReturnArray {
                array: Some(Box::new(encode_arrow(array))),
            }))
        }
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
        Msg::CallMethodOnHandle {
            receiver,
            selector,
            args,
        } => g::Message::CallMethodOnHandle(Box::new(g::CallMethodOnHandle {
            receiver: *receiver,
            selector: Some(selector.clone()),
            args: Some(args.clone()),
        })),
        Msg::InvokeBlock { block, batches } => g::Message::InvokeBlock(Box::new(g::InvokeBlock {
            block: *block,
            batches: Some(
                batches
                    .iter()
                    .map(|tuple| g::HandleList {
                        handles: Some(tuple.clone()),
                    })
                    .collect(),
            ),
        })),
        Msg::InvokeBlockReturn { results, error } => {
            g::Message::InvokeBlockReturn(Box::new(g::InvokeBlockReturn {
                results: Some(results.clone()),
                error: error.clone(),
            }))
        }
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

/// Collect a planus `[uint64]` accessor result into an owned `Vec` (absent vector -> empty).
fn read_u64_vec(v: Option<planus::Vector<'_, u64>>) -> Vec<u64> {
    v.map(|vec| vec.iter().collect()).unwrap_or_default()
}

/// Owned [`ArrowArray`] -> the generated builder struct.
fn encode_arrow(a: &ArrowArray) -> g::ArrowArray {
    g::ArrowArray {
        dtype: a.dtype,
        length: a.length,
        data: Some(a.data.clone()),
    }
}

/// A decoded `ArrowArrayRef` -> owned [`ArrowArray`].
fn decode_arrow(a: g::ArrowArrayRef<'_>) -> Result<ArrowArray, planus::Error> {
    Ok(ArrowArray {
        dtype: a.dtype()?,
        length: a.length()?,
        data: a.data()?.unwrap_or_default().to_vec(),
    })
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
            handles: read_u64_vec(c.handles()?),
            resources: read_u64_vec(c.resources()?),
            releases: read_u64_vec(c.releases()?),
            arrays: {
                let mut arrays = Vec::new();
                if let Some(v) = c.arrays()? {
                    for a in v {
                        arrays.push(decode_arrow(a?)?);
                    }
                }
                arrays
            },
        },
        g::MessageRef::CallReturn(c) => Msg::CallReturn {
            result: c.result()?.unwrap_or_default().to_string(),
        },
        g::MessageRef::CallReturnResource(c) => Msg::CallReturnResource {
            resource: c.resource()?,
        },
        g::MessageRef::CallReturnArray(c) => Msg::CallReturnArray {
            array: match c.array()? {
                Some(a) => decode_arrow(a)?,
                None => ArrowArray {
                    dtype: ArrowDType::Float64,
                    length: 0,
                    data: Vec::new(),
                },
            },
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
        g::MessageRef::CallMethodOnHandle(c) => Msg::CallMethodOnHandle {
            receiver: c.receiver()?,
            selector: c.selector()?.unwrap_or_default().to_string(),
            args: match c.args()? {
                Some(v) => v.iter().collect(),
                None => Vec::new(),
            },
        },
        g::MessageRef::InvokeBlock(b) => {
            let mut batches = Vec::new();
            if let Some(v) = b.batches()? {
                for tuple in v {
                    let tuple = tuple?;
                    batches.push(match tuple.handles()? {
                        Some(hs) => hs.iter().collect(),
                        None => Vec::new(),
                    });
                }
            }
            Msg::InvokeBlock {
                block: b.block()?,
                batches,
            }
        }
        g::MessageRef::InvokeBlockReturn(r) => Msg::InvokeBlockReturn {
            results: match r.results()? {
                Some(v) => v.iter().collect(),
                None => Vec::new(),
            },
            error: r.error()?.map(str::to_string),
        },
        g::MessageRef::HostOpReturn(h) => Msg::HostOpReturn {
            handle: h.handle()?,
            str: h.str()?.map(str::to_string),
            error: h.error()?.map(str::to_string),
        },
    }))
}
