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
//! `quoin-ext-proto` crate). A host->ext `Call` may be answered directly, or the handler may
//! first issue **re-entrant host-ops** through the [`Host`] client — `make_string`,
//! `handle_to_string`, `retain`, `release`, `call_method` (send a Quoin message to a handle),
//! and `invoke_block` (run a host block over a batch in one round-trip) — each a synchronous
//! round-trip the host services while parked on the reply. Host values the extension holds are
//! opaque [`Handle`]s indexing a GC-rooted table on the host (`docs/FUTURE_EXT_ARCH.md` §2).
//! General handle-typed call args/returns and Arrow arrive in later slices.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use quoin_ext_proto::Msg;

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
    /// The handle to the host block passed on this `Call`, if any (see [`Host::block`]).
    block: Option<Handle>,
}

impl<'a> Host<'a> {
    /// The host block handed to this call via `Extension call:with:block:`, or `None`. Invoke
    /// it over a batch of argument tuples with [`Host::invoke_block`].
    pub fn block(&self) -> Option<Handle> {
        self.block
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

/// Bind a unix socket at `path`, accept one host connection, and serve requests until the
/// host disconnects. Each `Call` frame invokes `handler(host, op, arg)`; the handler may use
/// `host` to issue re-entrant host-ops (including [`Host::block`] / [`Host::invoke_block`] when
/// the call carried a block), then returns the reply string sent back as a `CallReturn`.
///
/// Blocking and single-connection by design: the extension is its own process, and the VM
/// holds exactly one connection to it. Returns once the host disconnects.
pub fn serve(
    path: &str,
    mut handler: impl FnMut(&mut Host, &str, &str) -> String,
) -> io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let (mut stream, _addr) = listener.accept()?;
    while let Some(frame) = read_frame(&mut stream)? {
        match quoin_ext_proto::decode_envelope(&frame).map_err(invalid_data)? {
            Msg::Call { op, arg, block } => {
                // `host` borrows the stream for the call's re-entrant host-ops; the borrow
                // ends before we write the terminal `CallReturn` on the same stream. A `block`
                // of 0 (NULL_HANDLE) means none was passed.
                let result = {
                    let mut host = Host {
                        stream: &mut stream,
                        block: (block != 0).then_some(block),
                    };
                    handler(&mut host, &op, &arg)
                };
                write_frame(
                    &mut stream,
                    &quoin_ext_proto::encode(&Msg::CallReturn { result }),
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
