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
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use async_io::Timer;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use futures_rustls::TlsConnector;
use futures_rustls::rustls::pki_types::ServerName;
use futures_rustls::rustls::{ClientConfig, RootCertStore};
use futures_util::future::{AbortHandle, Abortable};
use once_cell::unsync::OnceCell;

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
    /// future class. See `docs/ASYNC_ARCH.md`.
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
    /// (docs/CONCURRENCY_ARCH.md §4). The job's `Send + Sync` closure owns
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
}

/// The plain-data outcome of an [`IoRequest`].
#[derive(Clone, Debug)]
pub enum IoResult {
    Slept,
    Connected(StreamId),
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
// SmolBackend — the native implementation, on `async-io`.
// ---------------------------------------------------------------------------

/// A spawned child in the table. The `Child` sits behind a `RefCell` whose ONLY
/// mutable borrower is an in-flight `ChildWait` (guarded by `waiting`); everything
/// else goes through the pid (`kill`) or the cells. While the entry holds the
/// un-dropped `Child`, the OS cannot recycle the pid (an exited child is a zombie
/// until async-process's reaper — which runs on `Child` drop — collects it), so
/// signalling by pid is race-free.
struct ChildSlot {
    child: RefCell<async_process::Child>,
    pid: u32,
    /// `(code, signal)` once exited; recorded by the wait that observed it (or by
    /// a `child_running` probe).
    exited: Cell<Option<(Option<i32>, Option<i32>)>>,
    /// A `ChildWait` is in flight (its RAII guard clears this even on cancel).
    waiting: Cell<bool>,
    /// Detached: outlives the VM (the table's teardown kill skips it).
    detached: Cell<bool>,
}

impl Drop for SmolInner {
    fn drop(&mut self) {
        // Backend teardown (process exit): a still-running, undetached child dies
        // with the VM — the streaming twin of RunProcess's kill_on_drop. Detached
        // children are the one deliberate survivor.
        #[cfg(unix)]
        for slot in self.children.borrow().values() {
            if slot.exited.get().is_none() && !slot.detached.get() {
                unsafe {
                    libc::kill(slot.pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
}

/// Decompose an `ExitStatus` into the `(code, signal)` pair the results carry.
fn exit_parts(status: std::process::ExitStatus) -> (Option<i32>, Option<i32>) {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    (status.code(), signal)
}

/// Clears the slot's `waiting` flag when the wait op ends — including a CANCELLED
/// wait (the future is dropped mid-park), which would otherwise wedge the flag and
/// refuse every later wait on that child.
struct WaitGuard(Rc<ChildSlot>);
impl Drop for WaitGuard {
    fn drop(&mut self) {
        self.0.waiting.set(false);
    }
}

struct SmolInner {
    streams: RefCell<HashMap<StreamId, Box<dyn AsyncStream>>>,
    // Listening sockets live in their own registry: a `TcpListener` accepts connections
    // (it isn't an `AsyncStream`), so it can't share the `streams` map. Same id space.
    listeners: RefCell<HashMap<StreamId, async_net::TcpListener>>,
    // Spawned children, addressed by their own ids (same counter as streams — the
    // spaces never meet). See `ChildSlot`.
    children: RefCell<HashMap<u64, Rc<ChildSlot>>>,
    next_id: Cell<u64>,
    // TLS connectors are built lazily and cached: loading the webpki root bundle once
    // (rather than per connection) is the whole reason `TlsWrap` is cheap. `unsync`
    // because the VM + backend live on one thread (gc_arena is `!Send`).
    tls_secure: OnceCell<TlsConnector>,
    tls_insecure: OnceCell<TlsConnector>,
    // Ids whose stream/listener is currently leased out to an in-flight op. While an
    // id is leased it is absent from both registries, so `close` would otherwise be
    // a silent no-op: the parked op never woke, and the lease's drop re-inserted the
    // fd — an unkillable, unclosable connection.
    leased: RefCell<HashSet<StreamId>>,
    // Leased ids that were closed while their op was in flight. The lease's drop
    // consumes the tombstone and drops the resource (closing the fd) instead of
    // re-inserting it.
    closed_while_leased: RefCell<HashSet<StreamId>>,
    // Abort handles for in-flight leased ops, so `close` interrupts a parked
    // read/write/accept promptly (the op resolves to a catchable "closed" error)
    // instead of leaving the task waiting on a handle that no longer exists.
    op_aborts: RefCell<HashMap<StreamId, AbortHandle>>,
}

impl SmolInner {
    fn insert(&self, stream: Box<dyn AsyncStream>) -> StreamId {
        let id = StreamId(self.next_id.get());
        self.next_id.set(self.next_id.get() + 1);
        self.streams.borrow_mut().insert(id, stream);
        id
    }

    /// Reinsert a (now-TLS) stream at an id that was just vacated by `take_stream` —
    /// `TlsWrap` swaps the conduit in place without minting a new id.
    fn insert_at(&self, id: StreamId, stream: Box<dyn AsyncStream>) -> StreamId {
        self.streams.borrow_mut().insert(id, stream);
        id
    }

    fn insert_listener(&self, listener: async_net::TcpListener) -> StreamId {
        let id = StreamId(self.next_id.get());
        self.next_id.set(self.next_id.get() + 1);
        self.listeners.borrow_mut().insert(id, listener);
        id
    }

    fn connector(&self, insecure: bool) -> TlsConnector {
        if insecure {
            self.tls_insecure.get_or_init(insecure_connector).clone()
        } else {
            self.tls_secure.get_or_init(secure_connector).clone()
        }
    }
}

/// A validating client connector trusting the Mozilla webpki root bundle. The ring
/// crypto provider is pinned explicitly (via `builder_with_provider`) so we never
/// depend on a process-default provider being installed — with aws-lc-rs disabled
/// there is none, and the default-provider builder would panic.
fn secure_connector() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder_with_provider(Arc::new(
        futures_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports the default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

/// A connector that accepts any server certificate. For local debugging only — the
/// `insecure:` flag at the QN surface is the deterrent. The handshake signature is
/// still checked (only the certificate *chain/identity* is skipped), which is the
/// conventional shape of rustls' "dangerous: accept any cert" verifier.
fn insecure_connector() -> TlsConnector {
    let config = ClientConfig::builder_with_provider(Arc::new(
        futures_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports the default protocol versions")
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(danger::NoCertVerification::new()))
    .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

/// The "accept any certificate" verifier behind `insecure:`. Skips chain/name
/// validation but still verifies handshake signatures against the ring provider's
/// algorithms — the standard rustls danger-verifier pattern.
mod danger {
    use futures_rustls::rustls::client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    };
    use futures_rustls::rustls::crypto::{
        WebPkiSupportedAlgorithms, ring, verify_tls12_signature, verify_tls13_signature,
    };
    use futures_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use futures_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub struct NoCertVerification(WebPkiSupportedAlgorithms);

    impl NoCertVerification {
        pub fn new() -> Self {
            NoCertVerification(ring::default_provider().signature_verification_algorithms)
        }
    }

    impl ServerCertVerifier for NoCertVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            verify_tls12_signature(message, cert, dss, &self.0)
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            verify_tls13_signature(message, cert, dss, &self.0)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.supported_schemes()
        }
    }
}

/// Native backend backed by `async-io`'s reactor. Owns the stream registry; cloned
/// `Rc<SmolInner>` handles are captured by each returned future, so resources live
/// entirely outside the GC arena.
#[derive(Clone)]
pub struct SmolBackend {
    inner: Rc<SmolInner>,
}

impl Default for SmolBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SmolBackend {
    pub fn new() -> Self {
        SmolBackend {
            inner: Rc::new(SmolInner {
                streams: RefCell::new(HashMap::new()),
                listeners: RefCell::new(HashMap::new()),
                children: RefCell::new(HashMap::new()),
                next_id: Cell::new(1),
                tls_secure: OnceCell::new(),
                tls_insecure: OnceCell::new(),
                leased: RefCell::new(HashSet::new()),
                closed_while_leased: RefCell::new(HashSet::new()),
                op_aborts: RefCell::new(HashMap::new()),
            }),
        }
    }
}

/// Look up and remove a stream from the registry so the op can own it by value for
/// the duration of the await (no `RefCell` borrow is held across `.await`). A single
/// stream is only ever used by one fiber, so removing it for the op is safe — and it
/// structurally enforces "no concurrent ops on the same stream". Only `TlsWrap` uses
/// this raw form (the handshake genuinely consumes the stream, even on failure);
/// abortable byte ops must lease via [`StreamLease`] instead, or a cancelled task
/// would close the fd it was merely waiting on.
fn take_stream(inner: &SmolInner, id: StreamId) -> Result<Box<dyn AsyncStream>, IoError> {
    inner
        .streams
        .borrow_mut()
        .remove(&id)
        .ok_or_else(|| IoError {
            kind: std::io::ErrorKind::NotFound,
            message: format!("unknown stream id {}", id.0),
        })
}

/// Own a stream for the duration of one op, returning it to the registry on drop —
/// crucially also when the op's future is **aborted** mid-await (`Async.timeout:` /
/// `cancel` on a task parked in a read or write). Cancellation must stop the wait,
/// not destroy the stream: before this guard, the dropped future took the socket
/// with it, so the peer saw an EOF and every later op on the id was "unknown stream".
struct StreamLease {
    inner: Rc<SmolInner>,
    id: StreamId,
    stream: Option<Box<dyn AsyncStream>>,
}

impl StreamLease {
    fn take(inner: &Rc<SmolInner>, id: StreamId) -> Result<Self, IoError> {
        let stream = take_stream(inner, id)?;
        inner.leased.borrow_mut().insert(id);
        Ok(Self {
            inner: inner.clone(),
            id,
            stream: Some(stream),
        })
    }

    fn stream(&mut self) -> &mut Box<dyn AsyncStream> {
        self.stream.as_mut().expect("stream is leased until drop")
    }
}

impl Drop for StreamLease {
    fn drop(&mut self) {
        self.inner.leased.borrow_mut().remove(&self.id);
        self.inner.op_aborts.borrow_mut().remove(&self.id);
        if let Some(s) = self.stream.take() {
            // A close that arrived while the op was in flight left a tombstone:
            // honor it by dropping the stream (closing the fd) instead of
            // resurrecting a handle the program already closed.
            if self.inner.closed_while_leased.borrow_mut().remove(&self.id) {
                drop(s);
            } else {
                self.inner.streams.borrow_mut().insert(self.id, s);
            }
        }
    }
}

/// The listener analogue of [`StreamLease`]: a cancelled `accept` (server `stop`, or
/// a timeout around it) must not tear down the listening socket.
struct ListenerLease {
    inner: Rc<SmolInner>,
    id: StreamId,
    listener: Option<async_net::TcpListener>,
}

impl ListenerLease {
    fn take(inner: &Rc<SmolInner>, id: StreamId) -> Result<Self, IoError> {
        let listener = inner
            .listeners
            .borrow_mut()
            .remove(&id)
            .ok_or_else(|| IoError {
                kind: std::io::ErrorKind::NotFound,
                message: format!("unknown listener id {}", id.0),
            })?;
        inner.leased.borrow_mut().insert(id);
        Ok(Self {
            inner: inner.clone(),
            id,
            listener: Some(listener),
        })
    }

    fn listener(&mut self) -> &async_net::TcpListener {
        self.listener
            .as_ref()
            .expect("listener is leased until drop")
    }
}

impl Drop for ListenerLease {
    fn drop(&mut self) {
        self.inner.leased.borrow_mut().remove(&self.id);
        self.inner.op_aborts.borrow_mut().remove(&self.id);
        if let Some(l) = self.listener.take() {
            if self.inner.closed_while_leased.borrow_mut().remove(&self.id) {
                drop(l); // closed mid-accept: release the port, don't resurrect it
            } else {
                self.inner.listeners.borrow_mut().insert(self.id, l);
            }
        }
    }
}

impl IoBackend for SmolBackend {
    fn perform(&self, req: IoRequest) -> IoFuture {
        let inner = self.inner.clone();
        match req {
            IoRequest::Sleep { ms } => Box::pin(async move {
                Timer::after(Duration::from_millis(ms)).await;
                IoResult::Slept
            }),

            IoRequest::Connect { host, port } => Box::pin(async move {
                // `async-net` resolves `host:port` (getaddrinfo on the blocking pool)
                // and connects; the stream drops into the same `AsyncStream` registry.
                match async_net::TcpStream::connect((host.as_str(), port)).await {
                    Ok(stream) => IoResult::Connected(inner.insert(Box::new(stream))),
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::ConnectUnix { path } => Box::pin(async move {
                match async_net::unix::UnixStream::connect(&path).await {
                    Ok(stream) => IoResult::Connected(inner.insert(Box::new(stream))),
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::Read { id, max, buf } => Box::pin(async move {
                // Leased, not taken: aborting a parked read (task cancel / timeout)
                // must return the stream to the registry, not drop the fd. The op is
                // additionally abortable by `close` on the handle (see `op_aborts`):
                // the parked task then wakes with a catchable "closed" error and the
                // lease's tombstone check closes the fd.
                let lease = match StreamLease::take(&inner, id) {
                    Ok(l) => l,
                    Err(e) => return IoResult::Err(e),
                };
                let (abort, reg) = AbortHandle::new_pair();
                inner.op_aborts.borrow_mut().insert(id, abort);
                let res = Abortable::new(
                    async move {
                        let mut lease = lease;
                        let mut buf = buf;
                        buf.resize(max, 0);
                        let r = lease.stream().read(&mut buf).await;
                        drop(lease);
                        (r, buf)
                    },
                    reg,
                )
                .await;
                match res {
                    Ok((Ok(n), mut buf)) => {
                        buf.truncate(n);
                        IoResult::Read(buf)
                    }
                    Ok((Err(e), _)) => IoResult::Err(e.into()),
                    Err(_aborted) => IoResult::Err(IoError {
                        kind: std::io::ErrorKind::NotConnected,
                        message: "stream closed while a read was in flight".to_string(),
                    }),
                }
            }),

            IoRequest::ReadTimed { id, max, ms, buf } => Box::pin(async move {
                // Leased like Read, plus a wall-clock deadline: whichever of the read or
                // the timer resolves first wins. On timeout the lease drops (returning
                // the stream to the registry — the caller decides whether to close it).
                let lease = match StreamLease::take(&inner, id) {
                    Ok(l) => l,
                    Err(e) => return IoResult::Err(e),
                };
                let (abort, reg) = AbortHandle::new_pair();
                inner.op_aborts.borrow_mut().insert(id, abort);
                let read = Abortable::new(
                    async move {
                        let mut lease = lease;
                        let mut buf = buf;
                        buf.resize(max, 0);
                        let r = lease.stream().read(&mut buf).await;
                        drop(lease);
                        (r, buf)
                    },
                    reg,
                );
                let timeout = async {
                    Timer::after(Duration::from_millis(ms)).await;
                };
                match futures_lite::future::or(async { Some(read.await) }, async {
                    timeout.await;
                    None
                })
                .await
                {
                    Some(Ok((Ok(n), mut buf))) => {
                        buf.truncate(n);
                        IoResult::Read(buf)
                    }
                    Some(Ok((Err(e), _))) => IoResult::Err(e.into()),
                    Some(Err(_aborted)) => IoResult::Err(IoError {
                        kind: std::io::ErrorKind::NotConnected,
                        message: "stream closed while a read was in flight".to_string(),
                    }),
                    None => IoResult::Err(IoError {
                        kind: std::io::ErrorKind::TimedOut,
                        message: format!("read timed out after {ms}ms"),
                    }),
                }
            }),

            IoRequest::Compute(job) => {
                Box::pin(async move { IoResult::Computed(crate::compute::offload(job).await) })
            }
            IoRequest::WorkerRecv(rx) => {
                Box::pin(async move { IoResult::WorkerMsg(rx.recv().await.ok()) })
            }
            IoRequest::WorkerRecvTimed { rx, ms } => Box::pin(async move {
                let recv = async { rx.recv().await.ok() };
                let deadline = async {
                    async_io::Timer::after(std::time::Duration::from_millis(ms)).await;
                    None
                };
                IoResult::WorkerMsg(futures_lite::future::or(recv, deadline).await)
            }),
            IoRequest::WorkerJoin(rx) => Box::pin(async move {
                IoResult::WorkerDone(rx.recv().await.unwrap_or_else(|_| {
                    Err("worker vanished without reporting a result".to_string())
                }))
            }),
            IoRequest::Write { id, bytes } => Box::pin(async move {
                // Leased like Read. An aborted write may leave the peer with a
                // partial message — the canceller's problem — but the stream itself
                // stays usable (and properly closeable).
                let lease = match StreamLease::take(&inner, id) {
                    Ok(l) => l,
                    Err(e) => return IoResult::Err(e),
                };
                let (abort, reg) = AbortHandle::new_pair();
                inner.op_aborts.borrow_mut().insert(id, abort);
                let res = Abortable::new(
                    async move {
                        let mut lease = lease;
                        let r = async {
                            lease.stream().write_all(&bytes).await?;
                            lease.stream().flush().await?;
                            Ok::<usize, std::io::Error>(bytes.len())
                        }
                        .await;
                        drop(lease);
                        r
                    },
                    reg,
                )
                .await;
                match res {
                    Ok(Ok(n)) => IoResult::Wrote(n),
                    Ok(Err(e)) => IoResult::Err(e.into()),
                    Err(_aborted) => IoResult::Err(IoError {
                        kind: std::io::ErrorKind::NotConnected,
                        message: "stream closed while a write was in flight".to_string(),
                    }),
                }
            }),

            IoRequest::Close { id } => Box::pin(async move {
                // Drop the stream/listener (closing the fd) without holding the borrow
                // across any await; missing ids are a no-op so double-close is harmless.
                let _ = inner.streams.borrow_mut().remove(&id);
                let _ = inner.listeners.borrow_mut().remove(&id);
                IoResult::Closed
            }),

            IoRequest::Listen { host, port } => Box::pin(async move {
                match async_net::TcpListener::bind((host.as_str(), port)).await {
                    Ok(listener) => match listener.local_addr() {
                        Ok(addr) => IoResult::Listening {
                            id: inner.insert_listener(listener),
                            port: addr.port(),
                        },
                        Err(e) => IoResult::Err(e.into()),
                    },
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::Accept { id } => Box::pin(async move {
                // Leased for the accept (no map borrow held across the await — one
                // accept in flight per listener, like the byte ops); the lease also
                // survives an aborted accept (server `stop` cancels a parked one),
                // which must not tear the listening socket down. The accepted stream
                // drops into the shared `AsyncStream` registry.
                let lease = match ListenerLease::take(&inner, id) {
                    Ok(l) => l,
                    Err(e) => return IoResult::Err(e),
                };
                let (abort, reg) = AbortHandle::new_pair();
                inner.op_aborts.borrow_mut().insert(id, abort);
                let res = Abortable::new(
                    async move {
                        let mut lease = lease;
                        let r = lease.listener().accept().await;
                        drop(lease);
                        r
                    },
                    reg,
                )
                .await;
                match res {
                    Ok(Ok((stream, _peer))) => IoResult::Connected(inner.insert(Box::new(stream))),
                    Ok(Err(e)) => IoResult::Err(e.into()),
                    Err(_aborted) => IoResult::Err(IoError {
                        kind: std::io::ErrorKind::NotConnected,
                        message: "listener closed while an accept was in flight".to_string(),
                    }),
                }
            }),

            IoRequest::OpenFile { path } => Box::pin(async move {
                // `async-fs` opens on the blocking pool (regular files aren't pollable),
                // same trick as `async-net`'s DNS. The `File` is `AsyncRead + AsyncWrite +
                // Unpin`, so it drops into the same registry as any socket.
                match async_fs::File::open(&path).await {
                    Ok(file) => IoResult::Connected(inner.insert(Box::new(file))),
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::RunProcess {
                program,
                args,
                env,
                dir,
                input,
            } => Box::pin(async move {
                let mut cmd = async_process::Command::new(&program);
                cmd.args(&args);
                if let Some(env) = env {
                    for (k, v) in env {
                        cmd.env(k, v);
                    }
                }
                if let Some(dir) = dir {
                    cmd.current_dir(dir);
                }
                cmd.stdin(if input.is_some() {
                    std::process::Stdio::piped()
                } else {
                    std::process::Stdio::null()
                });
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::piped());
                // Cancellation semantics: this op owns the whole child lifecycle, so
                // dropping the future mid-flight (Async.timeout: fired, task
                // cancelled) must kill the child, not leak it.
                cmd.kill_on_drop(true);
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => return IoResult::Err(e.into()),
                };
                if let (Some(mut sin), Some(bytes)) = (child.stdin.take(), input) {
                    use futures_lite::AsyncWriteExt;
                    // A child that exits without draining its input (`head -1`)
                    // breaks the pipe mid-write; that is normal, not an error —
                    // the exit status tells the real story either way.
                    let _ = sin.write_all(&bytes).await;
                    // `sin` drops here → the child sees EOF.
                }
                // `output()` reads stdout and stderr CONCURRENTLY while awaiting
                // exit — reading either alone deadlocks when the other pipe's
                // buffer fills.
                match child.output().await {
                    Ok(out) => {
                        let (code, signal) = exit_parts(out.status);
                        IoResult::ProcDone {
                            code,
                            signal,
                            stdout: out.stdout,
                            stderr: out.stderr,
                        }
                    }
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::SpawnProcess {
                program,
                args,
                env,
                dir,
            } => Box::pin(async move {
                let mut cmd = async_process::Command::new(&program);
                cmd.args(&args);
                if let Some(env) = env {
                    for (k, v) in env {
                        cmd.env(k, v);
                    }
                }
                if let Some(dir) = dir {
                    cmd.current_dir(dir);
                }
                cmd.stdin(std::process::Stdio::piped());
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::piped());
                // NOT kill_on_drop: the handle's reap queue owns the kill (a
                // detached child must survive its Child being dropped).
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => return IoResult::Err(e.into()),
                };
                let pid = child.id();
                let stdin = inner.insert(Box::new(WriteOnlyStream(
                    child.stdin.take().expect("stdin was piped"),
                )));
                let stdout = inner.insert(Box::new(ReadOnlyStream(
                    child.stdout.take().expect("stdout was piped"),
                )));
                let stderr = inner.insert(Box::new(ReadOnlyStream(
                    child.stderr.take().expect("stderr was piped"),
                )));
                let id = inner.next_id.get();
                inner.next_id.set(id + 1);
                inner.children.borrow_mut().insert(
                    id,
                    Rc::new(ChildSlot {
                        child: RefCell::new(child),
                        pid,
                        exited: Cell::new(None),
                        waiting: Cell::new(false),
                        detached: Cell::new(false),
                    }),
                );
                IoResult::ProcSpawned {
                    child: id,
                    pid,
                    stdin,
                    stdout,
                    stderr,
                }
            }),

            IoRequest::ChildWait { id } => Box::pin(async move {
                let slot = match inner.children.borrow().get(&id) {
                    Some(s) => Rc::clone(s),
                    None => {
                        return IoResult::Err(IoError {
                            kind: std::io::ErrorKind::NotFound,
                            message: "process handle is closed".to_string(),
                        });
                    }
                };
                if let Some((code, signal)) = slot.exited.get() {
                    return IoResult::ProcExited { code, signal };
                }
                if slot.waiting.replace(true) {
                    return IoResult::Err(IoError {
                        kind: std::io::ErrorKind::WouldBlock,
                        message: "another task is already waiting on this process".to_string(),
                    });
                }
                let _guard = WaitGuard(Rc::clone(&slot));
                // The wait holds the child's only mutable borrow across the await —
                // everything concurrent (kill, running?, reap) goes through the pid
                // or the cells, never this RefCell (see ChildSlot).
                let status = {
                    let mut child = slot.child.borrow_mut();
                    child.status().await
                };
                match status {
                    Ok(status) => {
                        let (code, signal) = exit_parts(status);
                        slot.exited.set(Some((code, signal)));
                        IoResult::ProcExited { code, signal }
                    }
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::OpenFileWrite { path, append } => Box::pin(async move {
                // Same registry, same `Write` op as a socket. `async_fs::File` buffers
                // internally and is flushed by the `Write` arm above, so `Close` dropping the
                // handle cannot strand bytes that reached the backend.
                let mut opts = async_fs::OpenOptions::new();
                opts.write(true).create(true);
                if append {
                    opts.append(true);
                } else {
                    opts.truncate(true);
                }
                match opts.open(&path).await {
                    Ok(file) => IoResult::Connected(inner.insert(Box::new(file))),
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::OpenStdin => Box::pin(async move {
                // `Unblock` reads on the blocking pool, spawning nothing until the first read.
                // Dropping the inner `Stdin` does NOT close fd 0 — it is a handle onto a process
                // -wide static — so reaping this stream leaves the descriptor intact for anyone
                // else (the REPL's line editor, a later stream).
                let stdin = blocking::Unblock::new(std::io::stdin());
                IoResult::Connected(inner.insert(Box::new(ReadOnlyStream(stdin))))
            }),

            IoRequest::TlsWrap {
                id,
                domain,
                insecure,
            } => Box::pin(async move {
                // Take the plaintext stream out by value for the handshake (same as the
                // byte ops — no registry borrow is held across the await). The server
                // name drives SNI and certificate verification; `.to_owned()` lifts it
                // to `'static` for the connector.
                let stream = match take_stream(&inner, id) {
                    Ok(s) => s,
                    Err(e) => return IoResult::Err(e),
                };
                let server_name = match ServerName::try_from(domain.as_str()) {
                    Ok(name) => name.to_owned(),
                    Err(_) => {
                        // `stream` drops here → fd closed; the id stays vacant.
                        return IoResult::Err(IoError {
                            kind: std::io::ErrorKind::InvalidInput,
                            message: format!("invalid TLS server name '{domain}'"),
                        });
                    }
                };
                match inner.connector(insecure).connect(server_name, stream).await {
                    // Swap the conduit in place: the TLS stream takes over the same id.
                    Ok(tls) => IoResult::Connected(inner.insert_at(id, Box::new(tls))),
                    // Handshake failed: the underlying stream was consumed (fd closed).
                    Err(e) => IoResult::Err(e.into()),
                }
            }),
        }
    }

    fn close(&self, id: StreamId) {
        // Streams and listeners share one id space but separate registries — a
        // listener id left here kept the port bound (and the backlog accepting)
        // forever after `TcpListener.close`, since the reap path is the only close
        // most code reaches (the async `IoRequest::Close` is test-only).
        let _ = self.inner.streams.borrow_mut().remove(&id);
        let _ = self.inner.listeners.borrow_mut().remove(&id);
        // If the resource is leased out to an in-flight op, it is in neither map:
        // tombstone it (the lease's drop will close the fd instead of re-inserting)
        // and abort the op so the parked task wakes with a "closed" error now,
        // rather than hanging until the peer happens to act.
        if self.inner.leased.borrow().contains(&id) {
            self.inner.closed_while_leased.borrow_mut().insert(id);
            if let Some(h) = self.inner.op_aborts.borrow_mut().remove(&id) {
                h.abort();
            }
        }
    }

    fn reap_child(&self, id: u64) {
        // Kill (if still running), then drop the table entry. A parked wait holds
        // its own `Rc` — the kill resolves it with the signal exit; the `Child`
        // itself drops when the last Rc goes, and async-process's global reaper
        // collects the zombie.
        let slot = self.inner.children.borrow_mut().remove(&id);
        if let Some(slot) = slot {
            if slot.exited.get().is_none() && !slot.detached.get() {
                #[cfg(unix)]
                unsafe {
                    libc::kill(slot.pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }

    fn child_signal(&self, id: u64, signal: i32) -> Result<(), IoError> {
        let slot = match self.inner.children.borrow().get(&id) {
            Some(s) => Rc::clone(s),
            None => {
                return Err(IoError {
                    kind: std::io::ErrorKind::NotFound,
                    message: "process handle is closed".to_string(),
                });
            }
        };
        // Exited (recorded) → no-op: the pid may be recycled once the zombie is
        // reaped, and signalling a stranger is the one unforgivable outcome.
        if slot.exited.get().is_some() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            let rc = unsafe { libc::kill(slot.pid as libc::pid_t, signal) };
            if rc == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error().into())
            }
        }
        #[cfg(not(unix))]
        {
            let _ = signal;
            Err(IoError {
                kind: std::io::ErrorKind::Unsupported,
                message: "process signals are unix-only".to_string(),
            })
        }
    }

    fn child_running(&self, id: u64) -> bool {
        let slot = match self.inner.children.borrow().get(&id) {
            Some(s) => Rc::clone(s),
            None => return false,
        };
        if slot.exited.get().is_some() {
            return false;
        }
        // A parked wait holds the child's borrow — and also proves the child has
        // not exited yet (the wait would have resolved and recorded it).
        if slot.waiting.get() {
            return true;
        }
        let mut child = slot.child.borrow_mut();
        match child.try_status() {
            Ok(Some(status)) => {
                let (code, signal) = exit_parts(status);
                slot.exited.set(Some((code, signal)));
                false
            }
            Ok(None) => true,
            Err(_) => false,
        }
    }

    fn child_detach(&self, id: u64) {
        if let Some(slot) = self.inner.children.borrow().get(&id) {
            slot.detached.set(true);
        }
    }
}

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
            IoRequest::Read { max, .. } | IoRequest::ReadTimed { max, .. } => {
                let mut buf = self.reads.borrow_mut().pop_front().unwrap_or_default();
                buf.truncate(max);
                IoResult::Read(buf)
            }
            IoRequest::RunProcess { .. }
            | IoRequest::SpawnProcess { .. }
            | IoRequest::ChildWait { .. } => IoResult::Err(IoError {
                kind: std::io::ErrorKind::Unsupported,
                message: "the mock backend has no subprocess support".to_string(),
            }),
            IoRequest::Write { bytes, .. } => {
                let n = bytes.len();
                self.writes.borrow_mut().push(bytes);
                IoResult::Wrote(n)
            }
            IoRequest::Close { .. } => IoResult::Closed,
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
    use futures_lite::future::block_on;

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
