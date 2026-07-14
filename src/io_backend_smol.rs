//! The smol-backed native [`IoBackend`] (`SmolBackend`): every real-OS fulfillment of
//! an [`IoRequest`] — sockets, TLS, files, subprocesses, DNS, stdin. A `#[path]` child
//! of `io_backend.rs`, compiled out on wasm32 (no pollable fds there); the seam types
//! and `MockBackend` stay in the parent, which every runtime class keeps importing.

use super::*;

use async_io::Timer;
use futures_rustls::TlsConnector;
use futures_rustls::rustls::pki_types::ServerName;
use futures_rustls::rustls::{ClientConfig, RootCertStore};
use futures_util::future::{AbortHandle, Abortable};
use once_cell::unsync::OnceCell;

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
        // (Random-access files in `seekables` just drop with the struct — a
        // read-only fd has nothing to finish.)
        // Finish (don't just drop) any write-codec stream still open: its encoder
        // writes the trailer in `poll_close`. This is the once-per-session end —
        // the driver's per-run exit flush already wrote the buffered bytes, and a
        // REPL keeps such a stream writable across lines, so only true teardown
        // may finish it. Blocking is fine here: the scheduler is gone, and the
        // close bottoms out in local file I/O on the blocking pool. Best-effort,
        // like the exit flush (there is no one left to raise to).
        let pending: Vec<StreamId> = self.finish_pending.borrow_mut().drain().collect();
        for id in pending {
            let stream = self.streams.borrow_mut().remove(&id);
            if let Some(mut stream) = stream {
                use futures_lite::AsyncWriteExt;
                if let Err(e) = futures_lite::future::block_on(stream.close()) {
                    eprintln!("qn: could not finish a compressed stream on exit: {e:?}");
                }
            }
        }
    }
}

/// getnameinfo with NI_NAMEREQD, run on the blocking pool: the PTR name for `ip`,
/// or `None` when the address has no mapping — that is an ordinary answer, not an
/// error. Rust's std exposes no reverse lookup, so this is a direct libc call
/// (the process code's `kill` precedent).
#[cfg(unix)]
fn reverse_lookup(ip: std::net::IpAddr) -> Option<String> {
    use std::ffi::CStr;
    use std::net::IpAddr;
    let mut host = [0u8; 1025]; // NI_MAXHOST
    let rc = unsafe {
        match ip {
            IpAddr::V4(v4) => {
                let mut sa: libc::sockaddr_in = std::mem::zeroed();
                #[cfg(any(
                    target_os = "macos",
                    target_os = "ios",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                    target_os = "dragonfly"
                ))]
                {
                    sa.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
                }
                sa.sin_family = libc::AF_INET as libc::sa_family_t;
                sa.sin_addr.s_addr = u32::from_ne_bytes(v4.octets());
                libc::getnameinfo(
                    &sa as *const libc::sockaddr_in as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                    host.as_mut_ptr() as *mut libc::c_char,
                    host.len() as libc::socklen_t,
                    std::ptr::null_mut(),
                    0,
                    libc::NI_NAMEREQD,
                )
            }
            IpAddr::V6(v6) => {
                let mut sa: libc::sockaddr_in6 = std::mem::zeroed();
                #[cfg(any(
                    target_os = "macos",
                    target_os = "ios",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                    target_os = "dragonfly"
                ))]
                {
                    sa.sin6_len = std::mem::size_of::<libc::sockaddr_in6>() as u8;
                }
                sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sa.sin6_addr.s6_addr = v6.octets();
                libc::getnameinfo(
                    &sa as *const libc::sockaddr_in6 as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                    host.as_mut_ptr() as *mut libc::c_char,
                    host.len() as libc::socklen_t,
                    std::ptr::null_mut(),
                    0,
                    libc::NI_NAMEREQD,
                )
            }
        }
    };
    if rc != 0 {
        return None;
    }
    let name = unsafe { CStr::from_ptr(host.as_ptr() as *const libc::c_char) };
    name.to_str().ok().map(|s| s.to_string())
}

/// No getnameinfo off unix: reverse lookups answer "no mapping".
#[cfg(not(unix))]
fn reverse_lookup(_ip: std::net::IpAddr) -> Option<String> {
    None
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
    // Random-access files, likewise their own registry: `ReadAt` needs the concrete
    // `async_fs::File` (the `dyn AsyncStream` box erases `AsyncSeek`). Same id space.
    seekables: RefCell<HashMap<StreamId, async_fs::File>>,
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
    // Streams wrapped in a write-side codec (recorded by `WrapStream`), whose
    // close must drive `poll_close` (the encoder's finish) rather than drop the
    // fd. `FinishStream` and the sync `close` both clear an id; whatever is
    // still here at teardown gets finished by `Drop` — the backstop that makes
    // "wrote a .gz and fell off the end of the program" produce a valid file.
    finish_pending: RefCell<HashSet<StreamId>>,
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

    fn insert_seekable(&self, file: async_fs::File) -> StreamId {
        let id = StreamId(self.next_id.get());
        self.next_id.set(self.next_id.get() + 1);
        self.seekables.borrow_mut().insert(id, file);
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
pub(super) fn secure_connector() -> TlsConnector {
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
                seekables: RefCell::new(HashMap::new()),
                children: RefCell::new(HashMap::new()),
                next_id: Cell::new(1),
                tls_secure: OnceCell::new(),
                tls_insecure: OnceCell::new(),
                leased: RefCell::new(HashSet::new()),
                closed_while_leased: RefCell::new(HashSet::new()),
                finish_pending: RefCell::new(HashSet::new()),
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

/// The random-access-file analogue of [`StreamLease`]: a cancelled `ReadAt`
/// (a timeout around it) must not close the file out from under the handle.
struct SeekableLease {
    inner: Rc<SmolInner>,
    id: StreamId,
    file: Option<async_fs::File>,
}

impl SeekableLease {
    fn take(inner: &Rc<SmolInner>, id: StreamId) -> Result<Self, IoError> {
        let file = inner
            .seekables
            .borrow_mut()
            .remove(&id)
            .ok_or_else(|| IoError {
                kind: std::io::ErrorKind::NotFound,
                message: format!("unknown random-access file id {}", id.0),
            })?;
        inner.leased.borrow_mut().insert(id);
        Ok(Self {
            inner: inner.clone(),
            id,
            file: Some(file),
        })
    }

    fn file(&mut self) -> &mut async_fs::File {
        self.file.as_mut().expect("file is leased until drop")
    }
}

impl Drop for SeekableLease {
    fn drop(&mut self) {
        self.inner.leased.borrow_mut().remove(&self.id);
        self.inner.op_aborts.borrow_mut().remove(&self.id);
        if let Some(f) = self.file.take() {
            if self.inner.closed_while_leased.borrow_mut().remove(&self.id) {
                drop(f); // closed mid-read: honor it, don't resurrect the fd
            } else {
                self.inner.seekables.borrow_mut().insert(self.id, f);
            }
        }
    }
}

impl IoBackend for SmolBackend {
    // The `ProcWait` arm deliberately holds the child's sole RefCell borrow across the
    // `.await` (see the ChildSlot note at that borrow); no other arm holds one across an
    // await, so allow the lint for the whole dispatch fn rather than fight the borrow.
    #[allow(clippy::await_holding_refcell_ref)]
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
            IoRequest::DispatchRecv(rx) => {
                Box::pin(async move { IoResult::DispatchMsg(rx.recv().await.ok().map(Box::new)) })
            }
            IoRequest::FrameRecv(rx) => {
                Box::pin(async move { IoResult::FrameMsg(rx.recv().await.ok().map(Box::new)) })
            }
            IoRequest::ChanRecv(rx) => {
                Box::pin(async move { IoResult::ChanFrame(rx.recv().await.ok().map(Box::new)) })
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
                // Drop the stream/listener/file (closing the fd) without holding the
                // borrow across any await; missing ids are a no-op so double-close is
                // harmless.
                let _ = inner.streams.borrow_mut().remove(&id);
                let _ = inner.listeners.borrow_mut().remove(&id);
                let _ = inner.seekables.borrow_mut().remove(&id);
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

            IoRequest::WrapStream { id, codec } => Box::pin(async move {
                // Resolve the codec BEFORE taking the stream, so an unknown name
                // errs with the stream (and its fd) untouched in the registry.
                let (side, wrap) = match crate::io_codecs::lookup(&codec) {
                    Ok(f) => f,
                    Err(e) => return IoResult::Err(e),
                };
                let stream = match take_stream(&inner, id) {
                    Ok(s) => s,
                    Err(e) => return IoResult::Err(e),
                };
                if side == crate::io_codecs::Side::Write {
                    // From here on this id must be FINISHED, never dropped — the
                    // encoder's trailer is written by its `poll_close`.
                    inner.finish_pending.borrow_mut().insert(id);
                }
                IoResult::Connected(inner.insert_at(id, wrap(stream)))
            }),

            IoRequest::FinishStream { id } => Box::pin(async move {
                // A complete close: drive the `poll_close` chain (encoder finish →
                // trailer → inner close), then drop. The id leaves `finish_pending`
                // either way — on error the stream is dropped with the fd closed,
                // and the failure is reported exactly once, here.
                inner.finish_pending.borrow_mut().remove(&id);
                let mut stream = match take_stream(&inner, id) {
                    Ok(s) => s,
                    Err(e) => return IoResult::Err(e),
                };
                use futures_lite::AsyncWriteExt;
                match stream.close().await {
                    Ok(()) => IoResult::Closed,
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::OpenFileRandom { path } => Box::pin(async move {
                // Open + stat in one op, so `size` is the truth at open time.
                let file = match async_fs::File::open(&path).await {
                    Ok(f) => f,
                    Err(e) => return IoResult::Err(e.into()),
                };
                let size = match file.metadata().await {
                    Ok(m) => m.len(),
                    Err(e) => return IoResult::Err(e.into()),
                };
                IoResult::Opened {
                    id: inner.insert_seekable(file),
                    size,
                }
            }),

            IoRequest::ReadAt { id, offset, max } => Box::pin(async move {
                // Leased like the byte ops, and abortable by `close` on the handle;
                // the seek+read pair is safe because the lease means one op owns
                // the cursor at a time (pread semantics from the caller's view).
                let lease = match SeekableLease::take(&inner, id) {
                    Ok(l) => l,
                    Err(e) => return IoResult::Err(e),
                };
                let (abort, reg) = AbortHandle::new_pair();
                inner.op_aborts.borrow_mut().insert(id, abort);
                let res = Abortable::new(
                    async move {
                        let mut lease = lease;
                        let r = async {
                            use futures_lite::AsyncSeekExt;
                            lease.file().seek(std::io::SeekFrom::Start(offset)).await?;
                            // Fill to `max` or EOF: a single read may return short
                            // mid-file, and "short only at EOF" is the contract.
                            let mut buf = vec![0u8; max];
                            let mut filled = 0usize;
                            while filled < max {
                                let n = lease.file().read(&mut buf[filled..]).await?;
                                if n == 0 {
                                    break;
                                }
                                filled += n;
                            }
                            buf.truncate(filled);
                            Ok::<Vec<u8>, std::io::Error>(buf)
                        }
                        .await;
                        drop(lease);
                        r
                    },
                    reg,
                )
                .await;
                match res {
                    Ok(Ok(bytes)) => IoResult::Read(bytes),
                    Ok(Err(e)) => IoResult::Err(e.into()),
                    Err(_aborted) => IoResult::Err(IoError {
                        kind: std::io::ErrorKind::NotConnected,
                        message: "file closed while a read was in flight".to_string(),
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
                    // Intentional: this is the child's only borrow and is held across the
                    // await on purpose (see the ChildSlot note above) — everything
                    // concurrent goes through the pid or the cells, never this RefCell.
                    // (Lint allowed on `perform` — statement scope doesn't suppress it.)
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

            IoRequest::Resolve { host } => Box::pin(async move {
                // getaddrinfo on the blocking pool, exactly as `Connect` does
                // internally via async-net. Port 0: addresses, not sockets.
                let res = blocking::unblock(move || {
                    use std::net::ToSocketAddrs;
                    (host.as_str(), 0u16)
                        .to_socket_addrs()
                        .map(|it| it.map(|sa| sa.ip().to_string()).collect::<Vec<_>>())
                })
                .await;
                match res {
                    Ok(mut ips) => {
                        // getaddrinfo repeats each address per socket type — dedup,
                        // keeping resolver order.
                        let mut seen = HashSet::new();
                        ips.retain(|ip| seen.insert(ip.clone()));
                        IoResult::Resolved(ips)
                    }
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::ResolveReverse { addr } => Box::pin(async move {
                let ip: std::net::IpAddr = match addr.parse() {
                    Ok(ip) => ip,
                    Err(_) => {
                        return IoResult::Err(IoError {
                            kind: std::io::ErrorKind::InvalidInput,
                            message: format!("reverse: not an IP address: '{addr}'"),
                        });
                    }
                };
                let name = blocking::unblock(move || reverse_lookup(ip)).await;
                IoResult::Resolved(name.into_iter().collect())
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
        let _ = self.inner.seekables.borrow_mut().remove(&id);
        // A drop-close is a complete close: nothing left to finish at teardown.
        let _ = self.inner.finish_pending.borrow_mut().remove(&id);
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

    fn needs_finish(&self, id: StreamId) -> bool {
        self.inner.finish_pending.borrow().contains(&id)
    }

    fn reap_child(&self, id: u64) {
        // Kill (if still running), then drop the table entry. A parked wait holds
        // its own `Rc` — the kill resolves it with the signal exit; the `Child`
        // itself drops when the last Rc goes, and async-process's global reaper
        // collects the zombie.
        let slot = self.inner.children.borrow_mut().remove(&id);
        if let Some(slot) = slot
            && slot.exited.get().is_none()
            && !slot.detached.get()
        {
            #[cfg(unix)]
            unsafe {
                libc::kill(slot.pid as libc::pid_t, libc::SIGKILL);
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
