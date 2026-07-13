//! Async I/O backend seam (Stage 0).
//!
//! This module is the bridge described in `ASYNC_ARCH.md`. The VM never touches a
//! runtime directly: a fiber will suspend with a plain-data [`IoRequest`] and the
//! scheduler fulfills it through an [`IoBackend`], handing back a plain-data
//! [`IoResult`]. Keeping the request/result types free of `Gc`/`Value` is what lets
//! them cross a fiber yield (the `no_gc_across_yield` lint enforces this elsewhere)
//! and what keeps the async surface area contained to the single scheduler `.await`.
//!
//! Stage 0 is self-contained: the types, a smol-backed native implementation, and a
//! mock for tests. No VM wiring yet — that is Stage 1.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
#[cfg(not(target_arch = "wasm32"))]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use futures_lite::{AsyncReadExt, AsyncWriteExt};

/// An opaque handle to a backend-owned resource. The QN side holds only this
/// integer (wrapped in a small GC object); the real stream lives in the backend
/// registry, outside the arena. See `ASYNC_ARCH.md` → *Resource model & lifecycle*.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StreamId(pub u64);

/// Any byte stream the backend can own: TCP today, TLS-over-TCP / Unix sockets /
/// pipes later. Keying the registry on this (rather than a concrete type) is why
/// the byte ops (`Read`/`Write`/`Close`) never grow as new stream kinds are added.
pub trait AsyncStream: futures_lite::AsyncRead + futures_lite::AsyncWrite + Unpin {}
impl<T: futures_lite::AsyncRead + futures_lite::AsyncWrite + Unpin> AsyncStream for T {}

/// Adapts a read-only source into the `AsyncStream` registry, whose entries must also be
/// `AsyncWrite` (every other conduit — socket, file — is bidirectional). Writes fail with
/// `Unsupported` rather than panicking or silently succeeding: `[IO]Stdin.write:'x'` is a
/// programmer error, and it should say so.
pub struct ReadOnlyStream<R>(pub R);

impl<R: futures_lite::AsyncRead + Unpin> futures_lite::AsyncRead for ReadOnlyStream<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<R: Unpin> futures_lite::AsyncWrite for ReadOnlyStream<R> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this stream is read-only",
        )))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// The write-only twin of [`ReadOnlyStream`], for a child process's stdin pipe:
/// reads fail with `Unsupported` (reading your own child's *input* is a programmer
/// error), writes pass through, and `Close` drops the pipe — which is how the
/// child sees EOF.
pub struct WriteOnlyStream<W>(pub W);

impl<W: futures_lite::AsyncWrite + Unpin> futures_lite::AsyncWrite for WriteOnlyStream<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_close(cx)
    }
}

impl<W: Unpin> futures_lite::AsyncRead for WriteOnlyStream<W> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this stream is write-only",
        )))
    }
}

/// A plain-data I/O error (no `std::io::Error` borrow of OS state, Clone-friendly).
#[derive(Clone, Debug)]
pub struct IoError {
    pub kind: std::io::ErrorKind,
    pub message: String,
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

/// A request a fiber wants the backend to fulfill. Plain data only — safe to carry
/// across a fiber yield. Byte ops are keyed on a [`StreamId`]; only *creation* ops
/// (`Connect`, and later `TlsWrap`/`Listen`/`UdpBind`) are resource-specific.
#[derive(Clone, Debug)]
pub enum IoRequest {
    /// Park for `ms` milliseconds (the simplest op — no socket; ideal Stage 1 proof).
    Sleep { ms: u64 },
    /// Open a TCP connection to `host:port`, resolving DNS internally; on success
    /// registers the stream and returns its id. Carrying `host`/`port` (rather than a
    /// pre-resolved `SocketAddr`) folds resolution into the one op — manual DNS is a
    /// future class. See `docs/internal/ASYNC_ARCH.md`.
    Connect { host: String, port: u16 },
    /// Connect to a unix-domain socket at `path`, registering the stream and returning
    /// its id. The host side of the out-of-process extension transport (Tier 1): an
    /// extension call is then just `Write`+`Read` on this stream. Mirrors `Connect`;
    /// the `UnixStream` drops into the same `AsyncStream` registry.
    ConnectUnix { path: String },
    /// Read up to `max` bytes. An empty result means EOF. `buf` is the buffer to fill —
    /// the backend `resize(max, 0)`s it, so `Vec::new()` always works, and a caller that
    /// recycles the returned `IoResult::Read` vec back through here pays no allocation
    /// and re-zeroes only the tail its last fill truncated (nothing, on a full fill).
    /// The recycle loop lives in `streams.rs::fill_once` + `Scheduler::read_scratch`;
    /// one-shot callers just pass `Vec::new()`.
    Read {
        id: StreamId,
        max: usize,
        buf: Vec<u8>,
    },
    /// Like `Read`, but the op fails with a `TimedOut` error if no bytes arrive within
    /// `ms` milliseconds. Used to bound reads that have no surrounding `Async.timeout:`
    /// (e.g. the extension handshake, which runs at spawn time), so a peer that accepts
    /// the socket but never replies cannot park the caller forever.
    ReadTimed {
        id: StreamId,
        max: usize,
        ms: u64,
        buf: Vec<u8>,
    },
    /// Write all of `bytes`.
    Write { id: StreamId, bytes: Vec<u8> },
    /// Offload a pure, self-contained CPU-bound job to the compute pool
    /// (docs/internal/CONCURRENCY_ARCH.md §4). The job's `Send + Sync` closure owns
    /// its detached inputs; the caller parks exactly as for IO.
    Compute(crate::compute::ComputeJob),
    /// Park until the next cross-worker message on this lane (a worker's
    /// inbox or a parent's view of a worker's outbox). The endpoint is
    /// plain `Send` data; resolving to `None` means the far side closed.
    WorkerRecv(async_channel::Receiver<crate::worker::WorkerMsg>),
    /// Like `WorkerRecv` with a deadline: `None` for BOTH closed and timed
    /// out (callers that care distinguish via the registry's liveness).
    WorkerRecvTimed {
        rx: async_channel::Receiver<crate::worker::WorkerMsg>,
        ms: u64,
    },
    /// Park until the worker's done lane reports (its unit finished or
    /// failed); resolving the lane closed means the worker vanished.
    WorkerJoin(async_channel::Receiver<Result<quoin_ext_proto::DataValue, String>>),
    /// Close and deregister the stream.
    Close { id: StreamId },
    /// Upgrade the stream at `id` to TLS *in place*: take it out of the registry, run
    /// the client handshake (`domain` is the SNI / certificate name), and put the
    /// resulting `TlsStream` back at the *same* id — so this composes for both the
    /// "TLS from byte zero" case (`Connect` then `TlsWrap`) and STARTTLS-style upgrade
    /// of an already-used plaintext socket. `insecure` skips certificate validation
    /// (local debugging only). On failure the underlying stream is dropped (fd closed)
    /// and the id is left vacant. Result reuses `Connected(id)`.
    TlsWrap {
        id: StreamId,
        domain: String,
        insecure: bool,
    },
    /// Open a file (read-only) and register it as a stream, returning its id — so a file
    /// reads through the same `ByteStream`/`StringStream` as a socket. `path` stays an
    /// `OsString` end to end (the QN String → path conversion happens at the `[IO]File`
    /// boundary, not here). Result reuses `Connected(id)`.
    OpenFile { path: OsString },
    /// Open a file for *writing* and register it as a stream. `append: false` truncates (or
    /// creates); `append: true` positions at the end. The stream that comes back is buffered
    /// on the VM side (`streams.rs`), so a `write:` per line does not cost a scheduler round
    /// trip each. Result reuses `Connected(id)`.
    OpenFileWrite { path: OsString, append: bool },
    /// Register the process's standard input as a stream, returning its id — so stdin reads
    /// through the same `ByteStream`/`StringStream` as a socket or a file, and *parks the task*
    /// instead of freezing the single-threaded scheduler.
    ///
    /// Backed by `blocking::Unblock`, not `async_io::Async`: `Async` needs a pollable fd, and
    /// redirected stdin (`qn app.qn < file`) is a regular file, which is not. `Unblock` reads on
    /// the blocking pool and works uniformly for a tty, a pipe, and a file. Result reuses
    /// `Connected(id)`.
    OpenStdin,
    /// Bind a listening TCP socket on `host:port` (`port` 0 = ephemeral). Registers the
    /// listener and returns `Listening { id, port }` with the actual bound port.
    Listen { host: String, port: u16 },
    /// Accept one connection from the listener `id`, registering the accepted stream and
    /// returning its `Connected(id)`. Parks until a peer connects.
    Accept { id: StreamId },
    /// One-shot subprocess: spawn, feed `input` to stdin (then close it), read stdout
    /// and stderr CONCURRENTLY (either alone can deadlock on a full pipe buffer), and
    /// wait for exit — the whole lifecycle inside this one op, nothing registered.
    /// The command is spawned `kill_on_drop`, so cancelling the parked task (an
    /// `Async.timeout:` firing) kills the child rather than leaking it.
    RunProcess {
        program: OsString,
        args: Vec<OsString>,
        /// Vars set ON TOP of the inherited environment (`Command::env` semantics).
        env: Option<Vec<(OsString, OsString)>>,
        dir: Option<OsString>,
        input: Option<Vec<u8>>,
    },
    /// Spawn a subprocess for streaming: the child registers in the child table
    /// (`ChildWait` / the sync signal ops address it by id) and its three pipes
    /// register as ordinary streams — stdout/stderr read like a socket, stdin is
    /// write-only (`Close` on it is how the child sees EOF). NOT kill-on-drop: the
    /// handle's reap queue owns the kill (unless detached).
    SpawnProcess {
        program: OsString,
        args: Vec<OsString>,
        env: Option<Vec<(OsString, OsString)>>,
        dir: Option<OsString>,
    },
    /// Park until the spawned child `id` exits; answers `ProcExited`. Exclusive —
    /// the wait holds the child's only mutable borrow, so a second concurrent wait
    /// on the same child errs rather than panicking (kill goes by pid and never
    /// borrows, so killing a waited-on child works and resolves the wait).
    ChildWait { id: u64 },
    /// Wrap the stream at `id` in a named codec IN PLACE (same id — the TlsWrap
    /// mechanics, made generic): take it out of the registry, pass it through the
    /// `io_codecs` factory table, put the transformed stream back. The codec table
    /// is the extension point — this op never grows another variant per codec.
    /// An unknown codec errs and leaves the original stream untouched.
    WrapStream { id: StreamId, codec: String },
    /// Close the stream at `id` COMPLETELY: take it out of the registry and drive
    /// its `poll_close` chain before dropping. The write-codec twin of the reap
    /// path's drop-close — a gzip encoder writes its final deflate block and
    /// trailer in `poll_close`, so dropping the fd instead truncates the output.
    /// Every stream the backend flagged at wrap time (`needs_finish`) is closed
    /// through here: explicitly (`close` parks on this op), by the reap drain, or
    /// by backend teardown.
    FinishStream { id: StreamId },
    /// Open `path` for RANDOM-ACCESS reading. The file lands in its own registry
    /// (same id space as streams — the listener precedent): seeking needs the
    /// concrete `async_fs::File`, and the `dyn AsyncStream` box erases
    /// `AsyncSeek`. Resolves `Opened { id, size }`, the size from an open-time
    /// stat (not a snapshot taken earlier — no staleness race).
    OpenFileRandom { path: OsString },
    /// Positioned read, pread-style — no cursor, so calls are independent and
    /// there is no hidden position state: up to `max` bytes starting at byte
    /// `offset`, short only at EOF. The file is leased for the op, like every
    /// stream op; a concurrent second read on the same id errs rather than
    /// interleaving seeks.
    ReadAt {
        id: StreamId,
        offset: u64,
        max: usize,
    },
    /// Resolve `host` to its addresses — the same getaddrinfo-on-the-blocking-pool
    /// trick `Connect` uses internally, exposed. Resolves `Resolved(ips)`
    /// (A + AAAA, resolver order, deduplicated) or an error for a name that
    /// doesn't resolve.
    Resolve { host: String },
    /// Reverse-resolve an IP to its hostname (getnameinfo, name required).
    /// Resolves `Resolved` with one name, or empty when the address has no
    /// mapping — an unmapped address is an ordinary answer, not an error.
    ResolveReverse { addr: String },
}

/// The plain-data outcome of an [`IoRequest`].
#[derive(Clone, Debug)]
pub enum IoResult {
    Slept,
    Connected(StreamId),
    /// A random-access file opened: its id plus the size from an open-time stat.
    Opened {
        id: StreamId,
        size: u64,
    },
    /// A `Resolve`/`ResolveReverse` answer: IP strings (forward, deduplicated,
    /// resolver order) or zero-or-one hostname (reverse).
    Resolved(Vec<String>),
    /// A bound listening socket: its id plus the *actual* local port (so a `:0`
    /// ephemeral bind is usable — the caller can read the port it got).
    Listening {
        id: StreamId,
        port: u16,
    },
    /// Bytes read; empty = EOF.
    Read(Vec<u8>),
    /// A `Compute` job finished: the pure function's own result. `Err` is
    /// the job's domain error (e.g. malformed gzip), NOT an IO failure —
    /// callers wrap it in the same error type their inline path uses.
    Computed(Result<crate::compute::ComputeOut, String>),
    /// The next cross-worker message, or `None` if the lane is closed and
    /// drained (the far side exited).
    WorkerMsg(Option<crate::worker::WorkerMsg>),
    /// A worker's terminal report: its unit's outcome, or an `Err` for the
    /// lane closing unreported (the worker vanished).
    WorkerDone(Result<quoin_ext_proto::DataValue, String>),
    Wrote(usize),
    Closed,
    /// A `RunProcess` finished: exit code (`None` when signal-terminated — then
    /// `signal` says which), and the child's complete output.
    ProcDone {
        code: Option<i32>,
        signal: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    /// A `SpawnProcess` succeeded: the child-table id, the OS pid, and the three
    /// pipes as registered streams.
    ProcSpawned {
        child: u64,
        pid: u32,
        stdin: StreamId,
        stdout: StreamId,
        stderr: StreamId,
    },
    /// A `ChildWait` resolved (same code/signal split as `ProcDone`).
    ProcExited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Err(IoError),
}

/// A boxed, single-threaded future. `'static` because each future owns the `Rc`
/// clones it needs (it borrows nothing from `&self`), which keeps the scheduler's
/// `FuturesUnordered` free of lifetime entanglement with the backend. Not `Send`:
/// the whole VM + scheduler runs on one thread (gc_arena is `!Send`), so we drive
/// these with a single-threaded `block_on`.
pub type IoFuture = Pin<Box<dyn Future<Output = IoResult>>>;

/// The seam. Object-safe so the VM can hold a `Box<dyn IoBackend>` and swap smol →
/// tokio → a WASM host backend without touching anything above this trait.
impl IoRequest {
    /// Human-readable park description for `VM.ps` / `$ps` ("what is this
    /// task waiting on"). Purely observability — keep it cheap and short.
    pub fn label(&self) -> String {
        match self {
            IoRequest::Sleep { ms } => format!("io: sleep {ms}ms"),
            IoRequest::Connect { host, port } => format!("io: connect {host}:{port}"),
            IoRequest::ConnectUnix { .. } => "io: connect unix".to_string(),
            IoRequest::Read { .. } => "io: read".to_string(),
            IoRequest::ReadTimed { ms, .. } => format!("io: read (timeout {ms}ms)"),
            IoRequest::Write { .. } => "io: write".to_string(),
            IoRequest::Compute(job) => format!("compute: {}", job.label),
            IoRequest::WorkerRecv(_) => "worker receive".to_string(),
            IoRequest::WorkerRecvTimed { ms, .. } => {
                format!("worker receive (timeout {ms}ms)")
            }
            IoRequest::WorkerJoin(_) => "worker join".to_string(),
            IoRequest::Close { .. } => "io: close".to_string(),
            IoRequest::TlsWrap { .. } => "io: tls handshake".to_string(),
            IoRequest::OpenFile { .. } => "io: open file".to_string(),
            IoRequest::OpenFileWrite { append, .. } => {
                if *append {
                    "io: open file (append)".to_string()
                } else {
                    "io: open file (write)".to_string()
                }
            }
            IoRequest::OpenStdin => "io: open stdin".to_string(),
            IoRequest::Listen { host, port } => format!("io: listen {host}:{port}"),
            IoRequest::Accept { .. } => "io: accept".to_string(),
            IoRequest::RunProcess { program, .. } => {
                format!("proc: run {}", program.to_string_lossy())
            }
            IoRequest::SpawnProcess { program, .. } => {
                format!("proc: spawn {}", program.to_string_lossy())
            }
            IoRequest::ChildWait { .. } => "proc: wait".to_string(),
            IoRequest::WrapStream { codec, .. } => format!("io: wrap {codec}"),
            IoRequest::FinishStream { .. } => "io: finish".to_string(),
            IoRequest::OpenFileRandom { .. } => "io: open file (random)".to_string(),
            IoRequest::ReadAt { .. } => "io: read at".to_string(),
            IoRequest::Resolve { host } => format!("dns: resolve {host}"),
            IoRequest::ResolveReverse { addr } => format!("dns: reverse {addr}"),
        }
    }
}

pub trait IoBackend {
    fn perform(&self, req: IoRequest) -> IoFuture;

    /// Synchronously close and deregister a stream (drop it → close the fd). Used by
    /// the reap path: the QN socket handle's `Drop`, and explicit/scope close, push a
    /// `StreamId` onto a non-GC queue the scheduler drains here — no `await`, no task
    /// context. Missing ids are a no-op, so double-close is harmless.
    fn close(&self, id: StreamId);

    /// Whether the stream at `id` was wrapped in a write-side codec and so must be
    /// closed through `FinishStream` (which drives its `poll_close`) rather than
    /// dropped — the reap drain asks this before choosing. Default: nothing does.
    fn needs_finish(&self, _id: StreamId) -> bool {
        false
    }

    /// Synchronously kill (if still running) and deregister a spawned child — the
    /// child twin of `close`, drained from the child-reap queue when an undetached
    /// Process handle is collected or the VM exits. Missing/exited ids are a no-op.
    fn reap_child(&self, _id: u64) {}

    /// Send a signal to a spawned child, by PID — never touching the `Child`, so it
    /// works (and resolves the wait) while a `ChildWait` is parked on it. A no-op
    /// once the exit is recorded, so a stale handle can't signal a recycled pid.
    fn child_signal(&self, _id: u64, _signal: i32) -> Result<(), IoError> {
        Err(IoError {
            kind: std::io::ErrorKind::Unsupported,
            message: "this backend has no subprocess support".to_string(),
        })
    }

    /// Whether a spawned child is still running — exact, not a pid probe: recorded
    /// exit → false; a parked wait (which by definition hasn't resolved) → true;
    /// otherwise a non-blocking `try_status`, recording an exit it discovers.
    fn child_running(&self, _id: u64) -> bool {
        false
    }

    /// Mark a child detached: neither the reap path nor backend teardown kills it.
    fn child_detach(&self, _id: u64) {}
}

// ---------------------------------------------------------------------------
// SmolBackend — the native implementation, on `async-io`. Split into its own
// `#[path]` child file (like `runner`'s `runner_*.rs` siblings) because it is the
// one part of this module that cannot exist on wasm32: the smol reactor needs
// pollable fds. The seam types above and `MockBackend` below stay universal.
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
#[path = "io_backend_smol.rs"]
mod io_backend_smol;
#[cfg(not(target_arch = "wasm32"))]
pub use io_backend_smol::SmolBackend;

/// The concrete backend `VmState` embeds (a concrete type, not `Box<dyn IoBackend>`,
/// so nothing changes in the native hot path). On wasm32 the slot is filled by the
/// canned-data `MockBackend`, but it is inert in practice: with no scheduler,
/// `await_io` raises its catchable "outside the VM scheduler" error before any
/// request reaches a backend. A real browser backend (in-memory files, virtual
/// stdin) would replace this alias *and* teach `await_io` a synchronous-completion
/// path — that pair is the seam a future slice plugs into.
#[cfg(not(target_arch = "wasm32"))]
pub type DefaultBackend = SmolBackend;
#[cfg(target_arch = "wasm32")]
pub type DefaultBackend = MockBackend;

// ---------------------------------------------------------------------------
// MockBackend — deterministic, no real I/O, for VM-level tests later.
// ---------------------------------------------------------------------------

/// A backend that fulfills requests from canned data instead of the OS. `Read`
/// pops the next queued buffer (EOF once drained); `Write` records the bytes. Lets
/// the whole networking stdlib be tested without a network.
#[derive(Default)]
pub struct MockBackend {
    reads: RefCell<VecDeque<Vec<u8>>>,
    writes: RefCell<Vec<Vec<u8>>>,
    next_id: Cell<u64>,
}

impl MockBackend {
    pub fn new() -> Self {
        MockBackend {
            reads: RefCell::new(VecDeque::new()),
            writes: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
        }
    }

    /// Queue bytes to be returned by the next `Read`.
    pub fn push_read(&self, bytes: Vec<u8>) {
        self.reads.borrow_mut().push_back(bytes);
    }

    /// All bytes handed to `Write`, in order.
    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.writes.borrow().clone()
    }
}

impl IoBackend for MockBackend {
    fn perform(&self, req: IoRequest) -> IoFuture {
        let result = match req {
            IoRequest::Sleep { .. } => IoResult::Slept,
            IoRequest::Compute(job) => IoResult::Computed(job.run()),
            IoRequest::WorkerRecv(rx) => IoResult::WorkerMsg(rx.try_recv().ok()),
            IoRequest::WorkerRecvTimed { rx, .. } => IoResult::WorkerMsg(rx.try_recv().ok()),
            IoRequest::WorkerJoin(rx) => {
                IoResult::WorkerDone(rx.try_recv().unwrap_or_else(|_| {
                    Err("worker vanished without reporting a result".to_string())
                }))
            }
            IoRequest::Connect { .. } | IoRequest::ConnectUnix { .. } => {
                let id = StreamId(self.next_id.get());
                self.next_id.set(self.next_id.get() + 1);
                IoResult::Connected(id)
            }
            IoRequest::Read { max, .. }
            | IoRequest::ReadTimed { max, .. }
            | IoRequest::ReadAt { max, .. } => {
                let mut buf = self.reads.borrow_mut().pop_front().unwrap_or_default();
                buf.truncate(max);
                IoResult::Read(buf)
            }
            IoRequest::OpenFileRandom { .. } => {
                let id = StreamId(self.next_id.get());
                self.next_id.set(self.next_id.get() + 1);
                IoResult::Opened { id, size: 0 }
            }
            IoRequest::Resolve { .. } | IoRequest::ResolveReverse { .. } => {
                IoResult::Resolved(Vec::new()) // no resolver in the mock
            }
            IoRequest::RunProcess { .. }
            | IoRequest::SpawnProcess { .. }
            | IoRequest::ChildWait { .. } => IoResult::Err(IoError {
                kind: std::io::ErrorKind::Unsupported,
                message: "the mock backend has no subprocess support".to_string(),
            }),
            IoRequest::WrapStream { .. } => IoResult::Err(IoError {
                kind: std::io::ErrorKind::Unsupported,
                message: "the mock backend has no stream codecs".to_string(),
            }),
            IoRequest::Write { bytes, .. } => {
                let n = bytes.len();
                self.writes.borrow_mut().push(bytes);
                IoResult::Wrote(n)
            }
            IoRequest::Close { .. } | IoRequest::FinishStream { .. } => IoResult::Closed,
            // No real handshake in the mock — the conduit keeps its id, as in the
            // native backend's in-place swap.
            IoRequest::TlsWrap { id, .. } => IoResult::Connected(id),
            IoRequest::OpenFile { .. } | IoRequest::OpenFileWrite { .. } | IoRequest::OpenStdin => {
                let id = StreamId(self.next_id.get());
                self.next_id.set(self.next_id.get() + 1);
                IoResult::Connected(id)
            }
            IoRequest::Listen { .. } => {
                let id = StreamId(self.next_id.get());
                self.next_id.set(self.next_id.get() + 1);
                IoResult::Listening { id, port: 0 }
            }
            IoRequest::Accept { .. } => {
                let id = StreamId(self.next_id.get());
                self.next_id.set(self.next_id.get() + 1);
                IoResult::Connected(id)
            }
        };
        Box::pin(async move { result })
    }

    fn close(&self, _id: StreamId) {} // no fds in the mock
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_io::Timer;
    use futures_lite::future::block_on;
    use io_backend_smol::secure_connector;
    use std::time::Duration;

    #[test]
    fn mock_read_returns_canned_bytes_then_eof() {
        let mock = MockBackend::new();
        mock.push_read(b"hello".to_vec());

        let r = block_on(mock.perform(IoRequest::Read {
            id: StreamId(0),
            max: 64,
            buf: Vec::new(),
        }));
        assert!(matches!(r, IoResult::Read(b) if b == b"hello"));

        // Queue drained → EOF (empty read).
        let r = block_on(mock.perform(IoRequest::Read {
            id: StreamId(0),
            max: 64,
            buf: Vec::new(),
        }));
        assert!(matches!(r, IoResult::Read(b) if b.is_empty()));
    }

    #[test]
    fn mock_records_writes() {
        let mock = MockBackend::new();
        let r = block_on(mock.perform(IoRequest::Write {
            id: StreamId(0),
            bytes: b"abc".to_vec(),
        }));
        assert!(matches!(r, IoResult::Wrote(3)));
        assert_eq!(mock.writes(), vec![b"abc".to_vec()]);
    }

    #[test]
    fn smol_sleep_returns_slept() {
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::Sleep { ms: 5 }));
        assert!(matches!(r, IoResult::Slept));
    }

    #[test]
    fn smol_connect_write_read_echo() {
        use std::io::{Read, Write};

        // A blocking std echo server on its own thread: read 5 bytes, echo them.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 5];
            sock.read_exact(&mut buf).unwrap();
            sock.write_all(&buf).unwrap();
        });

        let backend = SmolBackend::new();
        block_on(async {
            let id = match backend
                .perform(IoRequest::Connect {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("connect failed: {other:?}"),
            };
            match backend
                .perform(IoRequest::Write {
                    id,
                    bytes: b"hello".to_vec(),
                })
                .await
            {
                IoResult::Wrote(5) => {}
                other => panic!("write failed: {other:?}"),
            }
            match backend
                .perform(IoRequest::Read {
                    id,
                    max: 64,
                    buf: Vec::new(),
                })
                .await
            {
                IoResult::Read(data) => assert_eq!(data, b"hello"),
                other => panic!("read failed: {other:?}"),
            }
            assert!(matches!(
                backend.perform(IoRequest::Close { id }).await,
                IoResult::Closed
            ));
        });

        server.join().unwrap();
    }

    #[test]
    fn smol_read_unknown_id_errors() {
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::Read {
            id: StreamId(999),
            max: 16,
            buf: Vec::new(),
        }));
        assert!(matches!(r, IoResult::Err(e) if e.kind == std::io::ErrorKind::NotFound));
    }

    /// End-to-end TLS over the loopback: a local rustls echo server with a self-signed
    /// cert, and a client that `Connect`s then `TlsWrap`s with `insecure: true` (the
    /// same escape hatch real users get) — so no test-only trust-anchor plumbing. Proves
    /// the handshake completes and bytes round-trip through the swapped-in TLS conduit.
    #[test]
    fn smol_tls_insecure_handshake_and_echo() {
        use futures_rustls::TlsAcceptor;
        use futures_rustls::rustls::ServerConfig;
        use futures_rustls::rustls::crypto::ring;
        use futures_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = cert.cert.der().clone();
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));

        let server_config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();

        // Bind on the main thread so the port is known synchronously, then hand the
        // listener to the server thread (it has its own reactor via `block_on`).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let acceptor = TlsAcceptor::from(Arc::new(server_config));
            block_on(async {
                let listener = async_io::Async::<std::net::TcpListener>::new(listener).unwrap();
                let (tcp, _) = listener.accept().await.unwrap();
                let mut tls = acceptor.accept(tcp).await.unwrap();
                let mut buf = [0u8; 5];
                tls.read_exact(&mut buf).await.unwrap();
                tls.write_all(&buf).await.unwrap();
                tls.flush().await.unwrap();
                // Hold the conduit open briefly so the client reads before the drop
                // closes the fd (simpler than a TLS half-close).
                Timer::after(Duration::from_millis(50)).await;
            });
        });

        let backend = SmolBackend::new();
        block_on(async {
            let id = match backend
                .perform(IoRequest::Connect {
                    host: "127.0.0.1".to_string(),
                    port: addr.port(),
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("connect failed: {other:?}"),
            };
            let id = match backend
                .perform(IoRequest::TlsWrap {
                    id,
                    domain: "localhost".to_string(),
                    insecure: true,
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("tls wrap failed: {other:?}"),
            };
            match backend
                .perform(IoRequest::Write {
                    id,
                    bytes: b"hello".to_vec(),
                })
                .await
            {
                IoResult::Wrote(5) => {}
                other => panic!("write failed: {other:?}"),
            }
            match backend
                .perform(IoRequest::Read {
                    id,
                    max: 64,
                    buf: Vec::new(),
                })
                .await
            {
                IoResult::Read(data) => assert_eq!(data, b"hello"),
                other => panic!("read failed: {other:?}"),
            }
        });

        server.join().unwrap();
    }

    /// The secure path (webpki roots) against a real host — the one thing the offline
    /// test can't cover, since a self-signed cert is rejected by the real verifier.
    /// Ignored by default (needs the network); run with `--ignored`.
    #[test]
    #[ignore = "hits the public internet (example.org:443); run with --ignored"]
    fn smol_tls_secure_real_host() {
        let backend = SmolBackend::new();
        block_on(async {
            let id = match backend
                .perform(IoRequest::Connect {
                    host: "example.org".to_string(),
                    port: 443,
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("connect failed: {other:?}"),
            };
            let id = match backend
                .perform(IoRequest::TlsWrap {
                    id,
                    domain: "example.org".to_string(),
                    insecure: false,
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("tls handshake failed: {other:?}"),
            };
            let req = b"GET / HTTP/1.0\r\nHost: example.org\r\nConnection: close\r\n\r\n".to_vec();
            match backend.perform(IoRequest::Write { id, bytes: req }).await {
                IoResult::Wrote(_) => {}
                other => panic!("write failed: {other:?}"),
            }
            match backend
                .perform(IoRequest::Read {
                    id,
                    max: 256,
                    buf: Vec::new(),
                })
                .await
            {
                IoResult::Read(data) => {
                    let head = String::from_utf8_lossy(&data);
                    assert!(head.starts_with("HTTP/1."), "unexpected response: {head:?}");
                }
                other => panic!("read failed: {other:?}"),
            }
        });
    }

    #[test]
    fn mock_fulfills_every_request_kind() {
        let mock = MockBackend::new();
        assert!(matches!(
            block_on(mock.perform(IoRequest::Sleep { ms: 0 })),
            IoResult::Slept
        ));
        assert!(matches!(
            block_on(mock.perform(IoRequest::Connect {
                host: "h".to_string(),
                port: 1,
            })),
            IoResult::Connected(_)
        ));
        assert!(matches!(
            block_on(mock.perform(IoRequest::Close { id: StreamId(1) })),
            IoResult::Closed
        ));
        assert!(matches!(
            block_on(mock.perform(IoRequest::OpenFile {
                path: OsString::from("f"),
            })),
            IoResult::Connected(_)
        ));
        assert!(matches!(
            block_on(mock.perform(IoRequest::Listen {
                host: "h".to_string(),
                port: 0,
            })),
            IoResult::Listening { port: 0, .. }
        ));
        assert!(matches!(
            block_on(mock.perform(IoRequest::Accept { id: StreamId(1) })),
            IoResult::Connected(_)
        ));
        // The mock keeps the id on a TlsWrap (no real handshake), mirroring the native
        // backend's in-place swap.
        assert!(matches!(
            block_on(mock.perform(IoRequest::TlsWrap {
                id: StreamId(7),
                domain: "h".to_string(),
                insecure: true,
            })),
            IoResult::Connected(StreamId(7))
        ));
    }

    #[test]
    fn mock_connect_and_open_mint_distinct_ids() {
        let mock = MockBackend::new();
        let a = block_on(mock.perform(IoRequest::Connect {
            host: "h".to_string(),
            port: 1,
        }));
        let b = block_on(mock.perform(IoRequest::OpenFile {
            path: OsString::from("f"),
        }));
        match (a, b) {
            (IoResult::Connected(x), IoResult::Connected(y)) => assert_ne!(x, y),
            other => panic!("expected two Connected ids, got {other:?}"),
        }
    }

    #[test]
    fn smol_backend_default_constructs() {
        // Exercise the `Default` impl (the rest of the suite builds via `new`).
        let _backend = SmolBackend::default();
    }

    #[test]
    fn secure_connector_loads_webpki_roots() {
        // The validating connector is otherwise reached only by the network-gated
        // real-host test; build it here so the webpki-roots load path is covered.
        let _connector = secure_connector();
    }

    #[test]
    fn smol_write_unknown_id_errors() {
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::Write {
            id: StreamId(999),
            bytes: b"x".to_vec(),
        }));
        assert!(matches!(r, IoResult::Err(e) if e.kind == std::io::ErrorKind::NotFound));
    }

    #[test]
    fn smol_close_unknown_id_is_noop() {
        let backend = SmolBackend::new();
        assert!(matches!(
            block_on(backend.perform(IoRequest::Close { id: StreamId(999) })),
            IoResult::Closed
        ));
    }

    #[test]
    fn smol_tlswrap_unknown_id_errors() {
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::TlsWrap {
            id: StreamId(999),
            domain: "localhost".to_string(),
            insecure: true,
        }));
        assert!(matches!(r, IoResult::Err(e) if e.kind == std::io::ErrorKind::NotFound));
    }

    #[test]
    fn smol_accept_unknown_listener_errors() {
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::Accept { id: StreamId(999) }));
        assert!(matches!(r, IoResult::Err(e) if e.kind == std::io::ErrorKind::NotFound));
    }

    #[test]
    fn smol_open_missing_file_errors() {
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::OpenFile {
            path: OsString::from("/no/such/quoin/file/anywhere.xyz"),
        }));
        assert!(matches!(r, IoResult::Err(e) if e.kind == std::io::ErrorKind::NotFound));
    }

    #[test]
    fn smol_listen_unbindable_addr_errors() {
        // 192.0.2.0/24 is TEST-NET-1 (RFC 5737): a literal IP (no DNS) that isn't a local
        // address, so the bind fails fast rather than hanging on name resolution.
        let backend = SmolBackend::new();
        let r = block_on(backend.perform(IoRequest::Listen {
            host: "192.0.2.1".to_string(),
            port: 0,
        }));
        assert!(matches!(r, IoResult::Err(_)));
    }

    /// `TlsWrap` with a server name rustls rejects: the stream is taken out and dropped,
    /// the id left vacant, and an `InvalidInput` error returned — before any handshake.
    #[test]
    fn smol_tlswrap_invalid_server_name_errors() {
        // A registered stream is required first; stand up a loopback listener and connect.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = std::thread::spawn(move || {
            let _ = listener.accept();
        });
        let backend = SmolBackend::new();
        block_on(async {
            let id = match backend
                .perform(IoRequest::Connect {
                    host: "127.0.0.1".to_string(),
                    port: addr.port(),
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("connect failed: {other:?}"),
            };
            // The empty string is not a valid TLS server name.
            let r = backend
                .perform(IoRequest::TlsWrap {
                    id,
                    domain: String::new(),
                    insecure: true,
                })
                .await;
            assert!(matches!(r, IoResult::Err(e) if e.kind == std::io::ErrorKind::InvalidInput));
        });
    }

    /// `insecure: false` routes through the secure (webpki) connector. The peer here
    /// speaks plaintext then drops, so the handshake fails — covering the secure-connector
    /// dispatch and the handshake-error arm without needing a real certificate.
    #[test]
    fn smol_tlswrap_secure_against_plaintext_peer_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                use std::io::Read;
                let mut buf = [0u8; 64];
                let _ = sock.read(&mut buf); // read the ClientHello, then drop -> EOF
            }
        });
        let backend = SmolBackend::new();
        block_on(async {
            let id = match backend
                .perform(IoRequest::Connect {
                    host: "127.0.0.1".to_string(),
                    port: addr.port(),
                })
                .await
            {
                IoResult::Connected(id) => id,
                other => panic!("connect failed: {other:?}"),
            };
            let r = backend
                .perform(IoRequest::TlsWrap {
                    id,
                    domain: "localhost".to_string(),
                    insecure: false,
                })
                .await;
            assert!(matches!(r, IoResult::Err(_)));
        });
    }
}
