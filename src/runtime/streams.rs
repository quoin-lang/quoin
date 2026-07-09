use crate::arg;
use crate::error::{IoErrorKind, QuoinError};
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::runtime::sockets::consume_socket;
use crate::value::{AnyCollect, Block, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::collect::Trace;
use gc_arena::{Gc, Mutation};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// Quoin's one I/O buffer size: how many bytes a single fill pulls from the backend per `Read`,
/// and how much a buffered write stream accumulates before it drains.
///
/// 16 KiB, measured rather than inherited. One `await_io` round trip costs ~270ns (a fiber park,
/// a scheduler pass, a backend poll) — an order more than the syscall it wraps — so the buffer
/// buys down that fixed cost. Reading a 64 MiB file, overhead above the 4.5 GB/s
/// memory-bandwidth floor is +28% at 4 KiB, +16% at 8 KiB, +9% at 16 KiB, +4% at 32 KiB, and
/// then *worsens* to +26% at 64 KiB — because the backend allocates and zeroes a fresh
/// `max`-byte Vec on every `Read` (`io_backend.rs`), which crosses the allocator's large-object
/// threshold there. That cliff is an artifact of the per-read allocation, not of the size, so
/// 16 KiB sits below it and stays right once the allocation is fixed.
///
/// The top end is set by memory, not throughput: this is also every *socket's* read-ahead, so a
/// server holding 10k connections holds 10k of these. 32 KiB would buy 4% and cost it 160 MB.
pub const IO_BUFFER_BYTES: usize = 16 * 1024;

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
    /// Bytes written by QN but not yet handed to the backend. Empty unless `wcap > 0`.
    wbuf: Vec<u8>,
    /// Write-buffer capacity; **0 means write-through**, which is what every socket gets.
    /// Buffering a socket would stall `[HTTP]Server`, which writes a response and then waits
    /// for the client. Only file write streams (`[IO]File.create:` / `append:`) buffer — the
    /// same split C stdio makes.
    wcap: usize,
}

impl NativeStream {
    fn id(&self) -> StreamId {
        self.id
    }

    fn is_buffered(&self) -> bool {
        self.wcap > 0
    }

    /// Whether this stream has been closed — the exit-flush registry drops closed streams.
    pub fn is_stream_closed(&self) -> bool {
        self.closed
    }

    /// The backend stream id, for the exit-flush registry.
    pub fn stream_id(&self) -> StreamId {
        self.id
    }

    /// Take the undrained bytes for the exit flush. `None` when there is nothing to write —
    /// including when the stream was closed (and therefore already flushed).
    pub fn take_pending(&mut self) -> Option<(StreamId, Vec<u8>)> {
        if self.closed || self.wbuf.is_empty() {
            return None;
        }
        Some((self.id, std::mem::take(&mut self.wbuf)))
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
        .construct_with("use ByteStream.over: (or streams from sockets/files)")
        // ByteStream.over: aSocket -> a buffered byte stream that *consumes* the socket:
        // the fd transfers to the stream and the socket is left closed (further ops on it
        // throw). Works over any conduit that is a `StreamId` — TcpSocket/TlsSocket today.
        .class_method("over:", |vm, mc, _r, args| {
            let id = consume_or_raise(mc, args[0], "ByteStream.over:")?;
            Ok(make_byte_stream(vm, mc, id))
        })
        // ByteStream.over: aSocket do:{|st| ...} -> run the block with the stream, closing
        // it on every exit path (normal/throw/cancel); returns the block's value.
        .class_method("over:do:", |vm, mc, _r, args| {
            let id = consume_or_raise(mc, args[0], "ByteStream.over:do:")?;
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
    make_byte_stream_with(vm, mc, id, 0)
}

/// A `ByteStream` that *buffers* writes: `[IO]File.create:` / `append:`. The caller must also
/// register it with `VmState::track_write_stream` so an unclosed stream is flushed at exit.
pub fn make_write_byte_stream<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    id: StreamId,
) -> Value<'gc> {
    make_byte_stream_with(vm, mc, id, IO_BUFFER_BYTES)
}

fn make_byte_stream_with<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    id: StreamId,
    wcap: usize,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "ByteStream");
    vm.new_native_state(
        mc,
        class,
        NativeStream {
            id,
            reap: vm.io.socket_reap.clone(),
            closed: false,
            rbuf: Vec::new(),
            wbuf: Vec::new(),
            wcap,
        },
    )
}

fn add_byte_stream_methods(builder: NativeClassBuilder) -> NativeClassBuilder {
    builder
        // read -> whatever is available right now: drain the buffer, or if empty do one
        // fill. Empty `Bytes` = EOF.
        .instance_method("read", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            if buffered_len(receiver)? == 0 {
                fill_once(vm, mc, receiver, id)?;
            }
            let bytes = drain_up_to(mc, receiver, usize::MAX)?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // read:n -> up to n bytes (may be short; empty = EOF). Buffer first, then at most
        // one fill — POSIX-style, like `TcpSocket.read:`.
        .typed_instance_method("read:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_stream_id(receiver)?;
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
            let id = open_stream_id(receiver)?;
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
                return Err(QuoinError::ValueError(
                    "ByteStream.readUntil:: empty delimiter".to_string(),
                ));
            }
            let id = open_stream_id(receiver)?;
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
        // readUntil:delim limit:n -> like readUntil:, but throws (IoError, kind
        // #limitExceeded) once more than `n` bytes are buffered with no delimiter among
        // them — bounding hostile delimiter-less input instead of buffering it without
        // end. A delimiter found within the buffer returns normally (the caller enforces
        // any policy on the returned line's length); EOF still returns the partial rest.
        .instance_method("readUntil:limit:", |vm, mc, receiver, args| {
            let delim = delim_bytes(&args, 0)?;
            if delim.is_empty() {
                return Err(QuoinError::ValueError(
                    "ByteStream.readUntil:limit:: empty delimiter".to_string(),
                ));
            }
            let limit = arg!(args, Int, 1).max(0) as usize;
            let id = open_stream_id(receiver)?;
            loop {
                if let Some(end) = find_subsequence(receiver, &delim)? {
                    let bytes = drain_up_to(mc, receiver, end)?;
                    return Ok(vm.new_bytes(mc, bytes));
                }
                if buffered_len(receiver)? > limit {
                    return Err(QuoinError::io(
                        IoErrorKind::LimitExceeded,
                        format!("ByteStream.readUntil:limit:: no delimiter within {limit} bytes"),
                    ));
                }
                if fill_once(vm, mc, receiver, id)? {
                    // EOF before the delimiter: hand back the partial remainder.
                    let bytes = drain_up_to(mc, receiver, usize::MAX)?;
                    return Ok(vm.new_bytes(mc, bytes));
                }
            }
        })
        // readAll -> read until EOF, returning all remaining bytes as one Bytes.
        .instance_method("readAll", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            while !fill_once(vm, mc, receiver, id)? {}
            let all = drain_up_to(mc, receiver, usize::MAX)?;
            Ok(vm.new_bytes(mc, all))
        })
        // readExactly:n -> exactly n bytes, or throw if the stream ends first.
        .typed_instance_method("readExactly:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_stream_id(receiver)?;
            let n = arg!(args, Int, 0).max(0) as usize;
            while buffered_len(receiver)? < n {
                if fill_once(vm, mc, receiver, id)? {
                    return Err(QuoinError::io(
                        IoErrorKind::UnexpectedEof,
                        format!("ByteStream.readExactly:: stream ended with fewer than {n} bytes"),
                    ));
                }
            }
            let bytes = drain_up_to(mc, receiver, n)?;
            Ok(vm.new_bytes(mc, bytes))
        })
        // writeAll:bytes -> write all of `bytes` (complete-or-throw). On a socket this goes
        // straight through; on a buffered file stream it lands in the write buffer and drains
        // once the buffer fills. Returns nil.
        .typed_instance_method("writeAll:", &["Bytes"], |vm, mc, receiver, args| {
            let bytes = arg!(args, Bytes, 0).to_vec(); // owned, before any await
            stream_write(vm, mc, receiver, bytes)?;
            Ok(vm.new_nil(mc))
        })
        // flush! -> hand any buffered bytes to the OS now. A no-op on a write-through stream
        // (every socket), so the same code works over a file and a socket. Returns nil.
        .instance_method("flush!", |vm, mc, receiver, _args| {
            stream_flush(vm, mc, receiver)?;
            Ok(vm.new_nil(mc))
        })
        // close -> close the stream (idempotent); its fd is reaped next scheduler turn and
        // any buffered-but-unread bytes are discarded. Further ops throw.
        .instance_method("close", |vm, mc, receiver, _args| {
            stream_flush(vm, mc, receiver)?;
            vm.untrack_write_stream(mc, receiver);
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
        // stringStream -> a text `StringStream` that *consumes* this byte stream: the fd
        // and any buffered read-ahead transfer up; this handle is left closed.
        .instance_method("stringStream", |vm, mc, receiver, _args| {
            let parts = consume_stream_or_raise(mc, receiver, "stringStream")?;
            let handle = make_string_stream_from(vm, mc, parts);
            retrack_write_stream(vm, mc, receiver, handle)?;
            Ok(handle)
        })
}

pub fn build_string_stream_class() -> NativeClassBuilder {
    let builder = NativeClassBuilder::new("StringStream", Some("Object"))
        .construct_with("use StringStream.over:")
        // StringStream.over: aByteStream -> a text stream that *consumes* the byte stream
        // (its fd and buffered read-ahead transfer; the byte stream is left closed).
        .class_method("over:", |vm, mc, _r, args| {
            let parts = consume_stream_or_raise(mc, args[0], "StringStream.over:")?;
            let handle = make_string_stream_from(vm, mc, parts);
            retrack_write_stream(vm, mc, args[0], handle)?;
            Ok(handle)
        })
        .class_method("over:do:", |vm, mc, _r, args| {
            let parts = consume_stream_or_raise(mc, args[0], "StringStream.over:do:")?;
            let handle = make_string_stream_from(vm, mc, parts);
            retrack_write_stream(vm, mc, args[0], handle)?;
            let block = arg!(args, Block, 1);
            scope_stream(vm, mc, handle, block)
        });
    add_string_stream_methods(builder)
}

/// Build a `StringStream` over an open `id`, seeded with `rbuf` (read-ahead inherited from
/// a consumed `ByteStream`, or empty when wrapping a socket directly). `pub` so the socket
/// `stringStream` method can construct one.
pub fn make_string_stream<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    id: StreamId,
    rbuf: Vec<u8>,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "StringStream");
    vm.new_native_state(
        mc,
        class,
        NativeStream {
            id,
            reap: vm.io.socket_reap.clone(),
            closed: false,
            rbuf,
            wbuf: Vec::new(),
            wcap: 0,
        },
    )
}

fn add_string_stream_methods(builder: NativeClassBuilder) -> NativeClassBuilder {
    builder
        // readLine -> the next line as a String, with a trailing "\r\n" or "\n" stripped;
        // nil at EOF. An empty line returns ""; a final line without a newline is returned
        // once (then nil). Throws if a line is not valid UTF-8.
        .instance_method("readLine", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            match read_line(vm, mc, receiver, id)? {
                Some(line) => Ok(line),
                None => Ok(vm.new_nil(mc)),
            }
        })
        // eachLine:{|line| ...} -> run the block on each line to EOF; returns self.
        .instance_method("eachLine:", |vm, mc, receiver, args| {
            let block = arg!(args, Block, 0);
            each_line(vm, mc, receiver, block)?;
            Ok(receiver)
        })
        // read -> the largest valid-UTF-8 prefix of what's currently available, as a
        // String, retaining any trailing partial code point for the next read; empty
        // String = EOF. Throws if the stream ends mid-sequence or on a truly invalid byte.
        .instance_method("read", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            loop {
                if buffered_len(receiver)? == 0 && fill_once(vm, mc, receiver, id)? {
                    return Ok(vm.new_string(mc, String::new())); // EOF
                }
                let (valid, hard_invalid) = utf8_split(receiver)?;
                if valid > 0 {
                    let bytes = drain_up_to(mc, receiver, valid)?;
                    let s = decode_utf8(bytes, "read")?;
                    return Ok(vm.new_string(mc, s));
                }
                // No valid leading bytes: either a definitively-invalid byte, or an
                // incomplete code point that more reads might complete.
                if hard_invalid {
                    return Err(QuoinError::ParseError(
                        "StringStream.read: invalid UTF-8 byte".to_string(),
                    ));
                }
                if fill_once(vm, mc, receiver, id)? {
                    return Err(QuoinError::ParseError(
                        "StringStream.read: stream ended mid UTF-8 sequence".to_string(),
                    ));
                }
            }
        })
        // readAll -> the whole remaining stream as one String (throws on invalid UTF-8).
        .instance_method("readAll", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            while !fill_once(vm, mc, receiver, id)? {}
            let all = drain_up_to(mc, receiver, usize::MAX)?;
            let s = decode_utf8(all, "readAll")?;
            Ok(vm.new_string(mc, s))
        })
        // write:text -> the String's UTF-8 bytes. Buffered on a file stream, straight through
        // on a socket. Returns nil.
        .typed_instance_method("write:", &["String"], |vm, mc, receiver, args| {
            let text = arg!(args, String, 0).to_string();
            stream_write(vm, mc, receiver, text.into_bytes())?;
            Ok(vm.new_nil(mc))
        })
        // writeln:text -> `write:` plus a trailing newline. The line-oriented half of the
        // filter idiom: `[IO]Stdin.eachLine:{ |l| out.writeln:l }`.
        .typed_instance_method("writeln:", &["String"], |vm, mc, receiver, args| {
            let mut text = arg!(args, String, 0).to_string();
            text.push('\n');
            stream_write(vm, mc, receiver, text.into_bytes())?;
            Ok(vm.new_nil(mc))
        })
        // flush! -> hand any buffered bytes to the OS now; a no-op on a write-through stream.
        .instance_method("flush!", |vm, mc, receiver, _args| {
            stream_flush(vm, mc, receiver)?;
            Ok(vm.new_nil(mc))
        })
        .instance_method("close", |vm, mc, receiver, _args| {
            stream_flush(vm, mc, receiver)?;
            vm.untrack_write_stream(mc, receiver);
            reap_stream_handle(vm, mc, receiver);
            Ok(vm.new_nil(mc))
        })
        .instance_method("closed?", |vm, mc, receiver, _args| {
            let closed = receiver
                .with_native_state::<NativeStream, _, _>(|s| s.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
}

/// Read one line (consuming through the next `\n`), or `None` at EOF. The `\n` and an
/// optional preceding `\r` are stripped; a final line without a newline is returned as-is.
/// Lines split across reads / multibyte code points split across reads are reassembled by
/// the buffer before decoding (a `\n` byte never falls inside a UTF-8 sequence).
fn read_line<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    id: StreamId,
) -> Result<Option<Value<'gc>>, QuoinError> {
    loop {
        if let Some(end) = find_subsequence(receiver, b"\n")? {
            let mut line = drain_up_to(mc, receiver, end)?;
            line.pop(); // the '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let s = decode_utf8(line, "readLine")?;
            return Ok(Some(vm.new_string(mc, s)));
        }
        if fill_once(vm, mc, receiver, id)? {
            // EOF: the remainder (if any) is the final, newline-less line.
            let rem = drain_up_to(mc, receiver, usize::MAX)?;
            if rem.is_empty() {
                return Ok(None);
            }
            let s = decode_utf8(rem, "readLine")?;
            return Ok(Some(vm.new_string(mc, s)));
        }
    }
}

/// Decode bytes as UTF-8, throwing a catchable error (named by `op`) on invalid input —
/// the same text-boundary policy as `Bytes.asString`.
fn decode_utf8(bytes: Vec<u8>, op: &str) -> Result<String, QuoinError> {
    String::from_utf8(bytes)
        .map_err(|_| QuoinError::ParseError(format!("StringStream.{op}: not valid UTF-8")))
}

/// `(valid_up_to, has_invalid_byte)` for the buffered bytes: how many leading bytes are
/// valid UTF-8, and whether the bytes at that boundary are *definitively* invalid (vs. a
/// merely-incomplete trailing sequence that more reads could complete).
fn utf8_split<'gc>(receiver: Value<'gc>) -> Result<(usize, bool), QuoinError> {
    receiver
        .with_native_state::<NativeStream, _, _>(|s| match std::str::from_utf8(&s.rbuf) {
            Ok(_) => (s.rbuf.len(), false),
            Err(e) => (e.valid_up_to(), e.error_len().is_some()),
        })
        .map_err(QuoinError::Other)
}

/// Everything a stream carries when handed from one layer to the next: the fd, the read-ahead,
/// and — for a buffered write stream — the bytes not yet drained plus the capacity that says it
/// buffers at all. All four transfer, so `([IO]File.create:p).stringStream` keeps its buffer
/// instead of silently dropping it.
pub struct StreamParts {
    id: StreamId,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    wcap: usize,
}

/// Consume a `ByteStream`, returning its parts and leaving it closed (the fd and the buffers
/// transfer up to a `StringStream`). The `ByteStream` analogue of `consume_socket`.
/// `Ok(None)` if already closed; errors only if `value` isn't a stream.
fn consume_stream<'gc>(
    mc: &Mutation<'gc>,
    value: Value<'gc>,
) -> Result<Option<StreamParts>, QuoinError> {
    value
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
            if s.is_closed() {
                None
            } else {
                let parts = StreamParts {
                    id: s.id(),
                    rbuf: std::mem::take(&mut s.rbuf), // hand the read-ahead upward
                    wbuf: std::mem::take(&mut s.wbuf), // ...and any undrained writes
                    wcap: s.wcap,
                };
                s.mark_closed(); // no reap: the fd moves into the string stream
                Some(parts)
            }
        })
        .map_err(QuoinError::Other)
}

fn consume_stream_or_raise<'gc>(
    mc: &Mutation<'gc>,
    source: Value<'gc>,
    op: &str,
) -> Result<StreamParts, QuoinError> {
    match consume_stream(mc, source)? {
        Some(parts) => Ok(parts),
        None => Err(QuoinError::io_closed(format!(
            "{op}: the source is already closed"
        ))),
    }
}

/// Rebuild a `StringStream` from a consumed byte stream's parts, preserving its write buffer.
fn make_string_stream_from<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    parts: StreamParts,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "StringStream");
    vm.new_native_state(
        mc,
        class,
        NativeStream {
            id: parts.id,
            reap: vm.io.socket_reap.clone(),
            closed: false,
            rbuf: parts.rbuf,
            wbuf: parts.wbuf,
            wcap: parts.wcap,
        },
    )
}

/// The exit-flush registry tracks *handles*, and `stringStream` retires one for another. Swap
/// the tracked handle so the survivor is the one flushed at exit. A no-op for write-through
/// streams, which are never tracked.
fn retrack_write_stream<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    old: Value<'gc>,
    new: Value<'gc>,
) -> Result<(), QuoinError> {
    let buffered = new
        .with_native_state::<NativeStream, _, _>(|s| s.is_buffered())
        .map_err(QuoinError::Other)?;
    if buffered {
        vm.untrack_write_stream(mc, old);
        vm.track_write_stream(new);
    }
    Ok(())
}

/// Write `bytes` to the stream. Write-through (`wcap == 0`) issues the `Write` immediately; a
/// buffered stream appends and drains only once the buffer reaches `wcap`, so a `writeln:` per
/// line does not cost a scheduler round trip each.
///
/// The native-state borrow is dropped before every await, and no `Gc` value is read after one
/// (`no_gc_across_yield` / `no_borrow_across_yield`).
fn stream_write<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    bytes: Vec<u8>,
) -> Result<(), QuoinError> {
    let id = open_stream_id(receiver)?;
    let cap = receiver
        .with_native_state::<NativeStream, _, _>(|s| s.wcap)
        .map_err(QuoinError::Other)?;

    if cap == 0 {
        return write_through(vm, id, bytes);
    }

    // Append, then drain in one `Write` once we reach the buffer size. A single write larger
    // than the buffer drains in one go rather than in `wcap`-sized pieces.
    let pending = receiver
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
            s.wbuf.extend_from_slice(&bytes);
            if s.wbuf.len() >= s.wcap {
                std::mem::take(&mut s.wbuf)
            } else {
                Vec::new()
            }
        })
        .map_err(QuoinError::Other)?;

    if pending.is_empty() {
        return Ok(());
    }
    write_through(vm, id, pending)
}

/// Drain a buffered stream's pending bytes. A no-op when nothing is pending, and on a
/// write-through stream (which never accumulates any).
fn stream_flush<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
) -> Result<(), QuoinError> {
    let closed = receiver
        .with_native_state::<NativeStream, _, _>(|s| s.is_closed())
        .map_err(QuoinError::Other)?;
    if closed {
        return Ok(()); // already flushed on the way out; `close` is idempotent
    }
    let id = open_stream_id(receiver)?;
    let pending = receiver
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| std::mem::take(&mut s.wbuf))
        .map_err(QuoinError::Other)?;
    if pending.is_empty() {
        return Ok(());
    }
    write_through(vm, id, pending)
}

/// One `Write` round trip, complete-or-throw.
fn write_through<'gc>(
    vm: &mut VmState<'gc>,
    id: StreamId,
    bytes: Vec<u8>,
) -> Result<(), QuoinError> {
    match vm.await_io(IoRequest::Write { id, bytes })? {
        IoResult::Wrote(_) => Ok(()),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("writeAll:", other)),
    }
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
        max: IO_BUFFER_BYTES,
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
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("read", other)),
    }
}

/// Consume a socket into a `StreamId`, or throw if it was already closed / isn't a socket.
fn consume_or_raise<'gc>(
    mc: &Mutation<'gc>,
    source: Value<'gc>,
    op: &str,
) -> Result<StreamId, QuoinError> {
    match consume_socket(mc, source)? {
        Some(id) => Ok(id),
        None => Err(QuoinError::io_closed(format!(
            "{op}: the source is already closed"
        ))),
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

/// Drive `block` over each line of `receiver` to EOF (the body of `eachLine:`).
///
/// GC-rooting: `block` comes from `arg!`, which reads the receiver+args snapshot pinned in
/// `active_native_args` for the whole native call, so it stays rooted; each `line` is handed
/// *into* `execute_block` (`vec![line]`), reachable through the callee frame across its
/// yields. Neither is held across a yield unrooted — hence the allow.
fn each_line<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    block: Gc<'gc, Block<'gc>>,
) -> Result<(), QuoinError> {
    let id = open_stream_id(receiver)?;
    while let Some(line) = read_line(vm, mc, receiver, id)? {
        vm.execute_block(mc, block, vec![line], None)?;
    }
    Ok(())
}

/// The `StreamId` of an open stream receiver, or a thrown error if it is closed.
fn open_stream_id<'gc>(receiver: Value<'gc>) -> Result<StreamId, QuoinError> {
    let (id, closed) = receiver
        .with_native_state::<NativeStream, _, _>(|s| (s.id(), s.is_closed()))
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(QuoinError::io_closed(
            "stream: operation on a closed stream",
        ));
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
        vm.io.socket_reap.borrow_mut().push(id);
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

fn unexpected(op: &str, got: IoResult) -> QuoinError {
    QuoinError::Other(format!("stream {op}: unexpected I/O result {got:?}"))
}
