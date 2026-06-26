//! `Extension` — the Quoin-facing handle to an out-of-process native extension
//! (Tier 1; see `docs/FUTURE_EXT_ARCH.md`). Slice 1 is the **transport keystone**:
//! spawn a subprocess, connect a unix domain socket, and round-trip one scalar op —
//! with the calling fiber parking on the socket fd through the existing reactor
//! (`await_io` `Write` then `Read`), so a slow extension never stalls the VM.
//!
//! This is a legacy (`&mut VmState`) native class, not an `ext_sdk` one: it is itself
//! an async/IO primitive that needs `await_io`, which lives below the SDK surface.
//!
//! Slice 3a adds the **handle table** (`docs/FUTURE_EXT_ARCH.md` §2): a `call:with:` is no
//! longer a one-shot request/reply but a re-entrant *conversation*. After sending the `Call`,
//! the host services a loop of frames — each is either a host-op request the extension issued
//! mid-call (`MakeString`/`HandleToString`/`Retain`/`Release`, answered with `HostOpReturn`) or
//! the terminal `CallReturn`. Handles minted during the call are call-local and swept on return
//! (`HandleTable::begin_call`/`end_call`); the extension `Retain`s any it wants to keep.
//!
//! Every frame is a FlatBuffers `Message` union (schema/codec in `quoin-ext-proto`) inside a
//! u32 length-prefixed frame. Handle-typed call args/returns, `CallMethodOnHandle`, batched
//! callbacks, Arrow, and crash/timeout are later slices.

use std::any::Any;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use gc_arena::collect::Trace;

use quoin_ext_proto::Msg;

use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

/// Native state behind an `Extension` value: the registered stream id for the UDS,
/// the child process, and its socket path (for cleanup).
#[derive(Debug)]
pub struct NativeExtension {
    id: StreamId,
    child: Child,
    sock_path: String,
}

impl AnyCollect for NativeExtension {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    // Holds no GC values — nothing to trace.
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl Drop for NativeExtension {
    fn drop(&mut self) {
        // Best-effort teardown: kill + reap the child, remove the socket file. The
        // host-side stream fd is left registered until VM exit (Slice 1 leaks it; the
        // reap-queue lifecycle is a later slice).
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

/// A short, unique unix-socket path. `/tmp` (not `temp_dir()`) keeps it well under the
/// ~104-byte `sun_path` limit on macOS, where `temp_dir()` is deep.
fn unique_sock_path() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("/tmp/quoin-ext-{}-{}.sock", std::process::id(), n)
}

/// Read up to one chunk from the extension stream, parking the fiber on the socket.
fn read_chunk<'gc>(vm: &mut VmState<'gc>, id: StreamId) -> Result<Vec<u8>, QuoinError> {
    match vm.await_io(IoRequest::Read { id, max: 4096 })? {
        IoResult::Read(b) => Ok(b),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "Extension: unexpected read result {other:?}"
        ))),
    }
}

/// Read exactly one length-prefixed reply frame (u32-LE length + payload), looping
/// over `Read`s (each a park point) until the whole frame has arrived.
fn read_reply_frame<'gc>(vm: &mut VmState<'gc>, id: StreamId) -> Result<Vec<u8>, QuoinError> {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < 4 {
        let chunk = read_chunk(vm, id)?;
        if chunk.is_empty() {
            return Err(QuoinError::Other(
                "Extension call: connection closed before reply".to_string(),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    while buf.len() < 4 + len {
        let chunk = read_chunk(vm, id)?;
        if chunk.is_empty() {
            return Err(QuoinError::Other(
                "Extension call: truncated reply".to_string(),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf[4..4 + len].to_vec())
}

/// Encode `msg` and write it as one length-prefixed frame, parking the fiber on the socket.
fn write_msg<'gc>(vm: &mut VmState<'gc>, id: StreamId, msg: &Msg) -> Result<(), QuoinError> {
    let payload = quoin_ext_proto::encode(msg);
    let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&payload);
    match vm.await_io(IoRequest::Write { id, bytes: frame })? {
        IoResult::Wrote(_) => Ok(()),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "Extension: unexpected write result {other:?}"
        ))),
    }
}

/// Read the Rust string behind a host `String` value, or `None` if it isn't one.
fn read_string_value(value: Value<'_>) -> Option<String> {
    match value {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::String(s) => Some(s.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// Service one re-entrant host-op the extension issued mid-call, writing back its
/// `HostOpReturn`. Returns `Ok(())` for every host-op; the caller's loop handles `CallReturn`.
fn service_host_op<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    id: StreamId,
    epoch: u32,
    msg: Msg,
) -> Result<(), QuoinError> {
    let reply = match msg {
        Msg::MakeString { value } => {
            let v = vm.new_string(mc, value);
            let handle = vm.handle_table.mint_local(v, epoch);
            Msg::HostOpReturn {
                handle,
                str: None,
                error: None,
            }
        }
        Msg::HandleToString { handle } => match vm.handle_table.get(handle) {
            Ok(v) => match read_string_value(v) {
                Some(s) => Msg::HostOpReturn {
                    handle: 0,
                    str: Some(s),
                    error: None,
                },
                None => host_op_error(format!("handle {handle} does not refer to a String")),
            },
            Err(e) => host_op_error(e),
        },
        Msg::Retain { handle } => match vm.handle_table.retain(handle) {
            Ok(()) => ack(),
            Err(e) => host_op_error(e),
        },
        Msg::Release { handles } => {
            vm.handle_table.release(&handles);
            ack()
        }
        other => {
            return Err(QuoinError::Other(format!(
                "Extension call: unexpected message from extension: {other:?}"
            )));
        }
    };
    write_msg(vm, id, &reply)
}

fn ack() -> Msg {
    Msg::HostOpReturn {
        handle: 0,
        str: None,
        error: None,
    }
}

fn host_op_error(message: String) -> Msg {
    Msg::HostOpReturn {
        handle: 0,
        str: None,
        error: Some(message),
    }
}

pub fn build_extension_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Extension", Some("Object"))
        // `Extension spawn: '<path-to-binary>'` -> spawn the extension subprocess and
        // connect to it, returning an Extension handle.
        .class_method("spawn:", |vm, mc, _receiver, args| {
            let bin_path = arg!(args, String, 0).to_string();
            let sock_path = unique_sock_path();
            let mut child = Command::new(&bin_path)
                .arg(&sock_path)
                .spawn()
                .map_err(|e| {
                    QuoinError::Other(format!(
                        "Extension spawn: failed to start '{bin_path}': {e}"
                    ))
                })?;

            // The child binds the socket asynchronously after exec, so retry the
            // connect briefly until it's listening (each attempt parks the fiber).
            let mut attempts = 0u32;
            let id = loop {
                match vm.await_io(IoRequest::ConnectUnix {
                    path: sock_path.clone(),
                })? {
                    IoResult::Connected(id) => break id,
                    IoResult::Err(_) if attempts < 100 => {
                        attempts += 1;
                        vm.await_io(IoRequest::Sleep { ms: 5 })?;
                    }
                    IoResult::Err(e) => {
                        let _ = child.kill();
                        return Err(QuoinError::from_io_error(&e));
                    }
                    other => {
                        let _ = child.kill();
                        return Err(QuoinError::Other(format!(
                            "Extension spawn: unexpected connect result {other:?}"
                        )));
                    }
                }
            };

            let class = vm.get_or_create_builtin_class(mc, "Extension");
            Ok(vm.new_native_state(
                mc,
                class,
                NativeExtension {
                    id,
                    child,
                    sock_path,
                },
            ))
        })
        // `ext call: '<op>' with: '<arg>'` -> send the `Call`, then service the conversation:
        // a loop of re-entrant host-ops the extension may issue (each answered inline) until it
        // sends the terminal `CallReturn`. Op + arg are strings; the result is a string.
        .instance_method("call:with:", |vm, mc, receiver, args| {
            let id = receiver
                .with_native_state::<NativeExtension, _, _>(|e| e.id)
                .map_err(QuoinError::Other)?;
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();

            // Open a call epoch so handles minted during it are call-local (swept on return).
            let epoch = vm.handle_table.begin_call();

            // The whole conversation must close out the epoch even on error, so the call's
            // transient handles never leak. Run it in a closure and sweep unconditionally.
            let outcome: Result<String, QuoinError> = (|| {
                write_msg(vm, id, &Msg::Call { op, arg: argv })?;
                loop {
                    let frame = read_reply_frame(vm, id)?;
                    let msg = quoin_ext_proto::decode_envelope(&frame).map_err(|e| {
                        QuoinError::Other(format!("Extension call: malformed frame: {e}"))
                    })?;
                    match msg {
                        Msg::CallReturn { result } => return Ok(result),
                        host_op => service_host_op(vm, mc, id, epoch, host_op)?,
                    }
                }
            })();

            vm.handle_table.end_call(epoch);
            Ok(vm.new_string(mc, outcome?))
        })
}
