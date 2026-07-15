//! quoin-ext — the **extension-side** SDK for out-of-process Quoin extensions
//! (Tier 1 of the extension architecture; see `docs/internal/FUTURE_EXT_ARCH.md`).
//!
//! An extension is a separate process the Quoin VM spawns and talks to over a unix
//! domain socket. This crate is the thin per-language client an extension links
//! against — it is **not** linked into the VM. (The VM-side host API is the
//! separate in-process `ext_sdk` surface.)
//!
//! ## Wire protocol
//!
//! Messages are length-prefixed frames: a little-endian `u32` length followed by that many
//! payload bytes. The payload is one MessagePack array (codec + `PROTOCOL.md` contract in the
//! shared `quoin-ext-proto` crate). A `Call` carries typed handle args — host-value handles via
//! [`Host::handles`] (a block is one of these) and ext-side resource ids via [`Host::resources`].
//! The handler may issue **re-entrant host-ops** through the [`Host`] client — `make_string`,
//! `handle_to_string`, `retain`, `release`, `call_method` (send a Quoin message to a handle),
//! `invoke_block` (run a host block over a batch in one round-trip), and the host-reach ops
//! `get_global` (reach a host class/global), `make_value` (build any host value from a
//! [`DataValue`]), and `read_handle` (project a handle to a [`DataValue`]) — each a synchronous
//! round-trip the host services while parked on the reply. It returns a [`Reply`]: a scalar string,
//! an ext-side **resource** (reaped via [`Host::releases`] on a later call), an [`ArrowArray`] (the
//! bulk data plane, copy-through), a structured [`DataValue`] tree, or a live host [`Handle`].
//! Structured args arrive via [`Host::data`]; bulk columns via [`Host::arrays`]. Host values the
//! extension holds are opaque [`Handle`]s indexing a GC-rooted table on the host (§2).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub use quoin_ext_proto::{ArrowArray, ArrowDType, DataValue};
use quoin_ext_proto::{ClassDecl, Msg, PROTOCOL_VERSION};

/// An opaque reference to a host value, as seen by the extension. Default lifetime is
/// call-local (auto-released when the originating call returns); promote it with
/// [`Host::retain`] to hold it across calls.
pub type Handle = u64;

/// Read one length-prefixed frame. `Ok(None)` on a clean EOF (peer closed between
/// frames); `Err` on a truncated frame or other I/O error.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    // Refuse a frame larger than the shared cap before allocating for it — a corrupt or
    // hostile length would otherwise `vec![0u8; len]` up to ~4 GiB from a 4-byte prefix.
    if len > quoin_ext_proto::MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "extension frame length {len} exceeds the {} byte limit",
                quoin_ext_proto::MAX_FRAME_LEN
            ),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Write `payload` as one length-prefixed frame and flush.
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// The host-callback client handed to a request handler for the duration of one `Call`.
///
/// Each method issues a re-entrant host-op (a `Msg` frame the host services mid-call) and
/// blocks on the matching `HostOpReturn`. It borrows the connection mutably, so host-ops are
/// strictly serialized within the call — exactly the request/response ping-pong the host's
/// service loop expects.
pub struct Host<'a> {
    stream: &'a mut UnixStream,
    /// Host-value handle args passed on this `Call` (a block is one of these), in order.
    handles: Vec<Handle>,
    /// Ext-side resource ids passed back as args on this `Call`, in order.
    resources: Vec<u64>,
    /// Ext-side resource ids the host has dropped; the handler should free them from its own
    /// registry (typically at the top of the call). Empty unless this extension hands out resources.
    releases: Vec<u64>,
    /// Bulk `Array` columns passed as args on this `Call`, in order (the data plane).
    arrays: Vec<ArrowArray>,
    /// The structured-value payload passed via `call:with:data:`, if any.
    data: Option<DataValue>,
    /// The class-dispatch context (`None` on the generic `serve` path): the registry and the
    /// object table, so [`Host::with_instance`] resolves live-instance references AND the wait
    /// loops can service a NESTED `Call` — the host re-entering this extension from inside
    /// a block/method it is currently servicing for us (strictly LIFO on the shared stream).
    ctx: Option<HostCtx<'a>>,
    /// Host-sent stack segments from FAILED host-ops (a Quoin block this call invoked raised,
    /// or a nested call failed deeper down) — appended to this call's `remote_stack` when the
    /// handler propagates the failure, preserving the cross-process interleave in unwind order.
    nested_error_stacks: Vec<String>,
}

/// The dispatch context a class extension threads into each call's [`Host`]. The table is
/// internally locked (lane threads share it), so a shared borrow suffices.
struct HostCtx<'a> {
    ext: &'a Extension,
    table: &'a ObjectTable,
}

impl<'a> Host<'a> {
    /// The host-value handle args for this call (see [`Host::invoke_block`] to run a block one).
    pub fn handles(&self) -> &[Handle] {
        &self.handles
    }

    /// The ext-side resource ids passed back as args for this call (look them up in your registry).
    pub fn resources(&self) -> &[u64] {
        &self.resources
    }

    /// Ext-side resource ids the host has dropped — free these from your registry.
    pub fn releases(&self) -> &[u64] {
        &self.releases
    }

    /// The bulk `Array` columns passed as args on this call (read `dtype`/`data` and crunch them).
    pub fn arrays(&self) -> &[ArrowArray] {
        &self.arrays
    }

    /// The structured-value payload passed via `call:with:data:`, as a `DataValue` tree, if any.
    pub fn data(&self) -> Option<&DataValue> {
        self.data.as_ref()
    }

    /// Resolve a live-instance reference nested inside a data tree (a [`DataValue::Resource`]
    /// leaf — e.g. inside a `List`/`Map` method argument, the way the Python SDK hands the live
    /// object directly) and run `f` over the instance, returning its result. `None` for a
    /// non-`Resource` value, a dead or foreign id, a type mismatch, the receiver or an instance
    /// ARGUMENT of the current call (both are taken out of the table for the call's duration),
    /// or on the generic `serve` path (no object table). The instance is taken out of the
    /// table while `f` runs — the same discipline as the receiver — so `f` sees it exclusively
    /// even with lane threads serving concurrently; consequently a nested `with_instance` on
    /// the SAME id inside `f` answers `None`.
    pub fn with_instance<T: 'static, R>(
        &self,
        value: &DataValue,
        f: impl FnOnce(&T) -> R,
    ) -> Option<R> {
        let DataValue::Resource { id, .. } = value else {
            return None;
        };
        let table = self.ctx.as_ref()?.table;
        let obj = table.take(*id)?;
        let result = obj.downcast_ref::<T>().map(f);
        table.reinsert(*id, obj);
        result
    }

    /// Make a host `String` value and return a (call-local) handle to it.
    pub fn make_string(&mut self, s: &str) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::MakeString {
            value: s.to_string(),
        })?;
        Ok(handle)
    }

    /// Read a `String`-typed handle back into a Rust string.
    pub fn handle_to_string(&mut self, handle: Handle) -> io::Result<String> {
        let (_, str) = self.host_op(&Msg::HandleToString { handle })?;
        str.ok_or_else(|| invalid_data("HandleToString reply carried no string"))
    }

    /// Promote a call-local handle to retained (global), so it survives past this call.
    pub fn retain(&mut self, handle: Handle) -> io::Result<()> {
        self.host_op(&Msg::Retain { handle }).map(|_| ())
    }

    /// Release retained handles (batched).
    pub fn release(&mut self, handles: &[Handle]) -> io::Result<()> {
        self.host_op(&Msg::Release {
            handles: handles.to_vec(),
        })
        .map(|_| ())
    }

    /// Send the Quoin message `selector` to the value behind `receiver`, with the values
    /// behind `args` as the arguments, and return a (call-local) handle to the result. The
    /// host performs a real method dispatch; a host-reported error (bad handle, wrong arity,
    /// or a raise during the send) surfaces as an `io::Error`.
    pub fn call_method(
        &mut self,
        receiver: Handle,
        selector: &str,
        args: &[Handle],
    ) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::CallMethodOnHandle {
            receiver,
            selector: selector.to_string(),
            args: args.to_vec(),
        })?;
        Ok(handle)
    }

    /// Invoke the host block `block` once per tuple in `batches`, in a single round-trip, and
    /// return one result handle per tuple (in order). The host runs the block N times locally;
    /// `batches` of length 1 is a single call. A bad handle or a raise surfaces as an `io::Error`.
    pub fn invoke_block(
        &mut self,
        block: Handle,
        batches: &[Vec<Handle>],
    ) -> io::Result<Vec<Handle>> {
        write_frame(
            self.stream,
            &quoin_ext_proto::encode(&Msg::InvokeBlock {
                block,
                batches: batches.to_vec(),
            }),
        )?;
        match self.read_op_reply()? {
            Msg::InvokeBlockReturn {
                results,
                error,
                remote_stack,
            } => match error {
                Some(e) => {
                    self.note_remote_stack(remote_stack);
                    Err(io::Error::other(e))
                }
                None => Ok(results),
            },
            other => Err(invalid_data(format!(
                "expected InvokeBlockReturn, got {other:?}"
            ))),
        }
    }

    /// Apply a host block (a `Handle` from a method argument — see [`Arg::handle`]) to each input,
    /// in one batched round-trip, returning one result per input. Each input is made into a host
    /// value, the block is invoked once per input, and each result is read back as a [`DataValue`].
    /// The unary mapping form (`v map: { |x| … }`); for richer call shapes use [`Host::invoke_block`]
    /// directly.
    pub fn apply_block(
        &mut self,
        block: Handle,
        inputs: &[DataValue],
    ) -> io::Result<Vec<DataValue>> {
        let batches: Vec<Vec<Handle>> = inputs
            .iter()
            .map(|d| Ok(vec![self.make_value(d.clone())?]))
            .collect::<io::Result<_>>()?;
        let results = self.invoke_block(block, &batches)?;
        results.iter().map(|h| self.read_handle(*h)).collect()
    }

    /// Resolve a name in the host's globals (a class is a class-valued global), returning a handle
    /// to its value — so the extension can reach and drive host classes/globals (e.g.
    /// `call_method(get_global("Array"), "ofFloats:", [list])`). Unbound name -> `io::Error`.
    pub fn get_global(&mut self, name: &str) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::GetGlobal {
            name: name.to_string(),
        })?;
        Ok(handle)
    }

    /// Construct any host value from a [`DataValue`] tree, returning a handle to it (for building
    /// non-string method arguments). The general form of [`Host::make_string`].
    pub fn make_value(&mut self, value: DataValue) -> io::Result<Handle> {
        let (handle, _) = self.host_op(&Msg::MakeValue { value })?;
        Ok(handle)
    }

    /// Project the value behind `handle` to a [`DataValue`] tree — inspect any handle as native
    /// data (the general form of [`Host::handle_to_string`]).
    pub fn read_handle(&mut self, handle: Handle) -> io::Result<DataValue> {
        write_frame(
            self.stream,
            &quoin_ext_proto::encode(&Msg::ReadHandle { handle }),
        )?;
        match self.read_op_reply()? {
            Msg::ReadHandleReturn {
                value,
                error,
                remote_stack,
            } => match error {
                Some(e) => {
                    self.note_remote_stack(remote_stack);
                    Err(io::Error::other(e))
                }
                None => Ok(value),
            },
            other => Err(invalid_data(format!(
                "expected ReadHandleReturn, got {other:?}"
            ))),
        }
    }

    /// Send one host-op and await its `HostOpReturn`, surfacing a host-reported error as an
    /// `io::Error`. Returns the reply's `(handle, str)` payload (either may be unset).
    fn host_op(&mut self, msg: &Msg) -> io::Result<(Handle, Option<String>)> {
        write_frame(self.stream, &quoin_ext_proto::encode(msg))?;
        match self.read_op_reply()? {
            Msg::HostOpReturn {
                handle,
                str,
                error,
                remote_stack,
            } => match error {
                Some(e) => {
                    self.note_remote_stack(remote_stack);
                    Err(io::Error::other(e))
                }
                None => Ok((handle, str)),
            },
            other => Err(invalid_data(format!(
                "expected HostOpReturn, got {other:?}"
            ))),
        }
    }

    /// Record the host's stack segment from a failed host-op (empty = none) for this call's
    /// eventual `remote_stack`.
    fn note_remote_stack(&mut self, segment: String) {
        if !segment.is_empty() {
            self.nested_error_stacks.push(segment);
        }
    }

    /// Read the reply to a pending host-op. A **nested `Call`** arriving here instead is the
    /// host RE-ENTERING this extension — a block/method we are servicing called back in —
    /// so dispatch it and keep waiting: the conversation is a call stack over the socket,
    /// strictly LIFO, and our reply frame always follows the nested call's completion.
    fn read_op_reply(&mut self) -> io::Result<Msg> {
        loop {
            let frame = read_frame(self.stream)?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "host closed mid-host-op")
            })?;
            let msg = quoin_ext_proto::decode_frame(&frame).map_err(invalid_data)?;
            let Msg::Call {
                op,
                class_name,
                recv,
                method_args,
                releases,
                ..
            } = msg
            else {
                return Ok(msg);
            };
            let started = std::time::Instant::now();
            // Borrow the fields disjointly: the nested dispatch needs the stream AND the
            // dispatch context at once.
            let Host { stream, ctx, .. } = self;
            let reply = match ctx {
                Some(ctx) => {
                    // The host batches dropped instances onto every Call, nested ones too.
                    for rid in &releases {
                        ctx.table.take(*rid);
                    }
                    // A dispatch failure here must ANSWER the nested call (recoverably) —
                    // propagating would abandon its reply slot and desync the whole
                    // conversation. Includes the documented Rust limitation: a nested call
                    // to the outer call's receiver (or an instance argument) is taken out
                    // of the table and reports "no live instance".
                    match ctx
                        .ext
                        .dispatch(stream, ctx.table, &class_name, &op, recv, &method_args)
                    {
                        Ok(reply) => reply,
                        Err(e) => Msg::CallReturnError {
                            message: format!("nested extension call failed: {e}"),
                            remote_stack: String::new(),
                        },
                    }
                }
                // A generic handler is one closure with no dispatch table to re-enter;
                // answer the nested call with a recoverable error rather than desyncing.
                None => Msg::CallReturnError {
                    message: "nested extension call: this extension's generic handler \
                              cannot service a re-entrant call"
                        .to_string(),
                    remote_stack: String::new(),
                },
            };
            write_frame(
                stream,
                &quoin_ext_proto::encode_with_meta(&reply, Some(&reply_meta(started))),
            )?;
        }
    }
}

fn invalid_data(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// The meta stamped on every `CallReturn*` terminal: how long this side held the call
/// (from decoding the `Call` to writing its terminal, nested host round-trips included).
/// The host's boundary profiling (`VM.boundaryStats`) splits a call's wall time into
/// queue-wait / transport / remote-handler shares with it.
fn reply_meta(started: std::time::Instant) -> quoin_ext_proto::ReplyMeta {
    quoin_ext_proto::ReplyMeta {
        handler_micros: started.elapsed().as_micros() as u64,
    }
}

/// What a call handler returns: a scalar string, an ext-side resource id, a bulk `Array` (the data
/// plane), or a structured value (materialized as a nested Quoin Value). A `String`/`&str` converts
/// to `Reply::Scalar`, and a `DataValue` to `Reply::Data`, so handlers need little ceremony.
pub enum Reply {
    Scalar(String),
    Resource(u64),
    Array(ArrowArray),
    Data(Value),
    /// Return a live host value the extension holds (a handle from `get_global`/`make_value`/
    /// `call_method`); the host resolves it to the value and returns it to the caller.
    Handle(Handle),
}

impl From<String> for Reply {
    fn from(s: String) -> Self {
        Reply::Scalar(s)
    }
}

impl From<&str> for Reply {
    fn from(s: &str) -> Self {
        Reply::Scalar(s.to_string())
    }
}

impl From<DataValue> for Reply {
    fn from(d: DataValue) -> Self {
        Reply::Data(d.into())
    }
}

impl From<Value> for Reply {
    fn from(v: Value) -> Self {
        Reply::Data(v)
    }
}

/// A handler's structured return tree: [`DataValue`] plus live-instance leaves. `Instance` holds a
/// *new* object of a registered class — at dispatch it enters the object table and crosses as a
/// live-instance reference (MessagePack ext type 3), so a method can return e.g. a `List` of
/// instances or a `Map` containing them (the Python SDK's `isinstance` auto-packing, made explicit
/// for Rust). `Resource` passes an existing reference through verbatim (e.g. a leaf echoed back
/// out of [`Host::data`]). Lowering is atomic: the whole tree is validated before any instance is
/// inserted, so an unregistered type is a recoverable error that leaks nothing.
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(String),
    Float(f64),
    Decimal(String),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
    /// A new live instance of a registered class (see [`Value::instance`]).
    Instance(Box<dyn Any + Send>),
    /// An existing live-instance reference, passed through verbatim.
    Resource {
        id: u64,
        class_name: String,
    },
}

impl Value {
    /// Wrap a new instance of a registered class for embedding anywhere in a return tree.
    /// `T: Send` because it will live in the lane-shared object table.
    pub fn instance<T: Send + 'static>(obj: T) -> Value {
        Value::Instance(Box::new(obj))
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "Null"),
            Value::Bool(b) => write!(f, "Bool({b})"),
            Value::Int(i) => write!(f, "Int({i})"),
            Value::BigInt(s) => write!(f, "BigInt({s})"),
            Value::Float(x) => write!(f, "Float({x})"),
            Value::Decimal(s) => write!(f, "Decimal({s})"),
            Value::Str(s) => write!(f, "Str({s:?})"),
            Value::Bytes(b) => write!(f, "Bytes({} bytes)", b.len()),
            Value::List(xs) => f.debug_tuple("List").field(xs).finish(),
            Value::Map(kvs) => f.debug_tuple("Map").field(kvs).finish(),
            Value::Instance(_) => write!(f, "Instance(..)"),
            Value::Resource { id, class_name } => {
                write!(f, "Resource {{ id: {id}, class_name: {class_name:?} }}")
            }
        }
    }
}

impl From<DataValue> for Value {
    fn from(d: DataValue) -> Self {
        match d {
            DataValue::Null => Value::Null,
            DataValue::Bool(b) => Value::Bool(b),
            DataValue::Int(i) => Value::Int(i),
            DataValue::BigInt(s) => Value::BigInt(s),
            DataValue::Float(x) => Value::Float(x),
            DataValue::Decimal(s) => Value::Decimal(s),
            DataValue::Str(s) => Value::Str(s),
            DataValue::Bytes(b) => Value::Bytes(b),
            DataValue::List(xs) => Value::List(xs.into_iter().map(Into::into).collect()),
            DataValue::Map(kvs) => {
                Value::Map(kvs.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
            DataValue::Resource { id, class_name } => Value::Resource { id, class_name },
        }
    }
}

/// Lower a rich [`Value`] tree with NO object table (the generic `serve` path): pure data passes
/// through, but an `Instance` leaf is an author error — only a class-providing [`Extension`] owns
/// a table to insert into (generic extensions manage their own registries via [`Reply::Resource`]).
fn value_to_data_plain(v: Value) -> Result<DataValue, String> {
    Ok(match v {
        Value::Null => DataValue::Null,
        Value::Bool(b) => DataValue::Bool(b),
        Value::Int(i) => DataValue::Int(i),
        Value::BigInt(s) => DataValue::BigInt(s),
        Value::Float(x) => DataValue::Float(x),
        Value::Decimal(s) => DataValue::Decimal(s),
        Value::Str(s) => DataValue::Str(s),
        Value::Bytes(b) => DataValue::Bytes(b),
        Value::List(xs) => DataValue::List(
            xs.into_iter()
                .map(value_to_data_plain)
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(kvs) => DataValue::Map(
            kvs.into_iter()
                .map(|(k, v)| value_to_data_plain(v).map(|v| (k, v)))
                .collect::<Result<_, _>>()?,
        ),
        Value::Instance(_) => {
            return Err(
                "Value::Instance requires a class-providing Extension (the generic serve has \
                 no object table); return Reply::Resource with a self-managed id instead"
                    .to_string(),
            );
        }
        Value::Resource { id, class_name } => DataValue::Resource { id, class_name },
    })
}

/// Bind a unix socket at `path`, accept one host connection, and serve requests until the host
/// disconnects. Each `Call` frame invokes `handler(host, op, arg)`; the handler may use `host` to
/// read the call's handle/resource/array args and issue re-entrant host-ops, then returns a
/// [`Reply`] (scalar / resource / array) sent back as `CallReturn` / `CallReturnResource` /
/// `CallReturnArray`.
///
/// Blocking and single-connection by design: the extension is its own process, and the VM holds
/// exactly one connection to it. Returns once the host disconnects.
pub fn serve<R: Into<Reply>>(
    path: &str,
    mut handler: impl FnMut(&mut Host, &str, &str) -> R,
) -> io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let (mut stream, _addr) = listener.accept()?;
    // Unlink once the host is connected. The established connection is unaffected, and the
    // protocol never reconnects (one long-lived stream), so the path is dead weight from here
    // on. Doing it now is the only cleanup that survives a signal death of *either* process,
    // which runs no destructor -- the host's `Drop` covers only its graceful exits. `qn`'s
    // worker transport (src/worker.rs) unlinks after its own `accept` for the same reason.
    let _ = std::fs::remove_file(path);
    while let Some(frame) = read_frame(&mut stream)? {
        match quoin_ext_proto::decode_frame(&frame).map_err(invalid_data)? {
            // A generic-handler extension provides no classes (Phase 3): reply with an empty
            // manifest. The reply always carries this SDK's protocol version — the HOST is the
            // enforcer of the version handshake (its error reaches the user; ours would vanish
            // with the process).
            Msg::GetManifest { version: _ } => {
                write_frame(
                    &mut stream,
                    &quoin_ext_proto::encode(&Msg::ManifestReturn {
                        classes: Vec::new(),
                        version: PROTOCOL_VERSION,
                        // The generic path is single-connection by design (see the doc above).
                        lanes: 1,
                    }),
                )?;
            }
            Msg::Call {
                op,
                arg,
                handles,
                resources,
                releases,
                arrays,
                data,
                // The generic `call:with:` handler doesn't use the extension-backed-class fields.
                class_name: _,
                recv: _,
                method_args: _,
            } => {
                let started = std::time::Instant::now();
                // `host` borrows the stream for the call's re-entrant host-ops; the borrow ends
                // before we write the terminal reply on the same stream.
                let reply: Reply = {
                    let mut host = Host {
                        stream: &mut stream,
                        handles,
                        resources,
                        releases,
                        arrays,
                        data,
                        ctx: None,
                        nested_error_stacks: Vec::new(),
                    };
                    handler(&mut host, &op, &arg).into()
                };
                write_frame(
                    &mut stream,
                    &quoin_ext_proto::encode_with_meta(
                        &reply_to_msg(reply)?,
                        Some(&reply_meta(started)),
                    ),
                )?;
            }
            other => {
                return Err(invalid_data(format!(
                    "extension serve: expected a Call to begin a conversation, got {other:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Encode a handler's [`Reply`] as the matching terminal `CallReturn*` frame — the generic-`serve`
/// path, with no object table: a `Value::Instance` leaf is an author error (see
/// [`value_to_data_plain`]), surfaced as an `io::Error` like any other protocol bug.
fn reply_to_msg(reply: Reply) -> io::Result<Msg> {
    Ok(match reply {
        Reply::Scalar(result) => Msg::CallReturn { result },
        Reply::Resource(resource) => Msg::CallReturnResource {
            resource,
            class_name: String::new(),
        },
        Reply::Array(array) => Msg::CallReturnArray { array },
        Reply::Data(value) => Msg::CallReturnData {
            value: value_to_data_plain(value).map_err(invalid_data)?,
        },
        Reply::Handle(handle) => Msg::CallReturnHandle { handle },
    })
}

// ---------------------------------------------------------------------------
// Extension-backed classes (Phase 3): the SDK owns the object table.
//
// An extension provides one or more Quoin classes by registering plain Rust types. The host
// installs a real Quoin class whose method sends dispatch over the socket; the SDK keeps the
// instances in its own table keyed by an opaque id (the resource id the host holds), so writing
// an extension-backed class feels like writing an ordinary type — no handle plumbing in the
// method bodies, and instances are freed automatically when the host drops them.
// ---------------------------------------------------------------------------

/// One resolved method argument handed to a handler (Phase 3 — richer args). `Data` is a decoded
/// value; `Object` is another of *this extension's* live instances (resolved from the object table —
/// downcast with [`Arg::object`]); `Handle` is a host-value handle for a block or other host object
/// the handler drives via [`Host::apply_block`] / the [`Host`] callbacks; `Array` is a bulk numeric
/// column passed on the data plane (a host `Array` argument, whole-buffer).
pub enum Arg<'a> {
    Data(DataValue),
    Object(&'a dyn Any),
    Handle(Handle),
    Array(ArrowArray),
}

impl<'a> Arg<'a> {
    /// The decoded value, if this argument is plain data.
    pub fn data(&self) -> Option<&DataValue> {
        match self {
            Arg::Data(d) => Some(d),
            _ => None,
        }
    }

    /// The live instance behind an ext-object argument, downcast to `T` (the type it was built as).
    pub fn object<T: 'static>(&self) -> Option<&T> {
        match self {
            Arg::Object(o) => o.downcast_ref::<T>(),
            _ => None,
        }
    }

    /// The host-value handle behind a block / non-data argument (drive it via [`Host::apply_block`]).
    pub fn handle(&self) -> Option<Handle> {
        match self {
            Arg::Handle(h) => Some(*h),
            _ => None,
        }
    }

    /// The bulk column behind an `Array` argument (read `dtype`/`data` and crunch the buffer).
    pub fn array(&self) -> Option<&ArrowArray> {
        match self {
            Arg::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// What a class handler (constructor / method / makes) returns. `Ok` is the value; an `Err` is a
/// *recoverable* error — the SDK sends it as a `CallReturnError` and the host raises a catchable
/// Quoin error while the extension keeps running (so a SQL error never tears down a connection).
/// The boxed error accepts anything (`adbc` SQL errors, `io::Error`, …) via `?`.
pub type HandlerResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

/// Erased per-selector handlers (the concrete `T` is captured at registration and downcast here).
/// `Err(String)` is the recoverable error message → `CallReturnError`; transport/protocol failures
/// (bad frame, dead instance id) stay `io::Error` at the dispatch/serve level.
// `Send + Sync` on the handler boxes and `Send` on the boxed instances: lane threads share
// the `Extension` and the object table, so handlers and instances cross thread boundaries.
type CtorFn =
    Box<dyn Fn(&mut Host, &[Arg]) -> Result<Box<dyn Any + Send>, HandlerFailure> + Send + Sync>;
type MethodFn =
    Box<dyn Fn(&mut dyn Any, &mut Host, &[Arg]) -> Result<Reply, HandlerFailure> + Send + Sync>;
type MakesFn = Box<
    dyn Fn(&mut dyn Any, &mut Host, &[Arg]) -> Result<Box<dyn Any + Send>, HandlerFailure>
        + Send
        + Sync,
>;
type ClassMethodFn = Box<dyn Fn(&mut Host, &[Arg]) -> Result<Reply, HandlerFailure> + Send + Sync>;

/// One registered class's erased handler tables, keyed by selector.
struct ClassReg {
    name: String,
    constructors: HashMap<String, CtorFn>,
    methods: HashMap<String, MethodFn>,
    makes: HashMap<String, MakesFn>,
    class_methods: HashMap<String, ClassMethodFn>,
}

/// A failed handler, split for the wire: the short `message` (the Quoin error's `.message`)
/// and this extension's contribution to the opaque cross-process stack blob (the error's
/// `source()` chain — Rust's traceback — with the dispatch frame prepended at dispatch).
struct HandlerFailure {
    message: String,
    remote_stack: String,
}

impl From<String> for HandlerFailure {
    fn from(message: String) -> Self {
        HandlerFailure {
            message,
            remote_stack: String::new(),
        }
    }
}

/// Split a boxed handler error: `to_string()` is the message; the `source()` chain becomes
/// stack-blob lines (one `caused by:` per link).
fn failure_from(e: Box<dyn std::error::Error + Send + Sync>) -> HandlerFailure {
    let mut remote_stack = String::new();
    let mut source = e.source();
    while let Some(cause) = source {
        remote_stack.push_str("caused by: ");
        remote_stack.push_str(&cause.to_string());
        remote_stack.push('\n');
        source = cause.source();
    }
    HandlerFailure {
        message: e.to_string(),
        remote_stack,
    }
}

/// Downcast a table entry to the concrete type the handler was registered with. A mismatch can only
/// be a host/protocol bug; surfaced as a recoverable handler error rather than crashing the loop.
fn downcast<T: 'static>(obj: &mut dyn Any) -> Result<&mut T, HandlerFailure> {
    obj.downcast_mut::<T>().ok_or_else(|| {
        HandlerFailure::from("extension instance is not of the expected type".to_string())
    })
}

/// Configures one extension-backed class, backed by the Rust type `T`. Class-side `constructor`s
/// build a `T`; instance-side `method`s read/mutate it and return a [`Reply`]; `makes` produce a
/// new instance (of this or any registered class). Method arguments arrive as [`Arg`]s — data,
/// another of the extension's live instances (`Arg::object`), or a host block (`Arg::handle`).
pub struct ClassBuilder<T> {
    name: String,
    constructors: HashMap<String, CtorFn>,
    methods: HashMap<String, MethodFn>,
    makes: HashMap<String, MakesFn>,
    class_methods: HashMap<String, ClassMethodFn>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Send + 'static> ClassBuilder<T> {
    /// A class-side constructor: `Class sel: …` builds a new `T` (stored in the object table); the
    /// Quoin caller receives an instance.
    pub fn constructor(
        &mut self,
        selector: &str,
        f: impl Fn(&mut Host, &[Arg]) -> HandlerResult<T> + Send + Sync + 'static,
    ) -> &mut Self {
        self.constructors.insert(
            selector.to_string(),
            Box::new(move |host, args| {
                f(host, args)
                    .map(|t| Box::new(t) as Box<dyn Any + Send>)
                    .map_err(failure_from)
            }),
        );
        self
    }

    /// An instance-side method returning a value (a scalar string or structured [`DataValue`]).
    pub fn method<R: Into<Reply>>(
        &mut self,
        selector: &str,
        f: impl Fn(&mut T, &mut Host, &[Arg]) -> HandlerResult<R> + Send + Sync + 'static,
    ) -> &mut Self {
        self.methods.insert(
            selector.to_string(),
            Box::new(move |obj, host, args| {
                f(downcast::<T>(obj)?, host, args)
                    .map(Into::into)
                    .map_err(failure_from)
            }),
        );
        self
    }

    /// An instance-side method that yields a new instance — of this class (`scale:` / `clone`) or
    /// of *any* registered class (`Matrix.row:` -> `Vector`, a cross-class return). The returned
    /// type's registered class is recovered by `TypeId` at dispatch, so the host wraps it correctly.
    pub fn makes<U: Send + 'static>(
        &mut self,
        selector: &str,
        f: impl Fn(&mut T, &mut Host, &[Arg]) -> HandlerResult<U> + Send + Sync + 'static,
    ) -> &mut Self {
        self.makes.insert(
            selector.to_string(),
            Box::new(move |obj, host, args| {
                f(downcast::<T>(obj)?, host, args)
                    .map(|u| Box::new(u) as Box<dyn Any + Send>)
                    .map_err(failure_from)
            }),
        );
        self
    }

    /// A class-side selector returning a value rather than a new instance (`Class sel: …` -> data,
    /// a scalar, or a [`Value`] tree — which may itself carry new instances, e.g. a class-side
    /// factory returning a `List` of them). The Python SDK gets this implicitly: a "constructor"
    /// returning a non-instance replies as data.
    pub fn class_method<R: Into<Reply>>(
        &mut self,
        selector: &str,
        f: impl Fn(&mut Host, &[Arg]) -> HandlerResult<R> + Send + Sync + 'static,
    ) -> &mut Self {
        self.class_methods.insert(
            selector.to_string(),
            Box::new(move |host, args| f(host, args).map(Into::into).map_err(failure_from)),
        );
        self
    }

    fn into_reg(self) -> ClassReg {
        ClassReg {
            name: self.name,
            constructors: self.constructors,
            methods: self.methods,
            makes: self.makes,
            class_methods: self.class_methods,
        }
    }
}

/// A class-providing extension (Phase 3). Register classes with [`Extension::class`], then
/// [`Extension::serve`]. The SDK owns the instances (a `u64 -> Box<dyn Any>` table); the host holds
/// only opaque ids, and dropped instances are reaped from the table automatically.
#[derive(Default)]
pub struct Extension {
    classes: Vec<ClassReg>,
    /// Maps each registered Rust type to its Quoin class name, so an instance returned by a method
    /// (of this class or another) is wrapped as the right class on the host — cross-class returns.
    type_names: HashMap<TypeId, String>,
    /// Declared in the manifest; 0 (the `Default`) is normalized to 1 at send. See [`Extension::lanes`].
    lanes: u32,
}

impl Extension {
    pub fn new() -> Self {
        Self {
            classes: Vec::new(),
            type_names: HashMap::new(),
            lanes: 1,
        }
    }

    /// Declare how many lane connections this extension serves (default 1). A count above 1
    /// invites the host to open that many connections and issue calls on all of them
    /// concurrently — one conversation per lane, each serviced on its own thread — so calls
    /// to different instances can overlap (the host still serializes calls to any one
    /// instance). Declaring more than one lane asserts that your handlers tolerate that
    /// concurrency; hosts too old to understand the field simply stay at one connection.
    pub fn lanes(&mut self, n: u32) -> &mut Self {
        assert!(
            (1..=1024).contains(&n),
            "Extension::lanes: count must be 1..=1024, got {n}"
        );
        self.lanes = n;
        self
    }

    /// Register a class named `name` backed by the Rust type `T`; `build` configures its
    /// constructors and methods. `T: Send` because instances live in a table shared by the
    /// lane-serving threads (even at the default single lane).
    pub fn class<T: Send + 'static>(
        &mut self,
        name: &str,
        build: impl FnOnce(&mut ClassBuilder<T>),
    ) -> &mut Self {
        self.type_names.insert(TypeId::of::<T>(), name.to_string());
        let mut cb = ClassBuilder {
            name: name.to_string(),
            constructors: HashMap::new(),
            methods: HashMap::new(),
            makes: HashMap::new(),
            class_methods: HashMap::new(),
            _marker: PhantomData,
        };
        build(&mut cb);
        self.classes.push(cb.into_reg());
        self
    }

    /// Bind a unix socket at `path`, accept the host connection(s), and serve until the host
    /// disconnects: answer the spawn-time `GetManifest` from the registered classes, and route
    /// each method `Call` to its handler — materializing returned instances into the object
    /// table. With [`Extension::lanes`] above 1, up to `lanes - 1` further host connections
    /// are accepted and each is served on its own thread over the shared table; a host too
    /// old to open them costs nothing (the accept loop just idles until the first connection
    /// closes).
    pub fn serve(&self, path: &str) -> io::Result<()> {
        let listener = UnixListener::bind(path)?;
        let (stream, _addr) = listener.accept()?;
        let table = ObjectTable::default();
        if self.lanes <= 1 {
            // Single lane: unlink as soon as the host is connected (see the generic `serve` —
            // the one connection never reconnects, and an early unlink survives signal death).
            let _ = std::fs::remove_file(path);
            return self.serve_conn(stream, &table);
        }
        // Multi-lane: the path must OUTLIVE the handshake — the host connects the extra lanes
        // to this same path after reading the manifest — so the unlink moves to the end of the
        // accept loop. The flag stops the accept poll once the first connection closes (a host
        // that never opens extras — an older one, or one that dies — must not pin the loop).
        let done_accepting = AtomicBool::new(false);
        listener.set_nonblocking(true)?;
        let table = &table;
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut accepted = 1u32;
                while accepted < self.lanes && !done_accepting.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((conn, _addr)) => {
                            accepted += 1;
                            if conn.set_nonblocking(false).is_err() {
                                continue;
                            }
                            s.spawn(move || {
                                // A lane failing alone (a decode error, a broken pipe) ends
                                // that lane; the extension keeps serving the others.
                                if let Err(e) = self.serve_conn(conn, table) {
                                    eprintln!("quoin-ext: lane exited: {e}");
                                }
                            });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
                let _ = std::fs::remove_file(path);
            });
            let result = self.serve_conn(stream, table);
            done_accepting.store(true, Ordering::Relaxed);
            result
        })
    }

    /// Serve one connection (one lane) to completion: the shared frame loop behind
    /// [`Extension::serve`]. Only the first connection ever sees `GetManifest`, but answering
    /// it is stateless, so every lane handles it uniformly.
    fn serve_conn(&self, mut stream: UnixStream, table: &ObjectTable) -> io::Result<()> {
        while let Some(frame) = read_frame(&mut stream)? {
            match quoin_ext_proto::decode_frame(&frame).map_err(invalid_data)? {
                // The reply always carries this SDK's protocol version; the host enforces the
                // handshake (see the generic `serve`).
                Msg::GetManifest { version: _ } => {
                    write_frame(&mut stream, &quoin_ext_proto::encode(&self.manifest()))?;
                }
                Msg::Call {
                    op,
                    class_name,
                    recv,
                    method_args,
                    releases,
                    ..
                } => {
                    let started = std::time::Instant::now();
                    // The host batches dropped instances onto `releases`; free them from the table.
                    for rid in &releases {
                        table.take(*rid);
                    }
                    let reply =
                        self.dispatch(&mut stream, table, &class_name, &op, recv, &method_args)?;
                    write_frame(
                        &mut stream,
                        &quoin_ext_proto::encode_with_meta(&reply, Some(&reply_meta(started))),
                    )?;
                }
                other => {
                    return Err(invalid_data(format!(
                        "extension serve: expected a Call or GetManifest, got {other:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// The registered Quoin class name for a freshly built instance, recovered from its concrete
    /// type — so a method returning an instance of *any* registered class is wrapped correctly
    /// (cross-class returns). Empty if the type isn't registered (defensively, an `ExtResource`).
    fn class_name_of(&self, obj: &dyn Any) -> String {
        self.type_names
            .get(&obj.type_id())
            .cloned()
            .unwrap_or_default()
    }

    /// The `ManifestReturn` describing every registered class. Selector lists are
    /// SORTED: the handlers live in `HashMap`s, and hash order would make the manifest
    /// bytes differ from process to process — wire bytes must be deterministic (the
    /// host's replay tooling fingerprints them, and canonical output is right anyway).
    fn manifest(&self) -> Msg {
        let classes = self
            .classes
            .iter()
            .map(|c| {
                let mut instance_selectors: Vec<String> =
                    c.methods.keys().chain(c.makes.keys()).cloned().collect();
                instance_selectors.sort();
                let mut class_selectors: Vec<String> = c
                    .constructors
                    .keys()
                    .chain(c.class_methods.keys())
                    .cloned()
                    .collect();
                class_selectors.sort();
                ClassDecl {
                    name: c.name.clone(),
                    instance_selectors,
                    class_selectors,
                }
            })
            .collect();
        Msg::ManifestReturn {
            classes,
            version: PROTOCOL_VERSION,
            lanes: self.lanes.max(1),
        }
    }

    /// Route one method `Call` to its handler and produce the terminal reply frame.
    fn dispatch(
        &self,
        stream: &mut UnixStream,
        table: &ObjectTable,
        class_name: &str,
        op: &str,
        recv: u64,
        method_args: &[quoin_ext_proto::Arg],
    ) -> io::Result<Msg> {
        let class = self
            .classes
            .iter()
            .find(|c| c.name == class_name)
            .ok_or_else(|| invalid_data(format!("no extension-backed class '{class_name}'")))?;
        // Each branch TAKES the receiver and the ext-instance arguments out of the table for
        // the handler's duration: the receiver so its `&mut` can't alias, the arguments so
        // the table stays free — `Host` holds it `&mut` to service NESTED calls arriving
        // while the handler waits on a host-op. Consequence: a nested call addressed to the
        // outer call's receiver (or one of its instance arguments) finds "no live instance".
        if recv == 0 {
            if let Some(ctor) = class.constructors.get(op) {
                // Class-side: a constructor builds a new instance.
                let taken = take_arg_instances(table, method_args)?;
                let (result, nested) = {
                    let args = resolve_args(method_args, &taken);
                    let mut host = host_for_call(self, stream, table);
                    let r = ctor(&mut host, &args);
                    (r, std::mem::take(&mut host.nested_error_stacks))
                };
                reinsert_instances(table, taken);
                let obj = match result {
                    Ok(o) => o,
                    Err(failure) => return Ok(call_error(class_name, op, recv, failure, nested)),
                };
                let class_name = self.class_name_of(&*obj);
                Ok(Msg::CallReturnResource {
                    resource: table.insert(obj),
                    class_name,
                })
            } else if let Some(class_method) = class.class_methods.get(op) {
                // Class-side selector returning a value (possibly a tree carrying new instances).
                let taken = take_arg_instances(table, method_args)?;
                let (result, nested) = {
                    let args = resolve_args(method_args, &taken);
                    let mut host = host_for_call(self, stream, table);
                    let r = class_method(&mut host, &args);
                    (r, std::mem::take(&mut host.nested_error_stacks))
                };
                reinsert_instances(table, taken);
                match result {
                    Ok(reply) => Ok(self.finish_reply(reply, table)),
                    Err(failure) => Ok(call_error(class_name, op, recv, failure, nested)),
                }
            } else {
                Err(invalid_data(format!(
                    "no constructor '{op}' on class '{class_name}'"
                )))
            }
        } else if let Some(method) = class.methods.get(op) {
            // Reinserted even when the handler errors, so a recoverable error (e.g. a SQL
            // error) leaves the instance usable.
            let mut recv_box = table
                .take(recv)
                .ok_or_else(|| invalid_data(format!("no live instance {recv}")))?;
            let taken = match take_arg_instances(table, method_args) {
                Ok(t) => t,
                Err(e) => {
                    table.reinsert(recv, recv_box);
                    return Err(e);
                }
            };
            let (result, nested) = {
                let args = resolve_args(method_args, &taken);
                let mut host = host_for_call(self, stream, table);
                let r = method(recv_box.as_mut(), &mut host, &args);
                (r, std::mem::take(&mut host.nested_error_stacks))
            };
            reinsert_instances(table, taken);
            table.reinsert(recv, recv_box);
            match result {
                Ok(reply) => Ok(self.finish_reply(reply, table)),
                Err(failure) => Ok(call_error(class_name, op, recv, failure, nested)),
            }
        } else if let Some(makes) = class.makes.get(op) {
            let mut recv_box = table
                .take(recv)
                .ok_or_else(|| invalid_data(format!("no live instance {recv}")))?;
            let taken = match take_arg_instances(table, method_args) {
                Ok(t) => t,
                Err(e) => {
                    table.reinsert(recv, recv_box);
                    return Err(e);
                }
            };
            let (result, nested) = {
                let args = resolve_args(method_args, &taken);
                let mut host = host_for_call(self, stream, table);
                let r = makes(recv_box.as_mut(), &mut host, &args);
                (r, std::mem::take(&mut host.nested_error_stacks))
            };
            reinsert_instances(table, taken);
            table.reinsert(recv, recv_box);
            let new_obj = match result {
                Ok(o) => o,
                Err(failure) => return Ok(call_error(class_name, op, recv, failure, nested)),
            };
            let class_name = self.class_name_of(&*new_obj);
            Ok(Msg::CallReturnResource {
                resource: table.insert(new_obj),
                class_name,
            })
        } else {
            Err(invalid_data(format!(
                "no method '{op}' on class '{class_name}'"
            )))
        }
    }

    /// Encode a class handler's [`Reply`], lowering a rich [`Value`] tree through the object table:
    /// every `Instance` leaf's type is validated against the registry *first*, then each is
    /// inserted and becomes a live-instance reference — atomic, so an unregistered type is a
    /// recoverable `CallReturnError` that inserts nothing.
    fn finish_reply(&self, reply: Reply, table: &ObjectTable) -> Msg {
        match reply {
            Reply::Scalar(result) => Msg::CallReturn { result },
            Reply::Resource(resource) => Msg::CallReturnResource {
                resource,
                class_name: String::new(),
            },
            Reply::Array(array) => Msg::CallReturnArray { array },
            Reply::Data(value) => match self.check_instances(&value) {
                Ok(()) => Msg::CallReturnData {
                    value: self.lower_value(value, table),
                },
                Err(message) => Msg::CallReturnError {
                    message,
                    remote_stack: String::new(),
                },
            },
            Reply::Handle(handle) => Msg::CallReturnHandle { handle },
        }
    }

    /// Pass 1 of the atomic lowering: every `Instance` leaf must be of a registered class.
    fn check_instances(&self, v: &Value) -> Result<(), String> {
        match v {
            Value::Instance(obj) => {
                if self.type_names.contains_key(&(**obj).type_id()) {
                    Ok(())
                } else {
                    Err(
                        "returned Value::Instance of a type not registered with any class"
                            .to_string(),
                    )
                }
            }
            Value::List(xs) => xs.iter().try_for_each(|x| self.check_instances(x)),
            Value::Map(kvs) => kvs.iter().try_for_each(|(_, x)| self.check_instances(x)),
            _ => Ok(()),
        }
    }

    /// Pass 2: insert each `Instance` into the table and lower it to a live-instance reference
    /// (infallible after [`Extension::check_instances`]).
    fn lower_value(&self, v: Value, table: &ObjectTable) -> DataValue {
        match v {
            Value::Null => DataValue::Null,
            Value::Bool(b) => DataValue::Bool(b),
            Value::Int(i) => DataValue::Int(i),
            Value::BigInt(s) => DataValue::BigInt(s),
            Value::Float(x) => DataValue::Float(x),
            Value::Decimal(s) => DataValue::Decimal(s),
            Value::Str(s) => DataValue::Str(s),
            Value::Bytes(b) => DataValue::Bytes(b),
            Value::List(xs) => {
                DataValue::List(xs.into_iter().map(|x| self.lower_value(x, table)).collect())
            }
            Value::Map(kvs) => DataValue::Map(
                kvs.into_iter()
                    .map(|(k, x)| (k, self.lower_value(x, table)))
                    .collect(),
            ),
            Value::Instance(obj) => {
                let class_name = self.class_name_of(&*obj);
                DataValue::Resource {
                    id: table.insert(obj),
                    class_name,
                }
            }
            Value::Resource { id, class_name } => DataValue::Resource { id, class_name },
        }
    }
}

/// Build the per-call [`Host`] for a class dispatch: the class path carries everything in
/// `method_args`, so the legacy arg vectors are empty, and the dispatch context backs
/// [`Host::instance`] and nested-`Call` servicing (`read_op_reply`).
fn host_for_call<'a>(
    ext: &'a Extension,
    stream: &'a mut UnixStream,
    table: &'a ObjectTable,
) -> Host<'a> {
    Host {
        stream,
        handles: Vec::new(),
        resources: Vec::new(),
        releases: Vec::new(),
        arrays: Vec::new(),
        data: None,
        ctx: Some(HostCtx { ext, table }),
        nested_error_stacks: Vec::new(),
    }
}

/// Compose a failed call's `CallReturnError`: this extension's stack segment — the
/// dispatch frame line, then the error's `caused by:` chain — followed by any host
/// segments from failed host-ops (a Quoin block that raised), in unwind order. The blob
/// is OPAQUE to the host: it displays it fenced, never parses it (PROTOCOL.md §Errors).
fn call_error(
    class_name: &str,
    op: &str,
    recv: u64,
    failure: HandlerFailure,
    nested: Vec<String>,
) -> Msg {
    let mut remote_stack = if recv == 0 {
        format!("in {class_name}.{op}: {}\n", failure.message)
    } else {
        format!(
            "in {class_name}#{op} (instance {recv}): {}\n",
            failure.message
        )
    };
    remote_stack.push_str(&failure.remote_stack);
    for segment in nested {
        remote_stack.push_str(&segment);
        if !remote_stack.ends_with('\n') {
            remote_stack.push('\n');
        }
    }
    Msg::CallReturnError {
        message: failure.message,
        remote_stack,
    }
}

/// TAKE each ext-instance argument out of the table for the call's duration (duplicates
/// share one entry) — like the receiver, so the table stays free for a NESTED dispatch
/// while the handler runs. A bad id reinserts everything taken so far before erroring
/// (a protocol bug must not eat live instances).
fn take_arg_instances(
    table: &ObjectTable,
    method_args: &[quoin_ext_proto::Arg],
) -> io::Result<Vec<(u64, Box<dyn Any + Send>)>> {
    let mut taken: Vec<(u64, Box<dyn Any + Send>)> = Vec::new();
    for arg in method_args {
        if let quoin_ext_proto::Arg::Resource(id) = arg {
            if taken.iter().any(|(t, _)| t == id) {
                continue;
            }
            match table.take(*id) {
                Some(obj) => taken.push((*id, obj)),
                None => {
                    let message = format!("argument references no live instance {id}");
                    reinsert_instances(table, taken);
                    return Err(invalid_data(message));
                }
            }
        }
    }
    Ok(taken)
}

/// Put every taken argument instance back under its id once the handler is done.
fn reinsert_instances(table: &ObjectTable, taken: Vec<(u64, Box<dyn Any + Send>)>) {
    for (id, obj) in taken {
        table.reinsert(id, obj);
    }
}

/// Resolve the wire arguments to the handler-facing [`Arg`]s: data passes through, an
/// ext-instance id resolves into the TAKEN set (see [`take_arg_instances`] — infallible
/// after a successful take), and a handle passes through.
fn resolve_args<'t>(
    method_args: &[quoin_ext_proto::Arg],
    taken: &'t [(u64, Box<dyn Any + Send>)],
) -> Vec<Arg<'t>> {
    method_args
        .iter()
        .map(|a| match a {
            quoin_ext_proto::Arg::Data(d) => Arg::Data(d.clone()),
            quoin_ext_proto::Arg::Resource(id) => {
                let (_, obj) = taken
                    .iter()
                    .find(|(t, _)| t == id)
                    .expect("resolve_args: id vanished from the taken set");
                Arg::Object(obj.as_ref())
            }
            quoin_ext_proto::Arg::Handle(h) => Arg::Handle(*h),
            quoin_ext_proto::Arg::Array(a) => Arg::Array(a.clone()),
            // Quoin worker peers only (a shipped channel endpoint) — a host
            // never sends this kind to a foreign extension; surface it as an
            // opaque handle rather than crash if one ever appears.
            quoin_ext_proto::Arg::Chan(c) => Arg::Handle(*c),
        })
        .collect()
}

/// The SDK-owned instance table (Phase 3): live instances keyed by an opaque id — the resource id
/// the host holds for each. Ids start at 1, so `recv == 0` unambiguously means a class-side send.
/// Internally locked: lane threads share it, and the lock is structural only — it is held for
/// single map operations, never across a handler (instances are `take`n out for a call's
/// duration, which is what makes concurrent lanes safe; the host additionally never issues two
/// concurrent calls to one instance).
#[derive(Default)]
struct ObjectTable {
    inner: Mutex<TableInner>,
}

#[derive(Default)]
struct TableInner {
    objects: HashMap<u64, Box<dyn Any + Send>>,
    next_id: u64,
}

impl ObjectTable {
    /// Store an instance under a fresh id and return it.
    fn insert(&self, obj: Box<dyn Any + Send>) -> u64 {
        let mut inner = self.inner.lock().expect("object table lock poisoned");
        inner.next_id += 1;
        let id = inner.next_id;
        inner.objects.insert(id, obj);
        id
    }

    /// Remove and return the instance for `id` (e.g. the receiver of an instance method, or one the
    /// host has dropped), or `None` if it isn't live.
    fn take(&self, id: u64) -> Option<Box<dyn Any + Send>> {
        let mut inner = self.inner.lock().expect("object table lock poisoned");
        inner.objects.remove(&id)
    }

    /// Put a previously-`take`n instance back under its id.
    fn reinsert(&self, id: u64, obj: Box<dyn Any + Send>) {
        let mut inner = self.inner.lock().expect("object table lock poisoned");
        inner.objects.insert(id, obj);
    }
}
