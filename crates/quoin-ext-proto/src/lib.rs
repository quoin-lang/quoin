//! The Quoin out-of-process extension protocol (Tier 1; see `docs/FUTURE_EXT_ARCH.md`).
//!
//! The whole wire is MessagePack: one frame is one MessagePack array `[type, field, ...]`,
//! length-prefixed on the unix-domain socket with a little-endian `u32`. A structured value
//! has exactly one encoding — there is no separate envelope format and no negotiated
//! alternate payload representation. The codec is hand-rolled and dependency-free
//! (`codec.rs`); the byte-level contract an other-language SDK implements is `PROTOCOL.md`
//! next to this crate's `Cargo.toml` — any language with a MessagePack library (or two
//! hundred lines of patience) can speak it.
//!
//! Both the VM (host) and the `quoin-ext` extension SDK depend on this crate and talk
//! through the owned [`Msg`] enum plus [`encode`] / [`decode_frame`]: one frame is one
//! `Msg`, and the direction (host->ext vs ext->host) is implicit in which side reads it.
//! A host->ext `Call` may be answered directly with a `CallReturn*` terminal, or the
//! extension may interleave re-entrant host-op requests (each answered by the host
//! mid-call) before its terminal reply.

pub mod codec;

pub use codec::{decode_frame, encode, pack_dv, unpack_dv};

/// The protocol version spoken by this crate, exchanged in [`Msg::GetManifest`] /
/// [`Msg::ManifestReturn`] (the first frames on a fresh connection, so a mismatch is
/// caught before anything else crosses). Version 1 was the retired FlatBuffers wire;
/// bump on any change an existing decoder cannot skip (appending fields to a message
/// or adding message types does NOT require a bump — see `PROTOCOL.md` §Evolution).
pub const PROTOCOL_VERSION: u32 = 2;

/// The element type of a bulk [`ArrowArray`] column. The `u8` values are the wire tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrowDType {
    Float64 = 0,
    Int64 = 1,
}

/// A bulk numeric column — the data plane (§6/§7): a dtype plus the contiguous
/// little-endian value buffer (Arrow non-nullable primitive layout). `length` is the
/// element count (derivable from `data` for these fixed-width types, carried for Arrow
/// C-Data-Interface forward compat). Validity bitmaps, var-width buffers, and in-place
/// fd-sharing are later steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowArray {
    pub dtype: ArrowDType,
    pub length: u64,
    pub data: Vec<u8>,
}

/// A structured value tree — the wire mirror of the host `DataValue` (Phase 1), so an
/// extension can exchange arbitrary nil/bool/int/float/str/bytes/list/map data that
/// materializes as nested Quoin Values. Arbitrary-precision `BigInt`/`Decimal` travel as
/// their decimal-string form (MessagePack ext types 1 and 2).
#[derive(Debug, Clone, PartialEq)]
pub enum DataValue {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(String),
    Float(f64),
    Decimal(String),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<DataValue>),
    Map(Vec<(String, DataValue)>),
    /// A live extension instance carried *inside* a structured value (MessagePack ext type 3):
    /// `id` is the instance's ext-side object-table key. Ext -> host, `class_name` names the
    /// registered extension-backed class so the host wraps the id as the right installed class
    /// (a method can return e.g. a List of instances); host -> ext, `class_name` is empty — the
    /// extension resolves `id` in its own table. Only meaningful between a host and the one
    /// extension that owns the ids; the host refuses to send another extension's instance.
    Resource {
        id: u64,
        class_name: String,
    },
}

/// One extension-provided class (Phase 3), as declared in a [`Msg::ManifestReturn`]. The host
/// installs a real Quoin class named `name`; each selector becomes a method that dispatches over
/// the socket — `instance_selectors` on instances, `class_selectors` on the class itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDecl {
    pub name: String,
    pub instance_selectors: Vec<String>,
    pub class_selectors: Vec<String>,
}

/// One ordered method argument for an extension-backed-class send (Phase 3 — `Call.method_args`).
/// `Data` is an inline structured value; `Resource` is an ext-instance's object-table id (so a
/// method can take another of the extension's objects); `Handle` is a host-value handle for a block
/// or other non-data host object the extension drives via `invoke_block` / `call_method`; `Array`
/// is a bulk numeric column (the data plane, inline — so an extension-backed-class method can take
/// a host `Array` without exploding it into per-element values).
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Data(DataValue),
    Resource(u64),
    Handle(u64),
    Array(ArrowArray),
}

/// A single control-plane frame, in either direction. Encode with [`encode`]; decode a
/// received frame with [`decode_frame`]. The wire layout of each variant is `PROTOCOL.md`'s
/// message table. (No `Eq` — `DataValue` carries `f64`.)
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// host -> ext: invoke `op` with the scalar argument `arg`, plus typed arguments. `handles`
    /// are host-value handle ids (a block is one of these); `resources` are ext-side resource ids
    /// passed back as args; `releases` are ext-side resource ids the host dropped and the extension
    /// should free at the top of the call (the batched reap); `arrays` are bulk columns (data plane);
    /// `data` is an optional structured-value payload (Phase 1).
    Call {
        op: String,
        arg: String,
        handles: Vec<u64>,
        resources: Vec<u64>,
        releases: Vec<u64>,
        arrays: Vec<ArrowArray>,
        data: Option<DataValue>,
        /// Extension-backed classes (Phase 3): names the class a method send dispatches to (empty
        /// for the legacy generic path); `recv` is the instance's ext-side resource id (0 =
        /// class-side). The method's ordered arguments travel in `method_args`.
        class_name: String,
        recv: u64,
        method_args: Vec<Arg>,
    },
    /// ext -> host: the originating call is finished; `result` is the scalar return.
    CallReturn { result: String },
    /// ext -> host: the call failed with a recoverable error (`message`) — the host raises a
    /// catchable Quoin error and the extension stays alive.
    CallReturnError { message: String },
    /// ext -> host: the call returns an ext-side resource the host will hold as an opaque token
    /// (reaped on drop). `resource` is the extension-assigned id; `class_name` names the registered
    /// extension-backed class it's an instance of (Phase 3; empty = the opaque `ExtResource`).
    CallReturnResource { resource: u64, class_name: String },
    /// ext -> host: the call returns a bulk `Array` (the data plane).
    CallReturnArray { array: ArrowArray },
    /// ext -> host: the call returns a structured value (materialized as a nested Quoin Value).
    CallReturnData { value: DataValue },
    /// ext -> host: the call returns a live host value (the host resolves the handle to its value).
    CallReturnHandle { handle: u64 },
    /// host -> ext: sent once right after connect — asks the extension which classes it provides.
    /// `version` is the host's [`PROTOCOL_VERSION`]; an SDK that speaks a different version must
    /// refuse with a clear error naming both versions, not misdecode.
    GetManifest { version: u32 },
    /// ext -> host: the reply to `GetManifest`; the extension's provided classes (empty if none)
    /// plus the SDK's own protocol version — the host refuses a mismatch with a clear error.
    ManifestReturn {
        classes: Vec<ClassDecl>,
        version: u32,
    },
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
    /// ext -> host (re-entrant): resolve a name in the host's globals (Phase 2 — host reach),
    /// returning a handle to its value (`HostOpReturn`).
    GetGlobal { name: String },
    /// ext -> host (re-entrant): construct any host value from a `DataValue`, returning a handle.
    MakeValue { value: DataValue },
    /// ext -> host (re-entrant): project the value behind `handle` to a `DataValue`.
    ReadHandle { handle: u64 },
    /// host -> ext: the reply to `ReadHandle` — the projected value, or `error`.
    ReadHandleReturn {
        value: DataValue,
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

/// Upper bound on one length-prefixed frame's payload, enforced by both ends before
/// allocating toward the declared length. The u32 prefix alone permits ~4 GiB, so a
/// corrupt or hostile length would otherwise drive a multi-GB allocation / OOM from a
/// few bytes. 256 MiB is far above any realistic control frame or bulk Arrow column
/// yet bounds the blast radius of a bad prefix. Shared so host and SDK agree.
pub const MAX_FRAME_LEN: usize = 256 * 1024 * 1024;

/// Hard cap on MessagePack nesting depth accepted from a peer. The peer runs on the same
/// machine but is a separate process that can crash or be buggy/malicious; the decoder
/// recurses per level, so without a bound a deeply nested value from the extension
/// overflows the *host* stack — an uncatchable process abort that would defeat the whole
/// point of out-of-process isolation. Real payloads (DB rows, JSON, expression graphs)
/// are shallow; 64 is above any legitimate structure yet low enough that decoding to the
/// cap stays well within the 1 MiB VM coroutine stack (each decode frame is heavy, so the
/// bound must be conservative, not just finite).
const MAX_DV_DEPTH: usize = 64;
