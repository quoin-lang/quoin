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
use std::collections::{HashMap, VecDeque};
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
    /// Read up to `max` bytes. An empty result means EOF.
    Read { id: StreamId, max: usize },
    /// Write all of `bytes`.
    Write { id: StreamId, bytes: Vec<u8> },
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
    /// Bind a listening TCP socket on `host:port` (`port` 0 = ephemeral). Registers the
    /// listener and returns `Listening { id, port }` with the actual bound port.
    Listen { host: String, port: u16 },
    /// Accept one connection from the listener `id`, registering the accepted stream and
    /// returning its `Connected(id)`. Parks until a peer connects.
    Accept { id: StreamId },
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
    Wrote(usize),
    Closed,
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
pub trait IoBackend {
    fn perform(&self, req: IoRequest) -> IoFuture;

    /// Synchronously close and deregister a stream (drop it → close the fd). Used by
    /// the reap path: the QN socket handle's `Drop`, and explicit/scope close, push a
    /// `StreamId` onto a non-GC queue the scheduler drains here — no `await`, no task
    /// context. Missing ids are a no-op, so double-close is harmless.
    fn close(&self, id: StreamId);
}

// ---------------------------------------------------------------------------
// SmolBackend — the native implementation, on `async-io`.
// ---------------------------------------------------------------------------

struct SmolInner {
    streams: RefCell<HashMap<StreamId, Box<dyn AsyncStream>>>,
    // Listening sockets live in their own registry: a `TcpListener` accepts connections
    // (it isn't an `AsyncStream`), so it can't share the `streams` map. Same id space.
    listeners: RefCell<HashMap<StreamId, async_net::TcpListener>>,
    next_id: Cell<u64>,
    // TLS connectors are built lazily and cached: loading the webpki root bundle once
    // (rather than per connection) is the whole reason `TlsWrap` is cheap. `unsync`
    // because the VM + backend live on one thread (gc_arena is `!Send`).
    tls_secure: OnceCell<TlsConnector>,
    tls_insecure: OnceCell<TlsConnector>,
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
                next_id: Cell::new(1),
                tls_secure: OnceCell::new(),
                tls_insecure: OnceCell::new(),
            }),
        }
    }
}

/// Look up and remove a stream from the registry so the op can own it by value for
/// the duration of the await (no `RefCell` borrow is held across `.await`). A single
/// stream is only ever used by one fiber, so removing it for the op is safe — and it
/// structurally enforces "no concurrent ops on the same stream". The caller puts it
/// back when the op succeeds; `Close` simply drops it.
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

            IoRequest::Read { id, max } => Box::pin(async move {
                let mut stream = match take_stream(&inner, id) {
                    Ok(s) => s,
                    Err(e) => return IoResult::Err(e),
                };
                let mut buf = vec![0u8; max];
                let res = (&mut *stream).read(&mut buf).await;
                inner.streams.borrow_mut().insert(id, stream);
                match res {
                    Ok(n) => {
                        buf.truncate(n);
                        IoResult::Read(buf)
                    }
                    Err(e) => IoResult::Err(e.into()),
                }
            }),

            IoRequest::Write { id, bytes } => Box::pin(async move {
                let mut stream = match take_stream(&inner, id) {
                    Ok(s) => s,
                    Err(e) => return IoResult::Err(e),
                };
                let res = async {
                    (&mut *stream).write_all(&bytes).await?;
                    (&mut *stream).flush().await?;
                    Ok::<usize, std::io::Error>(bytes.len())
                }
                .await;
                inner.streams.borrow_mut().insert(id, stream);
                match res {
                    Ok(n) => IoResult::Wrote(n),
                    Err(e) => IoResult::Err(e.into()),
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
                // Take the listener out for the accept (no map borrow held across the
                // await — one accept in flight per listener, like the byte ops), then put
                // it back. The accepted stream drops into the shared `AsyncStream` registry.
                let listener = match inner.listeners.borrow_mut().remove(&id) {
                    Some(l) => l,
                    None => {
                        return IoResult::Err(IoError {
                            kind: std::io::ErrorKind::NotFound,
                            message: format!("unknown listener id {}", id.0),
                        });
                    }
                };
                let res = listener.accept().await;
                inner.listeners.borrow_mut().insert(id, listener);
                match res {
                    Ok((stream, _peer)) => IoResult::Connected(inner.insert(Box::new(stream))),
                    Err(e) => IoResult::Err(e.into()),
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
        let _ = self.inner.streams.borrow_mut().remove(&id);
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
            IoRequest::Connect { .. } => {
                let id = StreamId(self.next_id.get());
                self.next_id.set(self.next_id.get() + 1);
                IoResult::Connected(id)
            }
            IoRequest::Read { max, .. } => {
                let mut buf = self.reads.borrow_mut().pop_front().unwrap_or_default();
                buf.truncate(max);
                IoResult::Read(buf)
            }
            IoRequest::Write { bytes, .. } => {
                let n = bytes.len();
                self.writes.borrow_mut().push(bytes);
                IoResult::Wrote(n)
            }
            IoRequest::Close { .. } => IoResult::Closed,
            // No real handshake in the mock — the conduit keeps its id, as in the
            // native backend's in-place swap.
            IoRequest::TlsWrap { id, .. } => IoResult::Connected(id),
            IoRequest::OpenFile { .. } => {
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
        }));
        assert!(matches!(r, IoResult::Read(b) if b == b"hello"));

        // Queue drained → EOF (empty read).
        let r = block_on(mock.perform(IoRequest::Read {
            id: StreamId(0),
            max: 64,
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
            match backend.perform(IoRequest::Read { id, max: 64 }).await {
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
            match backend.perform(IoRequest::Read { id, max: 64 }).await {
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
            match backend.perform(IoRequest::Read { id, max: 256 }).await {
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
