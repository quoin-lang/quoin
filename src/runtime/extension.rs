//! `Extension` — the Quoin-facing handle to an out-of-process native extension
//! (Tier 1; see `docs/FUTURE_EXT_ARCH.md`). Slice 1 is the **transport keystone**:
//! spawn a subprocess, connect a unix domain socket, and round-trip one scalar op —
//! with the calling fiber parking on the socket fd through the existing reactor
//! (`await_io` `Write` then `Read`), so a slow extension never stalls the VM.
//!
//! This is a legacy (`&mut VmState`) native class, not an `ext_sdk` one: it is itself
//! an async/IO primitive that needs `await_io`, which lives below the SDK surface.
//!
//! Scope (Slice 1): scalars only, hand-rolled length-framing. Handles, FlatBuffers,
//! Arrow, batched callbacks, and crash/timeout handling are later slices.

use std::any::Any;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use gc_arena::collect::Trace;

use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::value::{AnyCollect, NativeClassBuilder, Value};
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
        // `ext call: '<op>' with: '<arg>'` -> send one request, await the reply. Slice-1
        // scalar protocol: op + arg are strings, result is a string.
        .instance_method("call:with:", |vm, mc, receiver, args| {
            let id = receiver
                .with_native_state::<NativeExtension, _, _>(|e| e.id)
                .map_err(QuoinError::Other)?;
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();

            // Request frame: u32-LE length + `op \0 arg`.
            let mut payload = op.into_bytes();
            payload.push(0);
            payload.extend_from_slice(argv.as_bytes());
            let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(&payload);

            match vm.await_io(IoRequest::Write { id, bytes: frame })? {
                IoResult::Wrote(_) => {}
                IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
                other => {
                    return Err(QuoinError::Other(format!(
                        "Extension call: unexpected write result {other:?}"
                    )));
                }
            }

            let reply = read_reply_frame(vm, id)?;
            Ok(vm.new_string(mc, String::from_utf8_lossy(&reply).into_owned()))
        })
}
