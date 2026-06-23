use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoError, IoRequest, IoResult, StreamId};
use crate::runtime::sockets::consume_socket;
use crate::value::{AnyCollect, Block, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::collect::Trace;
use gc_arena::{Gc, Mutation};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// How many bytes a single fill pulls from the backend per `Read`.
const FILL_CHUNK: usize = 8192;

/// Native backing for a buffered stream (`ByteStream`; `StringStream` joins it in 6b).
/// Like `NativeSocket` it owns a `StreamId` into the backend registry — the conduit
/// (TCP/TLS/file) is irrelevant once the handle is open — plus a clone of the VM's reap
/// queue, and carries no `Gc` fields. The extra piece is `rbuf`: bytes read from the
/// conduit but not yet consumed by QN (read-ahead). The fd is reaped on close/collection
/// via the shared queue, exactly as for sockets. See `docs/ASYNC_ARCH.md` (Stage 6).
pub struct NativeStream {
    id: StreamId,
    reap: Rc<RefCell<Vec<StreamId>>>,
    closed: bool,
    rbuf: Vec<u8>,
}

impl NativeStream {
    fn id(&self) -> StreamId {
        self.id
    }

    fn is_closed(&self) -> bool {
        self.closed
    }

    /// Mark closed; returns the previous flag (so `reap_stream_handle` enqueues only on
    /// the first close — double-close is a no-op).
    fn mark_closed(&mut self) -> bool {
        std::mem::replace(&mut self.closed, true)
    }
}

impl std::fmt::Debug for NativeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NativeStream{{id:{} closed:{} buffered:{}}}",
            self.id.0,
            self.closed,
            self.rbuf.len()
        )
    }
}

impl AnyCollect for NativeStream {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields (rbuf is plain bytes)
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        // The reap backstop: a stream collected without an explicit close reaps its fd.
        // Same constraint as `NativeSocket` — Drop may only push the plain id.
        if !self.closed {
            self.reap.borrow_mut().push(self.id);
        }
    }
}

pub fn build_byte_stream_class() -> NativeClassBuilder {
    let builder = NativeClassBuilder::new("ByteStream", Some("Object"))
        // ByteStream.over: aSocket -> a buffered byte stream that *consumes* the socket:
        // the fd transfers to the stream and the socket is left closed (further ops on it
        // throw). Works over any conduit that is a `StreamId` — TcpSocket/TlsSocket today.
        .class_method("over:", |vm, mc, _r, args| {
            let id = consume_or_raise(vm, mc, args[0], "ByteStream.over:")?;
            Ok(make_byte_stream(vm, mc, id))
        })
        // ByteStream.over: aSocket do:{|st| ...} -> run the block with the stream, closing
        // it on every exit path (normal/throw/cancel); returns the block's value.
        .class_method("over:do:", |vm, mc, _r, args| {
            let id = consume_or_raise(vm, mc, args[0], "ByteStream.over:do:")?;
            let handle = make_byte_stream(vm, mc, id);
            let block = arg!(args, Block, 1);
            scope_stream(vm, mc, handle, block)
        });
    add_byte_stream_methods(builder)
}

/// Build a `ByteStream` handle over an already-open `id`, wired to the VM's reap queue.
/// `pub` so the socket classes' `byteStream` method (and, later, `[IO]File`) can construct
/// one after obtaining a `StreamId`.
pub fn make_byte_stream<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, id: StreamId) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "ByteStream");
    vm.new_native_state(
        mc,
        class,
        NativeStream {
            id,
            reap: vm.socket_reap.clone(),
            closed: false,
            rbuf: Vec::new(),
        },
    )
}

fn add_byte_stream_methods(builder: NativeClassBuilder) -> NativeClassBuilder {
    builder
        // read -> whatever is available right now: drain the buffer, or if empty do one
        // fill. Empty `Bytes` = EOF.
        .instance_method("read", |vm, mc, receiver, _args| {
            let id = open_stream_id(vm, mc, receiver)?;
            if buffered_len(receiver)? == 0 {
                fill_once(vm, mc, receiver, id)?;
            }
            let bytes = drain_up_to(mc, receiver, usize::MAX)?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // read:n -> up to n bytes (may be short; empty = EOF). Buffer first, then at most
        // one fill — POSIX-style, like `TcpSocket.read:`.
        .typed_instance_method("read:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_stream_id(vm, mc, receiver)?;
            let n = arg!(args, Int, 0).max(0) as usize;
            if buffered_len(receiver)? == 0 {
                fill_once(vm, mc, receiver, id)?;
            }
            let bytes = drain_up_to(mc, receiver, n)?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // peek:n -> up to n bytes *without* consuming them. Fills until the buffer holds n
        // bytes (or EOF). Lets a caller look ahead before deciding how to frame.
        .typed_instance_method("peek:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_stream_id(vm, mc, receiver)?;
            let n = arg!(args, Int, 0).max(0) as usize;
            while buffered_len(receiver)? < n {
                if fill_once(vm, mc, receiver, id)? {
                    break; // EOF: return whatever we have
                }
            }
            let bytes = peek_up_to(receiver, n)?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // readUntil:delim -> bytes up to and *including* the first occurrence of `delim`
        // (a String or Bytes). If the stream ends before `delim`, returns the remaining
        // bytes (without it) — the caller can detect the missing delimiter.
        .instance_method("readUntil:", |vm, mc, receiver, args| {
            let delim = delim_bytes(&args, 0)?;
            if delim.is_empty() {
                return Err(raise(vm, mc, "ByteStream.readUntil:: empty delimiter"));
            }
            let id = open_stream_id(vm, mc, receiver)?;
            loop {
                if let Some(end) = find_subsequence(receiver, &delim)? {
                    let bytes = drain_up_to(mc, receiver, end)?;
                    return Ok(vm.new_bytes(mc, bytes));
                }
                if fill_once(vm, mc, receiver, id)? {
                    // EOF before the delimiter: hand back the partial remainder.
                    let bytes = drain_up_to(mc, receiver, usize::MAX)?;
                    return Ok(vm.new_bytes(mc, bytes));
                }
            }
        })
        // readExactly:n -> exactly n bytes, or throw if the stream ends first.
        .typed_instance_method("readExactly:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_stream_id(vm, mc, receiver)?;
            let n = arg!(args, Int, 0).max(0) as usize;
            while buffered_len(receiver)? < n {
                if fill_once(vm, mc, receiver, id)? {
                    return Err(raise(
                        vm,
                        mc,
                        &format!("ByteStream.readExactly:: stream ended with fewer than {n} bytes"),
                    ));
                }
            }
            let bytes = drain_up_to(mc, receiver, n)?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // writeAll:bytes -> write all of `bytes` straight through to the conduit
        // (complete-or-throw); the buffer is read-side only. Returns nil.
        .typed_instance_method("writeAll:", &["Bytes"], |vm, mc, receiver, args| {
            let id = open_stream_id(vm, mc, receiver)?;
            let bytes = arg!(args, Bytes, 0).to_vec(); // owned, before the await
            match vm.await_io(IoRequest::Write { id, bytes })? {
                IoResult::Wrote(_) => Ok(vm.new_nil(mc)),
                IoResult::Err(e) => Err(raise_io(vm, mc, &e)),
                other => Err(unexpected("writeAll:", other)),
            }
        })
        // close -> close the stream (idempotent); its fd is reaped next scheduler turn and
        // any buffered-but-unread bytes are discarded. Further ops throw.
        .instance_method("close", |vm, mc, receiver, _args| {
            reap_stream_handle(vm, mc, receiver);
            Ok(vm.new_nil(mc))
        })
        // closed? -> whether the stream has been closed (or consumed by a higher layer).
        .instance_method("closed?", |vm, mc, receiver, _args| {
            let closed = receiver
                .with_native_state::<NativeStream, _, _>(|s| s.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
}

/// Pull one `Read` from the conduit into `rbuf`. Returns `true` at EOF (an empty read).
/// The borrow of native state is released around the await — `rbuf` is plain bytes, so
/// nothing `Gc` is held across the suspend (`no_gc_across_yield`).
fn fill_once<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    id: StreamId,
) -> Result<bool, QuoinError> {
    match vm.await_io(IoRequest::Read {
        id,
        max: FILL_CHUNK,
    })? {
        IoResult::Read(chunk) if chunk.is_empty() => Ok(true), // EOF
        IoResult::Read(chunk) => {
            receiver
                .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
                    s.rbuf.extend_from_slice(&chunk)
                })
                .map_err(QuoinError::Other)?;
            Ok(false)
        }
        IoResult::Err(e) => Err(raise_io(vm, mc, &e)),
        other => Err(unexpected("read", other)),
    }
}

/// Consume a socket into a `StreamId`, or throw if it was already closed / isn't a socket.
fn consume_or_raise<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    source: Value<'gc>,
    op: &str,
) -> Result<StreamId, QuoinError> {
    match consume_socket(mc, source)? {
        Some(id) => Ok(id),
        None => Err(raise(
            vm,
            mc,
            &format!("{op}: the source is already closed"),
        )),
    }
}

/// Run `block` with an open stream `handle`, closing it on every exit path; returns the
/// block's value. No suspend here, so neither `handle` nor the result is held across one.
fn scope_stream<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    handle: Value<'gc>,
    block: Gc<'gc, Block<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let result = vm.execute_block(mc, block, vec![handle], None);
    reap_stream_handle(vm, mc, handle);
    result
}

/// The `StreamId` of an open stream receiver, or a thrown error if it is closed.
fn open_stream_id<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
) -> Result<StreamId, QuoinError> {
    let (id, closed) = receiver
        .with_native_state::<NativeStream, _, _>(|s| (s.id(), s.is_closed()))
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(raise(vm, mc, "stream: operation on a closed stream"));
    }
    Ok(id)
}

fn buffered_len<'gc>(receiver: Value<'gc>) -> Result<usize, QuoinError> {
    receiver
        .with_native_state::<NativeStream, _, _>(|s| s.rbuf.len())
        .map_err(QuoinError::Other)
}

/// Remove and return up to `n` bytes from the front of the buffer.
fn drain_up_to<'gc>(
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    n: usize,
) -> Result<Vec<u8>, QuoinError> {
    receiver
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
            let take = n.min(s.rbuf.len());
            s.rbuf.drain(..take).collect()
        })
        .map_err(QuoinError::Other)
}

/// Return (a copy of) up to `n` bytes from the front of the buffer without consuming them.
fn peek_up_to<'gc>(receiver: Value<'gc>, n: usize) -> Result<Vec<u8>, QuoinError> {
    receiver
        .with_native_state::<NativeStream, _, _>(|s| {
            let take = n.min(s.rbuf.len());
            s.rbuf[..take].to_vec()
        })
        .map_err(QuoinError::Other)
}

/// Index one past the end of the first occurrence of `delim` in the buffer, or `None`.
fn find_subsequence<'gc>(receiver: Value<'gc>, delim: &[u8]) -> Result<Option<usize>, QuoinError> {
    receiver
        .with_native_state::<NativeStream, _, _>(|s| {
            s.rbuf
                .windows(delim.len())
                .position(|w| w == delim)
                .map(|pos| pos + delim.len())
        })
        .map_err(QuoinError::Other)
}

/// Mark a stream closed (idempotent) and enqueue its fd for the driver to reap.
fn reap_stream_handle<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, handle: Value<'gc>) {
    let to_reap = handle
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
            if s.mark_closed() { None } else { Some(s.id()) }
        })
        .ok()
        .flatten();
    if let Some(id) = to_reap {
        vm.socket_reap.borrow_mut().push(id);
    }
}

/// Extract the delimiter bytes from a `String` or `Bytes` argument.
fn delim_bytes<'gc>(args: &[Value<'gc>], idx: usize) -> Result<Vec<u8>, QuoinError> {
    if let Some(Value::Object(obj)) = args.get(idx) {
        let b = obj.borrow();
        match &b.payload {
            ObjectPayload::Bytes(bytes) => return Ok(bytes.to_vec()),
            ObjectPayload::String(s) => return Ok(s.as_bytes().to_vec()),
            _ => {}
        }
    }
    Err(QuoinError::TypeError {
        expected: "Bytes or String".to_string(),
        got: args
            .get(idx)
            .map(|v| v.type_name().to_string())
            .unwrap_or_else(|| "None".to_string()),
        msg: format!("Expected a Bytes or String delimiter at argument index {idx}"),
    })
}

/// Throw a (catchable) network error carrying the backend's message.
fn raise_io<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, e: &IoError) -> QuoinError {
    raise(vm, mc, &e.message)
}

/// Throw a (catchable) string exception (the Stage-3/5 error model).
fn raise<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>, msg: &str) -> QuoinError {
    let val = vm.new_string(mc, msg.to_string());
    vm.active_exception = Some(val);
    QuoinError::Thrown
}

fn unexpected(op: &str, got: IoResult) -> QuoinError {
    QuoinError::Other(format!("stream {op}: unexpected I/O result {got:?}"))
}
