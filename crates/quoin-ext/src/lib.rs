//! quoin-ext — the **extension-side** SDK for out-of-process Quoin extensions
//! (Tier 1 of the extension architecture; see `docs/FUTURE_EXT_ARCH.md`).
//!
//! An extension is a separate process the Quoin VM spawns and talks to over a unix
//! domain socket. This crate is the thin per-language client an extension links
//! against — it is **not** linked into the VM. (The VM-side host API is the
//! separate in-process `ext_sdk` surface.)
//!
//! ## Wire protocol
//!
//! Messages are length-prefixed frames: a little-endian `u32` length followed by that many
//! payload bytes. The payload is a FlatBuffers `Message` union (schema + codec in the shared
//! `quoin-ext-proto` crate). A `Call` carries typed handle args — host-value handles via
//! [`Host::handles`] (a block is one of these) and ext-side resource ids via [`Host::resources`].
//! The handler may issue **re-entrant host-ops** through the [`Host`] client — `make_string`,
//! `handle_to_string`, `retain`, `release`, `call_method` (send a Quoin message to a handle),
//! and `invoke_block` (run a host block over a batch in one round-trip) — each a synchronous
//! round-trip the host services while parked on the reply. It returns a [`Reply`] — a scalar or
//! an ext-side **resource** the host then holds as an opaque token (reaped via [`Host::releases`]
//! on a later call). Host values the extension holds are opaque [`Handle`]s indexing a GC-rooted
//! table on the host (`docs/FUTURE_EXT_ARCH.md` §2). Bulk columnar data crosses as [`ArrowArray`]s
//! — call args via [`Host::arrays`], returns via [`Reply::Array`] (the data plane, copy-through).

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use quoin_ext_proto::Msg;
pub use quoin_ext_proto::{ArrowArray, ArrowDType, DataValue};

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
        let frame = read_frame(self.stream)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "host closed mid-host-op")
        })?;
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::InvokeBlockReturn { results, error } => match error {
                Some(e) => Err(io::Error::other(e)),
                None => Ok(results),
            },
            other => Err(invalid_data(format!(
                "expected InvokeBlockReturn, got {other:?}"
            ))),
        }
    }

    /// Send one host-op and await its `HostOpReturn`, surfacing a host-reported error as an
    /// `io::Error`. Returns the reply's `(handle, str)` payload (either may be unset).
    fn host_op(&mut self, msg: &Msg) -> io::Result<(Handle, Option<String>)> {
        write_frame(self.stream, &quoin_ext_proto::encode(msg))?;
        let frame = read_frame(self.stream)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "host closed mid-host-op")
        })?;
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::HostOpReturn { handle, str, error } => match error {
                Some(e) => Err(io::Error::other(e)),
                None => Ok((handle, str)),
            },
            other => Err(invalid_data(format!(
                "expected HostOpReturn, got {other:?}"
            ))),
        }
    }
}

fn invalid_data(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// What a call handler returns: a scalar string, an ext-side resource id, a bulk `Array` (the data
/// plane), or a structured value (materialized as a nested Quoin Value). A `String`/`&str` converts
/// to `Reply::Scalar`, and a `DataValue` to `Reply::Data`, so handlers need little ceremony.
pub enum Reply {
    Scalar(String),
    Resource(u64),
    Array(ArrowArray),
    Data(DataValue),
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
        Reply::Data(d)
    }
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
    while let Some(frame) = read_frame(&mut stream)? {
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::Call {
                op,
                arg,
                handles,
                resources,
                releases,
                arrays,
                data,
            } => {
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
                    };
                    handler(&mut host, &op, &arg).into()
                };
                let msg = match reply {
                    Reply::Scalar(result) => Msg::CallReturn { result },
                    Reply::Resource(resource) => Msg::CallReturnResource { resource },
                    Reply::Array(array) => Msg::CallReturnArray { array },
                    Reply::Data(value) => Msg::CallReturnData { value },
                };
                write_frame(&mut stream, &quoin_ext_proto::encode(&msg))?;
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
