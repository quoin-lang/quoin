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
//! mid-call (answered with `HostOpReturn`) or the terminal `CallReturn`. Handles minted during
//! the call are call-local and swept on return (`HandleTable::begin_call`/`end_call`); the
//! extension `Retain`s any it wants to keep.
//!
//! The host-ops are `MakeString`/`HandleToString`/`Retain`/`Release` (Slice 3a),
//! `CallMethodOnHandle` (Slice 3b — send a Quoin message to a handle), and `InvokeBlock`
//! (Slice 4 — invoke a host *block* handle over a batch of argument tuples in one round-trip;
//! the block reaches the extension via the `block` handle on `Call`, sent by `call:with:block:`).
//! Every frame is a FlatBuffers `Message` union (schema/codec in `quoin-ext-proto`) inside a u32
//! length-prefixed frame. General handle-typed call args/returns, Arrow, and crash/timeout are
//! later slices.

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

/// Resolve a receiver handle and a list of argument handles to their host `Value`s
/// (each `Copy`), so the table borrow is released before a `&mut self` method send.
fn resolve_handles<'gc>(
    vm: &VmState<'gc>,
    receiver: u64,
    args: &[u64],
) -> Result<(Value<'gc>, Vec<Value<'gc>>), String> {
    let recv = vm.handle_table.get(receiver)?;
    let mut arg_vals = Vec::with_capacity(args.len());
    for &handle in args {
        arg_vals.push(vm.handle_table.get(handle)?);
    }
    Ok((recv, arg_vals))
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
        Msg::CallMethodOnHandle {
            receiver,
            selector,
            args,
        } => match resolve_handles(vm, receiver, &args) {
            // Resolve all handles first (dropping the table borrow), then perform a real
            // host-side send; mint a call-local handle for the result. A Quoin-level raise
            // surfaces to the extension as a host-op error, not a failed `call:with:`.
            Ok((recv, arg_vals)) => match vm.call_method(mc, recv, &selector, arg_vals) {
                Ok(result) => {
                    let handle = vm.handle_table.mint_local(result, epoch);
                    Msg::HostOpReturn {
                        handle,
                        str: None,
                        error: None,
                    }
                }
                Err(e) => host_op_error(format!("call '{selector}' on handle: {e}")),
            },
            Err(e) => host_op_error(e),
        },
        Msg::InvokeBlock { block, batches } => {
            match invoke_block_batches(vm, mc, epoch, block, &batches) {
                Ok(results) => Msg::InvokeBlockReturn {
                    results,
                    error: None,
                },
                Err(e) => Msg::InvokeBlockReturn {
                    results: Vec::new(),
                    error: Some(e),
                },
            }
        }
        other => {
            return Err(QuoinError::Other(format!(
                "Extension call: unexpected message from extension: {other:?}"
            )));
        }
    };
    write_msg(vm, id, &reply)
}

/// Invoke the host block behind `block_handle` once per tuple in `batches`, minting a
/// call-local handle for each result. The host runs the block N times locally — the batch is
/// one re-entrant round-trip. Any bad handle or a raise during a block run fails the whole batch.
#[allow(no_gc_across_yield)]
fn invoke_block_batches<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    epoch: u32,
    block_handle: u64,
    batches: &[Vec<u64>],
) -> Result<Vec<u64>, String> {
    // Resolve the handle to a block value (rooted in the handle table, so safe to hold).
    let block = match vm.handle_table.get(block_handle)? {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::Block(b) => *b,
            _ => return Err(format!("handle {block_handle} does not refer to a block")),
        },
        _ => return Err(format!("handle {block_handle} does not refer to a block")),
    };

    let mut results = Vec::with_capacity(batches.len());
    for tuple in batches {
        let mut arg_vals = Vec::with_capacity(tuple.len());
        for &handle in tuple {
            arg_vals.push(vm.handle_table.get(handle)?);
        }
        let result = vm
            .execute_block(mc, block, arg_vals, None)
            .map_err(|e| format!("block invocation: {e}"))?;
        results.push(vm.handle_table.mint_local(result, epoch));
    }
    Ok(results)
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

/// Drive one extension call to completion: open a call epoch, optionally mint a call-local
/// handle for a host `block` the extension may invoke, send the `Call`, then service the
/// re-entrant host-op conversation until the terminal `CallReturn`. The epoch is closed out
/// unconditionally so the call's transient handles (including the block) never leak.
fn extension_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    id: StreamId,
    op: String,
    argv: String,
    block: Option<Value<'gc>>,
) -> Result<String, QuoinError> {
    let epoch = vm.handle_table.begin_call();
    let block_handle = match block {
        Some(value) => vm.handle_table.mint_local(value, epoch),
        None => crate::handle_table::NULL_HANDLE,
    };

    let outcome: Result<String, QuoinError> = (|| {
        write_msg(
            vm,
            id,
            &Msg::Call {
                op,
                arg: argv,
                block: block_handle,
            },
        )?;
        loop {
            let frame = read_reply_frame(vm, id)?;
            let msg = quoin_ext_proto::decode_envelope(&frame)
                .map_err(|e| QuoinError::Other(format!("Extension call: malformed frame: {e}")))?;
            match msg {
                Msg::CallReturn { result } => return Ok(result),
                host_op => service_host_op(vm, mc, id, epoch, host_op)?,
            }
        }
    })();

    vm.handle_table.end_call(epoch);
    outcome
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
            let result = extension_call(vm, mc, id, op, argv, None)?;
            Ok(vm.new_string(mc, result))
        })
        // `ext call: '<op>' with: '<arg>' block: { ... }` -> like `call:with:`, but also hands
        // the block to the extension as a (call-local) handle it can invoke over a batch during
        // the call (Slice 4). The block is any value; non-blocks simply won't be invocable.
        .instance_method("call:with:block:", |vm, mc, receiver, args| {
            let id = receiver
                .with_native_state::<NativeExtension, _, _>(|e| e.id)
                .map_err(QuoinError::Other)?;
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            let block = *args
                .get(2)
                .ok_or_else(|| QuoinError::Other("call:with:block: missing block".to_string()))?;
            let result = extension_call(vm, mc, id, op, argv, Some(block))?;
            Ok(vm.new_string(mc, result))
        })
}
