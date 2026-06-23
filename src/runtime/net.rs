use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoError, IoRequest, IoResult, StreamId};
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// Native backing state for a `TcpSocket` handle. Holds only the integer `StreamId`
/// (the real stream lives in the backend registry, outside the arena) plus a clone of
/// the VM's reap queue. No `Gc` fields. On close/collection the fd is reaped: `close`
/// pushes the id directly; the `Drop` (collection of an un-closed handle) pushes it as
/// the backstop. The driver sync-closes drained ids. See `docs/ASYNC_ARCH.md`.
pub struct NativeTcpSocket {
    id: StreamId,
    reap: Rc<RefCell<Vec<StreamId>>>,
    closed: bool,
}

impl NativeTcpSocket {
    fn id(&self) -> StreamId {
        self.id
    }

    fn is_closed(&self) -> bool {
        self.closed
    }

    /// Mark closed; returns the previous `closed` flag (so a double-close is a no-op).
    fn mark_closed(&mut self) -> bool {
        std::mem::replace(&mut self.closed, true)
    }
}

impl std::fmt::Debug for NativeTcpSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NativeTcpSocket{{id:{} closed:{}}}",
            self.id.0, self.closed
        )
    }
}

impl AnyCollect for NativeTcpSocket {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields
}

impl Drop for NativeTcpSocket {
    fn drop(&mut self) {
        // The backstop: a handle collected without an explicit close reaps its fd.
        // Drop must not touch other `Gc` or the async backend — pushing the plain id
        // onto the non-GC queue is the only safe move.
        if !self.closed {
            self.reap.borrow_mut().push(self.id);
        }
    }
}

pub fn build_tcp_socket_class() -> NativeClassBuilder {
    NativeClassBuilder::new("TcpSocket", Some("Object"))
        // TcpSocket.connect:'host:port' -> a connected socket. The caller owns it
        // (close it, or rely on the GC reap backstop). DNS is resolved internally.
        .class_method("connect:", |vm, mc, _receiver, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            match vm.await_io(IoRequest::Connect { host, port })? {
                IoResult::Connected(id) => Ok(make_socket(vm, mc, id)),
                IoResult::Err(e) => Err(raise_io(vm, mc, &e)),
                other => Err(unexpected("connect:", other)),
            }
        })
        // TcpSocket.connect:'host:port' do:{|s| ...} -> run the block with the socket,
        // closing it on exit (normal, throw, or cancel); returns the block's value.
        .class_method("connect:do:", |vm, mc, _receiver, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            let handle = match vm.await_io(IoRequest::Connect { host, port })? {
                IoResult::Connected(id) => make_socket(vm, mc, id),
                IoResult::Err(e) => return Err(raise_io(vm, mc, &e)),
                other => return Err(unexpected("connect:do:", other)),
            };
            let block = arg!(args, Block, 1);
            let result = vm.execute_block(mc, block, vec![handle], None);
            // Close on every path. No await here, so the block's result/`handle` are
            // not held across a suspend (the only awaits were Connect, and any inside
            // the block, where `handle` is rooted via its frame).
            reap_handle(vm, mc, handle);
            result
        })
        // read:n -> up to n bytes as Bytes (empty = EOF). Throws on a closed socket
        // or an I/O error.
        .typed_instance_method("read:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_id(vm, mc, receiver)?;
            let n = arg!(args, Int, 0).max(0) as usize;
            match vm.await_io(IoRequest::Read { id, max: n })? {
                IoResult::Read(bytes) => Ok(vm.new_bytes(mc, bytes)),
                IoResult::Err(e) => Err(raise_io(vm, mc, &e)),
                other => Err(unexpected("read:", other)),
            }
        })
        // readAll -> read until EOF, returning all bytes as one Bytes.
        .instance_method("readAll", |vm, mc, receiver, _args| {
            let id = open_id(vm, mc, receiver)?;
            let mut all = Vec::new();
            loop {
                match vm.await_io(IoRequest::Read { id, max: 8192 })? {
                    IoResult::Read(chunk) if chunk.is_empty() => break, // EOF
                    IoResult::Read(chunk) => all.extend_from_slice(&chunk),
                    IoResult::Err(e) => return Err(raise_io(vm, mc, &e)),
                    other => return Err(unexpected("readAll", other)),
                }
            }
            Ok(vm.new_bytes(mc, all))
        })
        // writeAll:bytes -> write all of `bytes` (complete-or-throw); returns nil.
        .typed_instance_method("writeAll:", &["Bytes"], |vm, mc, receiver, args| {
            let id = open_id(vm, mc, receiver)?;
            let bytes = arg!(args, Bytes, 0).to_vec(); // owned, before the await
            match vm.await_io(IoRequest::Write { id, bytes })? {
                IoResult::Wrote(_) => Ok(vm.new_nil(mc)),
                IoResult::Err(e) => Err(raise_io(vm, mc, &e)),
                other => Err(unexpected("writeAll:", other)),
            }
        })
        // close -> close the socket (idempotent). The fd is reaped on the next
        // scheduler turn; further ops on this socket throw.
        .instance_method("close", |vm, mc, receiver, _args| {
            reap_handle(vm, mc, receiver);
            Ok(vm.new_nil(mc))
        })
        // closed? -> whether the socket has been closed.
        .instance_method("closed?", |vm, mc, receiver, _args| {
            let closed = receiver
                .with_native_state::<NativeTcpSocket, _, _>(|s| s.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
}

/// Build a `TcpSocket` handle over `id`, wired to the VM's reap queue.
fn make_socket<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, id: StreamId) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "TcpSocket");
    vm.new_native_state(
        mc,
        class,
        NativeTcpSocket {
            id,
            reap: vm.socket_reap.clone(),
            closed: false,
        },
    )
}

/// The `StreamId` of an open socket receiver, or a thrown error if it is closed.
fn open_id<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
) -> Result<StreamId, QuoinError> {
    let (id, closed) = receiver
        .with_native_state::<NativeTcpSocket, _, _>(|s| (s.id(), s.is_closed()))
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(raise(vm, mc, "TcpSocket: operation on a closed socket"));
    }
    Ok(id)
}

/// Mark a handle closed (idempotent) and enqueue its fd for the driver to reap.
fn reap_handle<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, handle: Value<'gc>) {
    let to_reap = handle
        .with_native_state_mut::<NativeTcpSocket, _, _>(mc, |s| {
            if s.mark_closed() { None } else { Some(s.id()) }
        })
        .ok()
        .flatten();
    if let Some(id) = to_reap {
        vm.socket_reap.borrow_mut().push(id);
    }
}

/// Split `host:port` (on the last `:`). IPv6 in bracketed form is a future refinement.
fn parse_host_port(s: &str) -> Result<(String, u16), QuoinError> {
    match s.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port.parse::<u16>().map_err(|_| {
                QuoinError::Other(format!("TcpSocket.connect:: bad port in '{}'", s))
            })?;
            Ok((host.to_string(), port))
        }
        _ => Err(QuoinError::Other(format!(
            "TcpSocket.connect:: expected 'host:port', got '{}'",
            s
        ))),
    }
}

/// Throw a (catchable) network error carrying the backend's message.
fn raise_io<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, e: &IoError) -> QuoinError {
    raise(vm, mc, &e.message)
}

/// Throw a (catchable) string exception (the Stage-3 error model; a structured
/// `IoError` class is a noted refinement).
fn raise<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, msg: &str) -> QuoinError {
    let val = vm.new_string(mc, msg.to_string());
    vm.active_exception = Some(val);
    QuoinError::Thrown
}

fn unexpected(op: &str, got: IoResult) -> QuoinError {
    QuoinError::Other(format!("TcpSocket.{op}: unexpected I/O result {got:?}"))
}
