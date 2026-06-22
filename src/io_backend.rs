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
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use async_io::{Async, Timer};
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
    /// Open a TCP connection; on success registers the stream and returns its id.
    Connect { addr: SocketAddr },
    /// Read up to `max` bytes. An empty result means EOF.
    Read { id: StreamId, max: usize },
    /// Write all of `bytes`.
    Write { id: StreamId, bytes: Vec<u8> },
    /// Close and deregister the stream.
    Close { id: StreamId },
}

/// The plain-data outcome of an [`IoRequest`].
#[derive(Clone, Debug)]
pub enum IoResult {
    Slept,
    Connected(StreamId),
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
}

// ---------------------------------------------------------------------------
// SmolBackend — the native implementation, on `async-io`.
// ---------------------------------------------------------------------------

struct SmolInner {
    streams: RefCell<HashMap<StreamId, Box<dyn AsyncStream>>>,
    next_id: Cell<u64>,
}

impl SmolInner {
    fn insert(&self, stream: Box<dyn AsyncStream>) -> StreamId {
        let id = StreamId(self.next_id.get());
        self.next_id.set(self.next_id.get() + 1);
        self.streams.borrow_mut().insert(id, stream);
        id
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
                next_id: Cell::new(1),
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

            IoRequest::Connect { addr } => Box::pin(async move {
                match Async::<std::net::TcpStream>::connect(addr).await {
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
                // Drop the stream (closing the fd) without holding the borrow across
                // any await; missing ids are a no-op so double-close is harmless.
                let _ = inner.streams.borrow_mut().remove(&id);
                IoResult::Closed
            }),
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
        };
        Box::pin(async move { result })
    }
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
            let id = match backend.perform(IoRequest::Connect { addr }).await {
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
}
