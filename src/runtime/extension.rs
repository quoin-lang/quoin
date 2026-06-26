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
//! (Slice 4 — invoke a host *block* handle over a batch of argument tuples in one round-trip).
//! Every frame is a FlatBuffers `Message` union (schema/codec in `quoin-ext-proto`) inside a u32
//! length-prefixed frame.
//!
//! Slice 5b makes handles general `Call` arguments: `call:with:args:` passes a list whose elements
//! become either host-value handles (`Call.handles` — a block is one of these; the Slice-4 `block`
//! field is gone) or, for an `ExtResource`, the ext-side resource id (`Call.resources`). The mirror
//! direction — **ext-resource handles** — lets a call return an ext-owned resource
//! (`CallReturnResource`) which the host holds as an opaque `ExtResource` token and reaps on drop
//! (batched onto the next call's `Call.releases`, since a GC `Drop` can't send a frame).
//!
//! Slice 6b adds the **bulk data plane**: an `Array` arg routes into `Call.arrays` (copy-through, a
//! 3rd arg kind) and a call can return a bulk column via `CallReturnArray`, reconstructed host-side
//! as an `Array`. Whole columns cross the boundary as one buffer — never exploded into per-element
//! Values.
//!
//! Slice 5a adds **crash isolation**: a call whose I/O fails because the child exited surfaces a
//! typed `IoError` (not a hang), marks the extension dead so later calls fail fast, and `Drop`
//! reaps the host-side fd via the shared reap queue. A later slice adds **per-peer handle
//! bulk-release** (a dead/dropped extension's retained handles are freed via `release_for_ext`).
//!
//! **Timeouts** reuse the general `Async.timeout:do:` combinator (it aborts the parked socket
//! read and raises a catchable `TimeoutError`); the only extension-specific part is that a
//! cancelled (timed-out) call leaves the framed conversation desynced, so the extension is marked
//! dead — its connection can't be safely reused.
//!
//! **Structured values** (Phase 1): `call:with:data:` passes a Quoin value serialized to a
//! `DataValue` tree (`Call.data`), and a call may return one (`CallReturnData`), materialized back
//! into a nested Quoin Value. Both directions reuse the existing `value_to_data` / `data_to_value`
//! bridges (the latter via a `HostCtx` over the legacy `&mut VmState`).

use std::any::Any;
use std::cell::RefCell;
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gc_arena::collect::Trace;

use quoin_ext_proto::{ArrowArray, ArrowDType, DataValue as WireData, Msg};

use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::HostCtx;
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::runtime::array::{self, ArrayDType};
use crate::runtime::data_value::{DataValue as RtData, data_to_value, value_to_data};
use crate::runtime::list::NativeListState;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

/// Convert the wire `DataValue` to the runtime `DataValue` (decimal-string BigInt/Decimal are
/// parsed back to arbitrary precision), so the existing `data_to_value` bridge can materialize it.
fn wire_to_runtime(dv: &WireData) -> Result<RtData, QuoinError> {
    Ok(match dv {
        WireData::Null => RtData::Null,
        WireData::Bool(b) => RtData::Bool(*b),
        WireData::Int(i) => RtData::Int(*i),
        WireData::BigInt(s) => RtData::BigInt(
            s.parse()
                .map_err(|_| QuoinError::Other(format!("extension: invalid BigInt {s:?}")))?,
        ),
        WireData::Float(f) => RtData::Float(*f),
        WireData::Decimal(s) => RtData::Decimal(
            s.parse()
                .map_err(|_| QuoinError::Other(format!("extension: invalid Decimal {s:?}")))?,
        ),
        WireData::Str(s) => RtData::Str(s.clone()),
        WireData::Bytes(b) => RtData::Bytes(b.clone()),
        WireData::List(items) => RtData::Array(
            items
                .iter()
                .map(wire_to_runtime)
                .collect::<Result<_, _>>()?,
        ),
        WireData::Map(entries) => RtData::Object(
            entries
                .iter()
                .map(|(k, v)| Ok((k.clone(), wire_to_runtime(v)?)))
                .collect::<Result<_, QuoinError>>()?,
        ),
    })
}

/// Convert the runtime `DataValue` to the wire form (BigInt/Decimal as decimal strings).
fn runtime_to_wire(dv: &RtData) -> WireData {
    match dv {
        RtData::Null => WireData::Null,
        RtData::Bool(b) => WireData::Bool(*b),
        RtData::Int(i) => WireData::Int(*i),
        RtData::BigInt(n) => WireData::BigInt(n.to_string()),
        RtData::Float(f) => WireData::Float(*f),
        RtData::Decimal(d) => WireData::Decimal(d.to_string()),
        RtData::Str(s) => WireData::Str(s.clone()),
        RtData::Bytes(b) => WireData::Bytes(b.clone()),
        RtData::Array(items) => WireData::List(items.iter().map(runtime_to_wire).collect()),
        RtData::Object(entries) => WireData::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), runtime_to_wire(v)))
                .collect(),
        ),
    }
}

/// Bridge the host-side `Array` dtype to the wire `ArrowDType`.
fn to_wire_dtype(d: ArrayDType) -> ArrowDType {
    match d {
        ArrayDType::Float64 => ArrowDType::Float64,
        ArrayDType::Int64 => ArrowDType::Int64,
    }
}

/// Bridge the wire `ArrowDType` back to the host-side `Array` dtype.
fn from_wire_dtype(d: ArrowDType) -> ArrayDType {
    match d {
        ArrowDType::Int64 => ArrayDType::Int64,
        // Unknown future dtypes fall back to Float64 (forward-compat; trusted peer, §4).
        _ => ArrayDType::Float64,
    }
}

/// Native state behind an `Extension` value: the registered stream id for the UDS, the child
/// process, its socket path (for cleanup), the shared fd-reap queue, whether the extension has
/// been observed dead, and the queue of ext-side resource ids dropped by the host (flushed to
/// the extension as `Call.releases`).
#[derive(Debug)]
pub struct NativeExtension {
    id: StreamId,
    /// A process-unique, never-reused id for this extension; tags the host-value handles it mints
    /// so they can be bulk-released when it dies or is dropped (`HandleTable::release_for_ext`).
    ext_id: u64,
    child: Child,
    sock_path: String,
    /// Shared clone of `VmState::socket_reap`; `Drop` enqueues `id` so the driver closes the
    /// host-side UDS fd (the reactor can't be touched from `Drop`). Mirrors `NativeSocket`.
    reap: Rc<RefCell<Vec<StreamId>>>,
    /// Shared clone of `VmState::ext_handle_reap`; `Drop` enqueues `ext_id` so the driver
    /// bulk-releases this extension's host-value handles (a GC `Drop` can't touch the table).
    handle_reap: Rc<RefCell<Vec<u64>>>,
    /// Set once the child has been observed exited, so further calls fail fast (crash isolation).
    dead: bool,
    /// Ext-side resource ids whose host `ExtResource` was dropped, awaiting flush to the
    /// extension as `Call.releases`. Cloned into each `ExtResource` this extension hands out so
    /// its `Drop` can enqueue here (a GC `Drop` can't send a frame; mirrors the fd-reap pattern).
    resource_reap: Rc<RefCell<Vec<u64>>>,
}

/// Native state behind an `ExtResource` value: an opaque token for a resource that lives in the
/// extension process. Holds the extension-assigned id and a clone of that extension's
/// `resource_reap` queue; `Drop` enqueues the id so the next `Call` tells the extension to free it.
#[derive(Debug)]
pub struct NativeExtResource {
    resource_id: u64,
    reap: Rc<RefCell<Vec<u64>>>,
}

impl AnyCollect for NativeExtResource {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl Drop for NativeExtResource {
    fn drop(&mut self) {
        self.reap.borrow_mut().push(self.resource_id);
    }
}

impl NativeExtension {
    /// If a call's I/O failed *because* the child exited, mark the extension dead and return a
    /// short description of how it exited; otherwise `None` (the failure was something else).
    /// `try_wait` is non-blocking, so this is cheap and only runs on the error path.
    fn note_if_exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.dead = true;
                Some(match status.code() {
                    Some(code) => format!("exited with status {code}"),
                    None => "terminated by signal".to_string(),
                })
            }
            _ => None,
        }
    }
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
        // Best-effort teardown: enqueue the host-side fd and this extension's handles for the
        // driver to reap, kill + reap the child, and remove the socket file.
        self.reap.borrow_mut().push(self.id);
        self.handle_reap.borrow_mut().push(self.ext_id);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

/// The typed error raised when an extension's process has died (during or before a call). Surfaces
/// to Quoin as an `IoError` of kind `#closed`, so it's catchable like any other I/O failure.
fn extension_dead_error(detail: &str) -> QuoinError {
    QuoinError::io_closed(format!("Extension process died ({detail})"))
}

/// A process-unique, never-reused extension id (used to tag and bulk-release handles).
fn unique_ext_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
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
    ext_id: u64,
    msg: Msg,
) -> Result<(), QuoinError> {
    let reply = match msg {
        Msg::MakeString { value } => {
            let v = vm.new_string(mc, value);
            let handle = vm.handle_table.mint_local(v, epoch, ext_id);
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
                    let handle = vm.handle_table.mint_local(result, epoch, ext_id);
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
            match invoke_block_batches(vm, mc, epoch, ext_id, block, &batches) {
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
    ext_id: u64,
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
        results.push(vm.handle_table.mint_local(result, epoch, ext_id));
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
    ext_id: u64,
    op: String,
    argv: String,
    args: Vec<Value<'gc>>,
    data_arg: Option<Value<'gc>>,
    releases: Vec<u64>,
) -> Result<CallOutcome, QuoinError> {
    // Serialize the optional structured-value payload before opening the call (no handles involved).
    let data = match data_arg {
        Some(value) => Some(runtime_to_wire(&value_to_data(value)?)),
        None => None,
    };

    let epoch = vm.handle_table.begin_call();

    // Route each arg by token space: an `ExtResource` passes its (ext-side) resource id; an
    // `Array` is serialized into the bulk data plane; any other value is minted a call-local
    // host-value handle (a block is one of these).
    let mut handles = Vec::new();
    let mut resources = Vec::new();
    let mut arrays = Vec::new();
    for value in args {
        if let Ok(resource_id) =
            value.with_native_state::<NativeExtResource, _, _>(|r| r.resource_id)
        {
            resources.push(resource_id);
        } else if let Some((dtype, data)) = array::array_parts(value) {
            let length = (data.len() / 8) as u64;
            arrays.push(ArrowArray {
                dtype: to_wire_dtype(dtype),
                length,
                data,
            });
        } else {
            handles.push(vm.handle_table.mint_local(value, epoch, ext_id));
        }
    }

    let outcome: Result<CallOutcome, QuoinError> = (|| {
        write_msg(
            vm,
            id,
            &Msg::Call {
                op,
                arg: argv,
                handles,
                resources,
                releases,
                arrays,
                data,
            },
        )?;
        loop {
            let frame = read_reply_frame(vm, id)?;
            let msg = quoin_ext_proto::decode_envelope(&frame)
                .map_err(|e| QuoinError::Other(format!("Extension call: malformed frame: {e}")))?;
            match msg {
                Msg::CallReturn { result } => return Ok(CallOutcome::Scalar(result)),
                Msg::CallReturnResource { resource } => return Ok(CallOutcome::Resource(resource)),
                Msg::CallReturnArray { array } => return Ok(CallOutcome::Array(array)),
                Msg::CallReturnData { value } => return Ok(CallOutcome::Data(value)),
                host_op => service_host_op(vm, mc, id, epoch, ext_id, host_op)?,
            }
        }
    })();

    vm.handle_table.end_call(epoch);
    outcome
}

/// How a call finished: a scalar string, an ext-side resource the host will wrap as a token, a
/// bulk `Array` (the data plane), or a structured value (materialized as a nested Quoin Value).
enum CallOutcome {
    Scalar(String),
    Resource(u64),
    Array(ArrowArray),
    Data(WireData),
}

/// Wrap an ext-assigned resource id in an `ExtResource` value, tied to `reap` so its `Drop`
/// enqueues the id for release on this extension's next call.
fn make_ext_resource<'gc>(
    vm: &VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    resource_id: u64,
    reap: Rc<RefCell<Vec<u64>>>,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "ExtResource");
    vm.new_native_state(mc, class, NativeExtResource { resource_id, reap })
}

/// The shared body of the `call:` selectors: fail fast if the extension is already known dead,
/// flush dropped-resource releases, run the call, wrap the result (scalar or resource), and — if
/// the I/O failed because the child exited — surface a typed "died" error and mark it dead.
fn run_extension_method<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    op: String,
    argv: String,
    args: Vec<Value<'gc>>,
    data_arg: Option<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    // One peek at the native state: stream id, owning ext-id, liveness, the reap queue (for
    // releases + to clone into any returned resource), draining the pending releases to flush.
    let (id, ext_id, dead, resource_reap, releases) = receiver
        .with_native_state::<NativeExtension, _, _>(|e| {
            let releases: Vec<u64> = e.resource_reap.borrow_mut().drain(..).collect();
            (e.id, e.ext_id, e.dead, e.resource_reap.clone(), releases)
        })
        .map_err(QuoinError::Other)?;
    if dead {
        return Err(extension_dead_error("already exited"));
    }

    match extension_call(vm, mc, id, ext_id, op, argv, args, data_arg, releases) {
        Ok(CallOutcome::Scalar(result)) => Ok(vm.new_string(mc, result)),
        Ok(CallOutcome::Resource(resource_id)) => {
            Ok(make_ext_resource(vm, mc, resource_id, resource_reap))
        }
        Ok(CallOutcome::Array(array)) => Ok(array::new_array(
            vm,
            mc,
            from_wire_dtype(array.dtype),
            array.data,
        )),
        // Materialize a returned structured value into a nested Quoin Value via the existing
        // `data_to_value` bridge (`HostCtx` adapts the legacy `&mut VmState` to the `Host` surface).
        Ok(CallOutcome::Data(wire)) => {
            let rt = wire_to_runtime(&wire)?;
            let host = HostCtx::new(vm, mc);
            data_to_value(&rt, &host)
        }
        // A cancellation (a timeout via `Async.timeout:do:`, or a task cancel) interrupted the
        // call mid-conversation: the host abandoned a read, so the connection is desynced (a slow
        // extension's reply would arrive later, unread, and corrupt the next call). Mark the
        // extension dead + release its handles, then re-raise `Cancelled` unchanged so the timeout
        // combinator still turns it into a catchable `TimeoutError`. The peer is now unusable; the
        // program spawns a fresh `Extension` to retry.
        Err(QuoinError::Cancelled) => {
            let _ =
                receiver.with_native_state_mut::<NativeExtension, _, _>(mc, |ext| ext.dead = true);
            vm.handle_table.release_for_ext(ext_id);
            Err(QuoinError::Cancelled)
        }
        Err(e) => {
            let exit = receiver
                .with_native_state_mut::<NativeExtension, _, _>(mc, |ext| ext.note_if_exited())
                .ok()
                .flatten();
            match exit {
                // The child died: release the host-value handles it still held (its retained
                // globals) so they drop their GC roots instead of leaking until VM exit.
                Some(detail) => {
                    vm.handle_table.release_for_ext(ext_id);
                    Err(extension_dead_error(&detail))
                }
                None => Err(e),
            }
        }
    }
}

/// Extract the elements of a Quoin list value passed as the `args:` argument.
fn extract_args<'gc>(value: Value<'gc>) -> Result<Vec<Value<'gc>>, QuoinError> {
    value
        .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
        .map_err(|_| QuoinError::TypeError {
            expected: "List".to_string(),
            got: value.type_name().to_string(),
            msg: "call:with:args: expects a list of arguments".to_string(),
        })
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
                    ext_id: unique_ext_id(),
                    child,
                    sock_path,
                    reap: vm.socket_reap.clone(),
                    handle_reap: vm.ext_handle_reap.clone(),
                    dead: false,
                    resource_reap: Rc::new(RefCell::new(Vec::new())),
                },
            ))
        })
        // `ext call: '<op>' with: '<arg>'` -> send the `Call`, then service the conversation:
        // a loop of re-entrant host-ops the extension may issue (each answered inline) until it
        // sends the terminal `CallReturn`. Op + arg are strings; the result is a string or a
        // resource handle. No handle arguments.
        .instance_method("call:with:", |vm, mc, receiver, args| {
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            run_extension_method(vm, mc, receiver, op, argv, Vec::new(), None)
        })
        // `ext call: '<op>' with: '<arg>' args: #( v1 v2 … )` -> like `call:with:`, but also
        // passes typed handle arguments: each `ExtResource` in the list passes its resource id;
        // every other value (a block, string, etc.) is minted a call-local host-value handle.
        .instance_method("call:with:args:", |vm, mc, receiver, args| {
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            let list = *args.get(2).ok_or_else(|| {
                QuoinError::Other("call:with:args: missing args list".to_string())
            })?;
            let call_args = extract_args(list)?;
            run_extension_method(vm, mc, receiver, op, argv, call_args, None)
        })
        // `ext call: '<op>' with: '<arg>' data: <value>` -> like `call:with:`, but also passes a
        // structured-value payload (any Quoin value, serialized to a `DataValue` tree). The
        // extension reads it as native structured data; the result may likewise be structured.
        .instance_method("call:with:data:", |vm, mc, receiver, args| {
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            let data = *args.get(2).ok_or_else(|| {
                QuoinError::Other("call:with:data: missing data value".to_string())
            })?;
            run_extension_method(vm, mc, receiver, op, argv, Vec::new(), Some(data))
        })
}
