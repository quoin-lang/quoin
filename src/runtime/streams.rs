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
/// 16 KiB, measured rather than inherited — twice. One `await_io` round trip costs ~270ns (a
/// fiber park, a scheduler pass, a backend poll) — an order more than the syscall it wraps —
/// so the buffer buys down that fixed cost. The fill buffer RECYCLES through
/// `Scheduler::read_scratch` (the request carries it out, the `IoResult::Read` vec carries it
/// back), so the steady state allocates and zeroes nothing per read — fixing that was worth
/// −12% whole-process reading a 64 MiB file (profiling/read-buffer-recycle/notes.md).
/// Re-measured post-fix, BIGGER fills lose outright: +13% at 32 KiB, +19% at 64 KiB on the
/// same read — the kernel → fill buffer → `rbuf` copy chain stays cache-resident at 16 KiB
/// and does not above it. (The pre-fix curve blamed the 64 KiB cliff on the per-read
/// allocation crossing the allocator's large-object threshold; the allocation is gone and the
/// cliff remains.) So 16 KiB is simply the right size, files and sockets alike.
///
/// Memory agrees: this is also every *socket's* read-ahead, so a server holding 10k
/// connections holds 10k of these.
pub const IO_BUFFER_BYTES: usize = 16 * 1024;

/// Native backing for a buffered stream (`ByteStream`; `StringStream` joins it in 6b).
/// Like `NativeSocket` it owns a `StreamId` into the backend registry — the conduit
/// (TCP/TLS/file) is irrelevant once the handle is open — plus a clone of the VM's reap
/// queue, and carries no `Gc` fields. The extra piece is `rbuf`: bytes read from the
/// conduit but not yet consumed by QN (read-ahead). The fd is reaped on close/collection
/// via the shared queue, exactly as for sockets. See `docs/internal/ASYNC_ARCH.md` (Stage 6).
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
    /// Whether anything has ever been written through this handle — the write-side
    /// twin of the `rbuf`-empty check in `codec_wrap`'s preconditions (a codec must
    /// see the stream from its first byte).
    wrote: bool,
    /// Wrapped in a write-side codec: `close` must FINISH the stream (park on
    /// `FinishStream`, driving the encoder's trailer-writing `poll_close`) rather
    /// than enqueue the fd for a drop-reap.
    needs_finish: bool,
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

/// The shared `codecWrap:` body — wrap the underlying registry stream in a named
/// codec (io_codecs.rs), in place. Preconditions keep it honest, per the codec's
/// side. A READ codec (gunzip) wraps an open, UNREAD read stream (read-ahead
/// already buffered raw bytes would be silently lost to the decoder; a
/// write-buffered file stream has no read side to transform). A WRITE codec
/// (gzip) mirrors that: an open, UNWRITTEN file write stream — sockets are
/// refused because they are bidirectional, and wrapping the write side would
/// sever reads through the encoder's write-only adapter.
fn codec_wrap<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    args: &[Value<'gc>],
    who: &str,
) -> Result<Value<'gc>, QuoinError> {
    let codec = arg!(args, String, 0);
    // The same early lookup the backend does, here for the SIDE — so the
    // preconditions match the codec and a typo'd name errs before any I/O.
    let (side, _) = crate::io_codecs::lookup(&codec).map_err(|e| QuoinError::from_io_error(&e))?;
    let (id, closed, buffered, wcap, wrote) = receiver
        .with_native_state::<NativeStream, _, _>(|s| {
            (s.id, s.closed, s.rbuf.len(), s.wcap, s.wrote)
        })
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(QuoinError::io(
            IoErrorKind::Closed,
            format!("{who}: the stream is closed"),
        ));
    }
    match side {
        crate::io_codecs::Side::Read => {
            if wcap > 0 {
                return Err(QuoinError::io(
                    IoErrorKind::InvalidInput,
                    format!("{who}: this is a write stream — '{codec}' wraps read streams"),
                ));
            }
            if buffered > 0 {
                return Err(QuoinError::io(
                    IoErrorKind::InvalidInput,
                    format!(
                        "{who}: the stream has already been read — wrap it before the first read"
                    ),
                ));
            }
        }
        crate::io_codecs::Side::Write => {
            if wcap == 0 {
                return Err(QuoinError::io(
                    IoErrorKind::InvalidInput,
                    format!("{who}: '{codec}' wraps file write streams ([IO]File.create:/append:)"),
                ));
            }
            if wrote {
                return Err(QuoinError::io(
                    IoErrorKind::InvalidInput,
                    format!(
                        "{who}: the stream has already been written — wrap it before the first \
                         write"
                    ),
                ));
            }
        }
    }
    match vm.await_io(IoRequest::WrapStream {
        id,
        codec: codec.to_string(),
    })? {
        IoResult::Connected(_) => {
            if side == crate::io_codecs::Side::Write {
                receiver
                    .with_native_state_mut::<NativeStream, _, _>(mc, |s| s.needs_finish = true)
                    .map_err(QuoinError::Other)?;
            }
            Ok(receiver)
        }
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "{who}: unexpected io result {other:?}"
        ))),
    }
}

pub fn build_byte_stream_class() -> NativeClassBuilder {
    let builder = NativeClassBuilder::new("ByteStream", Some("Object"))
        .construct_with("use ByteStream.over: (or streams from sockets/files)")
        .class_doc(
            "A buffered binary stream — the one reading/writing surface over every conduit: \
             files (`[IO]File.byteStream` / `create:` / `append:`), sockets \
             (`TcpSocket#byteStream` or `ByteStream.over:`), and stdin \
             (`[IO]Stdin.byteStream`).\n\n\
             Reads are buffered through 16 KiB of read-ahead and *park the task* rather than \
             blocking the scheduler. The read family: `read` (whatever is available), \
             `read:` (up to n), `readExactly:` (n or throw), `readUntil:` (delimiter \
             framing), `readAll` (to EOF), `peek:` (look without consuming). Writes: \
             `writeAll:` — straight through on a socket, buffered on a file write stream \
             (drained by `flush!` / `close` / program exit). For text, wrap it with \
             `stringStream`.",
        )
        // ByteStream.over: aSocket -> a buffered byte stream that *consumes* the socket:
        // the fd transfers to the stream and the socket is left closed (further ops on it
        // throw). Works over any conduit that is a `StreamId` — TcpSocket/TlsSocket today.
        .class_method("over:", |vm, mc, _r, args| {
            let id = consume_or_raise(mc, args[0], "ByteStream.over:")?;
            Ok(make_byte_stream(vm, mc, id))
        })
        .doc(
            "A buffered ByteStream that *consumes* a socket (TcpSocket or TlsSocket): the \
             connection transfers to the stream and the socket is left closed — further ops \
             on it throw. Equivalent to the socket's own `byteStream`.",
        )
        // ByteStream.over: aSocket do:{|st| ...} -> run the block with the stream, closing
        // it on every exit path (normal/throw/cancel); returns the block's value.
        .class_method("over:do:", |vm, mc, _r, args| {
            let id = consume_or_raise(mc, args[0], "ByteStream.over:do:")?;
            let handle = make_byte_stream(vm, mc, id);
            let block = arg!(args, Block, 1);
            scope_stream(vm, mc, handle, block)
        })
        .doc(
            "Like `over:`, but scoped: run the block with the stream and close it on every \
             exit path (normal, throw, or cancel); answers the block's value.",
        );
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
            wrote: false,
            needs_finish: false,
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
        .doc(
            "Whatever bytes are available right now: drain the buffer, or — when it is \
             empty — wait for one fill from the conduit. Empty Bytes means EOF.",
        )
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
        .doc(
            "Up to n bytes, POSIX-style: possibly fewer than asked (one fill at most), and \
             empty Bytes at EOF. For exactly-n-or-throw semantics use `readExactly:`.",
        )
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
        .doc(
            "Up to n bytes *without* consuming them — the same bytes remain for the next \
             read. Fills until the buffer holds n bytes (or EOF cuts it short). The tool for \
             looking ahead before deciding how to frame.",
        )
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
        .doc(
            "Bytes up to and *including* the first occurrence of the delimiter (a String or \
             Bytes; empty throws a ValueError). If the stream ends first, the remainder is \
             returned without it — a result not ending in the delimiter means EOF. For \
             untrusted input prefer `readUntil:limit:`, which bounds the search.",
        )
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
        .doc(
            "Like `readUntil:`, but throws (IoError, kind #limitExceeded) once more than \
             `limit` bytes are buffered with no delimiter among them — bounding hostile \
             delimiter-less input instead of buffering it without end. EOF still returns the \
             partial remainder.",
        )
        // readAll -> read until EOF, returning all remaining bytes as one Bytes.
        .instance_method("readAll", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            while !fill_once(vm, mc, receiver, id)? {}
            let all = drain_up_to(mc, receiver, usize::MAX)?;
            Ok(vm.new_bytes(mc, all))
        })
        .doc(
            "Read to EOF and answer everything as one Bytes. The whole remainder is held in \
             memory — for bounded reading of long streams use `read:` or `readUntil:limit:` \
             in a loop.",
        )
        .typed_instance_method("codecWrap:", &["String"], |vm, mc, receiver, args| {
            codec_wrap(vm, mc, receiver, &args, "codecWrap:")
        })
        .doc(
            "Wrap the stream in a named codec (see io_codecs.rs) IN PLACE — a read codec \
             transforms every later read, a write codec every later write; answers the \
             receiver. Prefer the sugar (`gunzip` / `gzip`). A read codec needs an open, \
             unread read stream; a write codec an open, unwritten file write stream (its \
             `close` then finishes the encoder). An unknown codec throws an \
             IoError.\n\n\
             ```\n\
             ([IO]File.open:'logs.gz').byteStream.gunzip.readAll    \"* the decompressed bytes\n\
             ```",
        )
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
        .doc(
            "Exactly n bytes, or an IoError (kind #unexpectedEof) if the stream ends first — \
             the reader for length-prefixed framing, where a short read is a protocol \
             error.",
        )
        // writeAll:bytes -> write all of `bytes` (complete-or-throw). On a socket this goes
        // straight through; on a buffered file stream it lands in the write buffer and drains
        // once the buffer fills. Returns nil.
        .typed_instance_method("writeAll:", &["Bytes"], |vm, mc, receiver, args| {
            let bytes = arg!(args, Bytes, 0).to_vec(); // owned, before any await
            stream_write(vm, mc, receiver, bytes)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Write all of the Bytes — complete or throw. Straight through on a socket \
             stream; on a buffered file write stream it lands in the write buffer, draining \
             in 16 KiB chunks (`flush!` / `close` drain the rest). Returns nil.",
        )
        // flush! -> hand any buffered bytes to the OS now. A no-op on a write-through stream
        // (every socket), so the same code works over a file and a socket. Returns nil.
        .instance_method("flush!", |vm, mc, receiver, _args| {
            stream_flush(vm, mc, receiver)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Hand any buffered written bytes to the OS now. A no-op on a write-through \
             stream (every socket), so the same code works over a file and a socket. \
             Returns nil.",
        )
        // close -> close the stream (idempotent); its fd is reaped next scheduler turn and
        // any buffered-but-unread bytes are discarded. Further ops throw.
        .instance_method("close", |vm, mc, receiver, _args| {
            close_stream(vm, mc, receiver)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Flush any buffered writes and close the stream (idempotent — a second close is \
             a no-op). On a write-codec stream (`gzip`) this also FINISHES the encoder — \
             the trailer that makes the output valid is written here, so close such a \
             stream deliberately; a failed finish throws. Unread buffered bytes are \
             discarded and further operations throw. Returns nil.",
        )
        // closed? -> whether the stream has been closed (or consumed by a higher layer).
        .instance_method("closed?", |vm, mc, receiver, _args| {
            let closed = receiver
                .with_native_state::<NativeStream, _, _>(|s| s.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
        .doc(
            "Whether the stream has been closed — including by being consumed into a \
             StringStream.",
        )
        // stringStream -> a text `StringStream` that *consumes* this byte stream: the fd
        // and any buffered read-ahead transfer up; this handle is left closed.
        .instance_method("stringStream", |vm, mc, receiver, _args| {
            let parts = consume_stream_or_raise(mc, receiver, "stringStream")?;
            let handle = make_string_stream_from(vm, mc, parts);
            retrack_write_stream(vm, mc, receiver, handle)?;
            Ok(handle)
        })
        .doc(
            "A text StringStream that *consumes* this byte stream: the connection, any \
             read-ahead, and any pending writes all transfer, and this handle is left closed \
             (further ops on it throw).",
        )
}

pub fn build_string_stream_class() -> NativeClassBuilder {
    let builder = NativeClassBuilder::new("StringStream", Some("Object"))
        .construct_with("use StringStream.over:")
        .class_doc(
            "A text stream: the UTF-8 view of a byte conduit, usually obtained from \
             `[IO]File#stringStream`, a socket's `stringStream`, or `[IO]Stdin`.\n\n\
             Reading is line-oriented — `readLine` (nil at EOF), `eachLine:`, `readAll` — \
             and throws a catchable ParseError on invalid UTF-8; multibyte characters split \
             across reads are reassembled. Writing is `write:` / `writeln:` with plain \
             Strings. Like every stream, reads park the task, not the scheduler.\n\n\
             ```\n\
             var out = ([IO]File.create:'/tmp/lines.txt').stringStream\n\
             out.writeln:'alpha'\n\
             out.close\n\
             var st = ([IO]File.open:'/tmp/lines.txt').stringStream\n\
             st.readLine     \"* -> 'alpha'\n\
             st.readLine     \"* -> nil\n\
             st.close\n\
             [IO]File.delete:'/tmp/lines.txt'\n\
             ```",
        )
        // StringStream.over: aByteStream -> a text stream that *consumes* the byte stream
        // (its fd and buffered read-ahead transfer; the byte stream is left closed).
        .class_method("over:", |vm, mc, _r, args| {
            let parts = consume_stream_or_raise(mc, args[0], "StringStream.over:")?;
            let handle = make_string_stream_from(vm, mc, parts);
            retrack_write_stream(vm, mc, args[0], handle)?;
            Ok(handle)
        })
        .doc(
            "A text stream that *consumes* a ByteStream: the connection, read-ahead, and any \
             pending writes transfer, and the byte stream is left closed. Equivalent to the \
             byte stream's own `stringStream`.",
        )
        .class_method("over:do:", |vm, mc, _r, args| {
            let parts = consume_stream_or_raise(mc, args[0], "StringStream.over:do:")?;
            let handle = make_string_stream_from(vm, mc, parts);
            retrack_write_stream(vm, mc, args[0], handle)?;
            let block = arg!(args, Block, 1);
            scope_stream(vm, mc, handle, block)
        })
        .doc(
            "Like `over:`, but scoped: run the block with the text stream and close it on \
             every exit path (normal, throw, or cancel); answers the block's value.",
        );
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
            wrote: false,
            needs_finish: false,
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
        .doc(
            "The next line as a String, its trailing `\\n` (or `\\r\\n`) stripped; nil at \
             EOF. An empty line answers `''`; a final line without a newline is returned \
             once, then nil. Throws a ParseError if the line is not valid UTF-8.",
        )
        // eachLine:{|line| ...} -> run the block on each line to EOF; returns self.
        .instance_method("eachLine:", |vm, mc, receiver, args| {
            let block = arg!(args, Block, 0);
            each_line(vm, mc, receiver, block)?;
            Ok(receiver)
        })
        .doc(
            "Run the block on each remaining line (terminators stripped, as `readLine`) \
             until EOF; answers self. The whole-file loop: \
             `st.eachLine:{ |line| ... }`.",
        )
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
        .doc(
            "Whatever text is available right now: the largest valid-UTF-8 prefix of the \
             buffered bytes, as a String (a trailing partial code point is kept back for the \
             next read). An empty String means EOF; a truly invalid byte, or EOF in the \
             middle of a character, throws a ParseError.",
        )
        // readAll -> the whole remaining stream as one String (throws on invalid UTF-8).
        .instance_method("readAll", |vm, mc, receiver, _args| {
            let id = open_stream_id(receiver)?;
            while !fill_once(vm, mc, receiver, id)? {}
            let all = drain_up_to(mc, receiver, usize::MAX)?;
            let s = decode_utf8(all, "readAll")?;
            Ok(vm.new_string(mc, s))
        })
        .doc(
            "Read to EOF and answer the whole remainder as one String (throws a ParseError \
             on invalid UTF-8). The one-line way to slurp a file:\n\n\
             ```\n\
             var out = [IO]File.create:'/tmp/notes.txt'\n\
             out.writeAll:'hi'.asBytes\n\
             out.close\n\
             ([IO]File.open:'/tmp/notes.txt').stringStream.readAll     \"* -> 'hi'\n\
             [IO]File.delete:'/tmp/notes.txt'\n\
             ```",
        )
        .typed_instance_method("codecWrap:", &["String"], |vm, mc, receiver, args| {
            codec_wrap(vm, mc, receiver, &args, "codecWrap:")
        })
        .doc(
            "Wrap the stream in a named codec IN PLACE (see the ByteStream twin) — later \
             reads decode the transformed bytes as UTF-8, later writes go through the \
             transform; answers the receiver. Prefer the sugar (`gunzip` / `gzip`).",
        )
        // write:text -> the String's UTF-8 bytes. Buffered on a file stream, straight through
        // on a socket. Returns nil.
        .typed_instance_method("write:", &["String"], |vm, mc, receiver, args| {
            let text = arg!(args, String, 0).to_string();
            stream_write(vm, mc, receiver, text.into_bytes())?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Write the String's UTF-8 bytes, without a trailing newline. Buffered on a file \
             write stream, straight through on a socket. Returns nil.",
        )
        // writeln:text -> `write:` plus a trailing newline. The line-oriented half of the
        // filter idiom: `[IO]Stdin.eachLine:{ |l| out.writeln:l }`.
        .typed_instance_method("writeln:", &["String"], |vm, mc, receiver, args| {
            let mut text = arg!(args, String, 0).to_string();
            text.push('\n');
            stream_write(vm, mc, receiver, text.into_bytes())?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "`write:` plus a trailing newline — the line-oriented half of the filter idiom \
             `[IO]Stdin.eachLine:{ |l| out.writeln:l }`. Returns nil.",
        )
        // flush! -> hand any buffered bytes to the OS now; a no-op on a write-through stream.
        .instance_method("flush!", |vm, mc, receiver, _args| {
            stream_flush(vm, mc, receiver)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Hand any buffered written bytes to the OS now; a no-op on a write-through \
             (socket) stream. Returns nil.",
        )
        .instance_method("close", |vm, mc, receiver, _args| {
            close_stream(vm, mc, receiver)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Flush any buffered writes and close the stream (idempotent). On a \
             write-codec stream (`gzip`) this also finishes the encoder (writing the \
             trailer); a failed finish throws. Further operations throw. Returns nil.",
        )
        .instance_method("closed?", |vm, mc, receiver, _args| {
            let closed = receiver
                .with_native_state::<NativeStream, _, _>(|s| s.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
        .doc("Whether the stream has been closed.")
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
    wrote: bool,
    needs_finish: bool,
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
                    wrote: s.wrote,
                    needs_finish: s.needs_finish, // the finish duty transfers too
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
            wrote: parts.wrote,
            needs_finish: parts.needs_finish,
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
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
            s.wrote = true; // seen by codec_wrap's write-side precondition
            s.wcap
        })
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
///
/// The fill buffer recycles through `sched.read_scratch`: it rides out in the request,
/// comes back as the `IoResult::Read` vec, and returns to the pool after the copy into
/// `rbuf` — the steady state allocates and zeroes nothing per read (the backend's
/// `resize` on a recycled full-fill buffer is a no-op). A buffer lost to an aborted op
/// is simply not recycled.
fn fill_once<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    id: StreamId,
) -> Result<bool, QuoinError> {
    let buf = vm.sched.read_scratch.pop().unwrap_or_default();
    match vm.await_io(IoRequest::Read {
        id,
        max: IO_BUFFER_BYTES,
        buf,
    })? {
        IoResult::Read(chunk) if chunk.is_empty() => {
            recycle_read_buf(vm, chunk);
            Ok(true) // EOF
        }
        IoResult::Read(chunk) => {
            receiver
                .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
                    s.rbuf.extend_from_slice(&chunk)
                })
                .map_err(QuoinError::Other)?;
            recycle_read_buf(vm, chunk);
            Ok(false)
        }
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("read", other)),
    }
}

/// Return a fill buffer to the scratch pool. Capped: the pool needs to cover the reads
/// parked at one moment, not the stream count — beyond that, capacity is just dropped.
fn recycle_read_buf(vm: &mut VmState<'_>, buf: Vec<u8>) {
    const POOL_CAP: usize = 8;
    if vm.sched.read_scratch.len() < POOL_CAP && buf.capacity() > 0 {
        vm.sched.read_scratch.push(buf);
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
    match result {
        // The normal exit gets the full close — flush, and finish a write codec
        // (its trailer error surfaces here rather than being swallowed).
        Ok(v) => {
            close_stream(vm, mc, handle)?;
            Ok(v)
        }
        // The throw/cancel path must not park again: enqueue the drop-reap as
        // before. The driver's drain still FINISHES a write-codec id (it asks
        // the backend), so even this path yields a valid archive — only bytes
        // still in the handle's write buffer go down with the error.
        Err(e) => {
            vm.untrack_write_stream(mc, handle);
            reap_stream_handle(vm, mc, handle);
            Err(e)
        }
    }
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

/// The full explicit close: flush buffered writes, untrack from the exit-flush
/// registry, then release the fd — by parking on `FinishStream` when the stream
/// was wrapped in a write-side codec (the encoder's `poll_close` writes its
/// trailer; a drop would truncate the output), by the ordinary drop-reap
/// otherwise. The one place `close` can throw is that finish — a trailer that
/// cannot be written IS data loss, and it surfaces here, once.
fn close_stream<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
) -> Result<(), QuoinError> {
    stream_flush(vm, mc, receiver)?;
    vm.untrack_write_stream(mc, receiver);
    let finish = receiver
        .with_native_state::<NativeStream, _, _>(|s| {
            (!s.is_closed() && s.needs_finish).then(|| s.id())
        })
        .map_err(QuoinError::Other)?;
    let Some(id) = finish else {
        reap_stream_handle(vm, mc, receiver);
        return Ok(());
    };
    // Mark closed BEFORE parking, so further sends throw and the handle's Drop
    // won't re-enqueue the id; the op owns the stream from here.
    receiver
        .with_native_state_mut::<NativeStream, _, _>(mc, |s| {
            s.mark_closed();
        })
        .map_err(QuoinError::Other)?;
    match vm.await_io(IoRequest::FinishStream { id })? {
        IoResult::Closed => Ok(()),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("close", other)),
    }
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
