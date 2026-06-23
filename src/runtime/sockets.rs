use crate::arg;
use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::value::{AnyCollect, Block, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::collect::Trace;
use gc_arena::{Gc, Mutation};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// Native backing state for a socket handle — shared by `TcpSocket` and `TlsSocket`,
/// which differ only in how they are *created* (plaintext connect vs. TLS handshake).
/// Once open, both are just a `StreamId` into the backend registry and read/write/close
/// identically. Holds only the integer id (the real stream lives in the backend
/// registry, outside the arena) plus a clone of the VM's reap queue. No `Gc` fields. On
/// close/collection the fd is reaped: `close` pushes the id directly; the `Drop`
/// (collection of an un-closed handle) pushes it as the backstop. The driver sync-closes
/// drained ids. See `docs/ASYNC_ARCH.md`.
pub struct NativeSocket {
    id: StreamId,
    reap: Rc<RefCell<Vec<StreamId>>>,
    closed: bool,
}

impl NativeSocket {
    fn id(&self) -> StreamId {
        self.id
    }

    fn is_closed(&self) -> bool {
        self.closed
    }

    /// Mark closed; returns the previous `closed` flag. Two callers rely on that return:
    /// `reap_handle` enqueues the fd only on the first close (so double-close is a
    /// no-op), and the wrap-consume path (`tls_wrap`) calls this *without* reaping, to
    /// hand the fd off to the TLS layer rather than close it.
    fn mark_closed(&mut self) -> bool {
        std::mem::replace(&mut self.closed, true)
    }
}

impl std::fmt::Debug for NativeSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeSocket{{id:{} closed:{}}}", self.id.0, self.closed)
    }
}

impl AnyCollect for NativeSocket {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields
}

impl Drop for NativeSocket {
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
    let builder = NativeClassBuilder::new("TcpSocket", Some("Object"))
        // TcpSocket.connect:'host:port' -> a connected socket. The caller owns it
        // (close it, or rely on the GC reap backstop). DNS is resolved internally.
        .class_method("connect:", |vm, mc, _receiver, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            match vm.await_io(IoRequest::Connect { host, port })? {
                IoResult::Connected(id) => Ok(make_socket(vm, mc, "TcpSocket", id)),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(unexpected("connect:", other)),
            }
        })
        // TcpSocket.connect:'host:port' do:{|s| ...} -> run the block with the socket,
        // closing it on exit (normal, throw, or cancel); returns the block's value.
        .class_method("connect:do:", |vm, mc, _receiver, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            let handle = match vm.await_io(IoRequest::Connect { host, port })? {
                IoResult::Connected(id) => make_socket(vm, mc, "TcpSocket", id),
                IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
                other => return Err(unexpected("connect:do:", other)),
            };
            // Extract the block only *after* the await (as everywhere in this file): a
            // `Gc` pulled out before a suspend would be live across it.
            let block = arg!(args, Block, 1);
            scope_socket(vm, mc, handle, block)
        });
    add_socket_methods(builder)
}

pub fn build_tls_socket_class() -> NativeClassBuilder {
    let builder = NativeClassBuilder::new("TlsSocket", Some("Object"))
        // TLS from byte zero: open a plaintext connection and immediately hand it to the
        // handshake. The bare selectors forward `insecure = false` to the canonical
        // `insecure:`-bearing form; `insecure: true` skips certificate validation (local
        // debugging — the word at the call site is the warning).
        .class_method("connect:", |vm, mc, _r, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            tls_connect(vm, mc, host, port, false)
        })
        .class_method("connect:insecure:", |vm, mc, _r, args| {
            let hostport = arg!(args, String, 0);
            let insecure = arg!(args, Bool, 1);
            let (host, port) = parse_host_port(hostport.as_str())?;
            tls_connect(vm, mc, host, port, insecure)
        })
        .class_method("connect:do:", |vm, mc, _r, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            let handle = tls_connect(vm, mc, host, port, false)?;
            let block = arg!(args, Block, 1);
            scope_socket(vm, mc, handle, block)
        })
        .class_method("connect:insecure:do:", |vm, mc, _r, args| {
            let hostport = arg!(args, String, 0);
            let insecure = arg!(args, Bool, 1);
            let (host, port) = parse_host_port(hostport.as_str())?;
            let handle = tls_connect(vm, mc, host, port, insecure)?;
            let block = arg!(args, Block, 2);
            scope_socket(vm, mc, handle, block)
        })
        // Upgrade an already-connected TcpSocket to TLS in place (STARTTLS et al.). The
        // `host` is the certificate / SNI name (supplied explicitly — you may have
        // connected by IP or via a proxy). The TcpSocket is *consumed*; see `tls_wrap`.
        .class_method("wrap:host:", |vm, mc, _r, args| {
            let tcp = args[0];
            let host = arg!(args, String, 1).as_str().to_string();
            tls_wrap(vm, mc, tcp, host, false)
        })
        .class_method("wrap:host:insecure:", |vm, mc, _r, args| {
            let tcp = args[0];
            let host = arg!(args, String, 1).as_str().to_string();
            let insecure = arg!(args, Bool, 2);
            tls_wrap(vm, mc, tcp, host, insecure)
        })
        .class_method("wrap:host:do:", |vm, mc, _r, args| {
            let tcp = args[0];
            let host = arg!(args, String, 1).as_str().to_string();
            let handle = tls_wrap(vm, mc, tcp, host, false)?;
            let block = arg!(args, Block, 2);
            scope_socket(vm, mc, handle, block)
        })
        .class_method("wrap:host:insecure:do:", |vm, mc, _r, args| {
            let tcp = args[0];
            let host = arg!(args, String, 1).as_str().to_string();
            let insecure = arg!(args, Bool, 2);
            let handle = tls_wrap(vm, mc, tcp, host, insecure)?;
            let block = arg!(args, Block, 3);
            scope_socket(vm, mc, handle, block)
        });
    add_socket_methods(builder)
}

/// Add the byte-level methods common to every socket kind. `TcpSocket` and `TlsSocket`
/// share these verbatim: once a handle is open, the operations key only on its
/// `StreamId` and are oblivious to whether the conduit is plaintext or TLS.
fn add_socket_methods(builder: NativeClassBuilder) -> NativeClassBuilder {
    builder
        // read:n -> up to n bytes as Bytes (empty = EOF). Throws on a closed socket
        // or an I/O error.
        .typed_instance_method("read:", &["Integer"], |vm, mc, receiver, args| {
            let id = open_id(receiver)?;
            let n = arg!(args, Int, 0).max(0) as usize;
            match vm.await_io(IoRequest::Read { id, max: n })? {
                IoResult::Read(bytes) => Ok(vm.new_bytes(mc, bytes)),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(unexpected("read:", other)),
            }
        })
        // readAll -> read until EOF, returning all bytes as one Bytes.
        .instance_method("readAll", |vm, mc, receiver, _args| {
            let id = open_id(receiver)?;
            let mut all = Vec::new();
            loop {
                match vm.await_io(IoRequest::Read { id, max: 8192 })? {
                    IoResult::Read(chunk) if chunk.is_empty() => break, // EOF
                    IoResult::Read(chunk) => all.extend_from_slice(&chunk),
                    IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
                    other => return Err(unexpected("readAll", other)),
                }
            }
            Ok(vm.new_bytes(mc, all))
        })
        // writeAll:bytes -> write all of `bytes` (complete-or-throw); returns nil.
        .typed_instance_method("writeAll:", &["Bytes"], |vm, mc, receiver, args| {
            let id = open_id(receiver)?;
            let bytes = arg!(args, Bytes, 0).to_vec(); // owned, before the await
            match vm.await_io(IoRequest::Write { id, bytes })? {
                IoResult::Wrote(_) => Ok(vm.new_nil(mc)),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
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
                .with_native_state::<NativeSocket, _, _>(|s| s.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
        // byteStream -> a buffered `ByteStream` that *consumes* this socket: the fd
        // transfers to the stream and the socket is left closed (further ops throw). The
        // `ByteStream.over:` class form is equivalent.
        .instance_method("byteStream", |vm, mc, receiver, _args| {
            let id = match consume_socket(mc, receiver)? {
                Some(id) => id,
                None => {
                    return Err(QuoinError::io_closed(
                        "byteStream: the socket is already closed",
                    ));
                }
            };
            Ok(crate::runtime::streams::make_byte_stream(vm, mc, id))
        })
        // stringStream -> a text `StringStream` consuming this socket directly (= a
        // `byteStream` immediately wrapped). Starts with an empty buffer.
        .instance_method("stringStream", |vm, mc, receiver, _args| {
            let id = match consume_socket(mc, receiver)? {
                Some(id) => id,
                None => {
                    return Err(QuoinError::io_closed(
                        "stringStream: the socket is already closed",
                    ));
                }
            };
            Ok(crate::runtime::streams::make_string_stream(
                vm,
                mc,
                id,
                Vec::new(),
            ))
        })
}

/// Consume a socket handle, handing its fd up to a higher layer (a `ByteStream`): read the
/// `StreamId`, mark the socket closed *without* reaping (the fd transfers, exactly as
/// `tls_wrap` does for a TLS upgrade), and return the id. `Ok(None)` if the socket was
/// already closed; errors only if `value` is not a socket. The consumed handle's `Drop`
/// backstop is disarmed by the `mark_closed`, so it won't reap the id now owned upstream.
pub(crate) fn consume_socket<'gc>(
    mc: &Mutation<'gc>,
    value: Value<'gc>,
) -> Result<Option<StreamId>, QuoinError> {
    value
        .with_native_state_mut::<NativeSocket, _, _>(mc, |s| {
            if s.is_closed() {
                None
            } else {
                let id = s.id();
                s.mark_closed(); // no reap: the fd moves into the stream layer
                Some(id)
            }
        })
        .map_err(QuoinError::Other)
}

/// TLS from byte zero: connect plaintext, then upgrade. No intermediate `TcpSocket`
/// handle is materialized — only the raw `StreamId` (plain `Copy` data, safe across the
/// await) threads between the two ops — so there is nothing to consume here. The SNI /
/// certificate name is the host part of `host:port`.
fn tls_connect<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    host: String,
    port: u16,
    insecure: bool,
) -> Result<Value<'gc>, QuoinError> {
    let domain = host.clone();
    let id = match vm.await_io(IoRequest::Connect { host, port })? {
        IoResult::Connected(id) => id,
        IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
        other => return Err(unexpected("connect:", other)),
    };
    match vm.await_io(IoRequest::TlsWrap {
        id,
        domain,
        insecure,
    })? {
        IoResult::Connected(id) => Ok(make_socket(vm, mc, "TlsSocket", id)),
        // Handshake failed: the backend dropped the underlying stream (fd closed).
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("connect:", other)),
    }
}

/// Upgrade an already-connected `TcpSocket` to TLS *in place* (STARTTLS and friends).
///
/// The plaintext socket is **consumed**: its fd is handed off to the TLS layer, and the
/// original `TcpSocket` handle is left closed so it can neither be used again nor double
/// its fd back onto the reap queue. That consume step is the subtle part and easy to get
/// wrong, so concretely:
///
/// We read the `StreamId` out of the `TcpSocket`, then call `mark_closed()` *without*
/// enqueueing a reap (unlike `close`/`reap_handle`, which do enqueue). Skipping the reap
/// is load-bearing for two reasons:
///   1. The fd is NOT being closed here. `TlsWrap` takes that very stream out of the
///      backend registry and wraps it, so the underlying connection lives on inside the
///      new `TlsStream`. Reaping would close the connection we are upgrading.
///   2. Setting `closed = true` also disarms the consumed handle's `Drop` backstop
///      (which reaps only `if !closed`). So when that now-defunct `TcpSocket` is later
///      collected, it won't push the id — which by then belongs to the live `TlsSocket`
///      — back onto the reap queue and close the fd out from under it.
///
/// On handshake failure the backend has already dropped the stream (fd closed) and we
/// throw; the consumed `TcpSocket` stays closed, which is correct — a failed upgrade
/// leaves a dead connection, not a usable plaintext one.
fn tls_wrap<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    tcp: Value<'gc>,
    host: String,
    insecure: bool,
) -> Result<Value<'gc>, QuoinError> {
    let (id, closed) = tcp
        .with_native_state::<NativeSocket, _, _>(|s| (s.id(), s.is_closed()))
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(QuoinError::io_closed(
            "TlsSocket.wrap:: the socket is already closed",
        ));
    }
    // Consume the plaintext handle: mark closed but do NOT reap (see the doc comment) —
    // the fd is moving into the TLS layer, not being closed.
    tcp.with_native_state_mut::<NativeSocket, _, _>(mc, |s| {
        s.mark_closed();
    })
    .map_err(QuoinError::Other)?;
    match vm.await_io(IoRequest::TlsWrap {
        id,
        domain: host,
        insecure,
    })? {
        IoResult::Connected(id) => Ok(make_socket(vm, mc, "TlsSocket", id)),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("wrap:host:", other)),
    }
}

/// Run `block` with an open socket `handle`, closing it on every exit path (normal,
/// throw, or cancel); returns the block's value. The caller must already have done any
/// awaits needed to obtain `handle` — there is no suspend here, so neither `handle` nor
/// the block result is held across one.
fn scope_socket<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    handle: Value<'gc>,
    block: Gc<'gc, Block<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let result = vm.execute_block(mc, block, vec![handle], None);
    reap_handle(vm, mc, handle);
    result
}

/// Build a socket handle of class `class_name` over `id`, wired to the VM's reap queue.
fn make_socket<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    class_name: &str,
    id: StreamId,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, class_name);
    vm.new_native_state(
        mc,
        class,
        NativeSocket {
            id,
            reap: vm.socket_reap.clone(),
            closed: false,
        },
    )
}

/// The `StreamId` of an open socket receiver, or a thrown error if it is closed.
fn open_id<'gc>(receiver: Value<'gc>) -> Result<StreamId, QuoinError> {
    let (id, closed) = receiver
        .with_native_state::<NativeSocket, _, _>(|s| (s.id(), s.is_closed()))
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(QuoinError::io_closed(
            "socket: operation on a closed socket",
        ));
    }
    Ok(id)
}

/// Mark a handle closed (idempotent) and enqueue its fd for the driver to reap.
fn reap_handle<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, handle: Value<'gc>) {
    let to_reap = handle
        .with_native_state_mut::<NativeSocket, _, _>(mc, |s| {
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
            let port = port
                .parse::<u16>()
                .map_err(|_| QuoinError::Other(format!("socket connect: bad port in '{}'", s)))?;
            Ok((host.to_string(), port))
        }
        _ => Err(QuoinError::Other(format!(
            "socket connect: expected 'host:port', got '{}'",
            s
        ))),
    }
}

fn unexpected(op: &str, got: IoResult) -> QuoinError {
    QuoinError::Other(format!("socket {op}: unexpected I/O result {got:?}"))
}

// ===========================================================================
// TcpListener — Stage 7: accept incoming connections (QN servers).
// ===========================================================================

/// Native backing for a listening socket. Like `NativeSocket` it is just an fd in the
/// backend registry (reaped via the same queue), plus the actual bound `port` (so a `:0`
/// ephemeral bind is usable). A listener accepts connections rather than reading/writing,
/// so it is a distinct class — but the resource lifecycle is identical.
pub struct NativeListener {
    id: StreamId,
    reap: Rc<RefCell<Vec<StreamId>>>,
    closed: bool,
    port: u16,
}

impl NativeListener {
    fn id(&self) -> StreamId {
        self.id
    }
    fn is_closed(&self) -> bool {
        self.closed
    }
    fn port(&self) -> u16 {
        self.port
    }
    fn mark_closed(&mut self) -> bool {
        std::mem::replace(&mut self.closed, true)
    }
}

impl std::fmt::Debug for NativeListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NativeListener{{id:{} port:{} closed:{}}}",
            self.id.0, self.port, self.closed
        )
    }
}

impl AnyCollect for NativeListener {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields
}

impl Drop for NativeListener {
    fn drop(&mut self) {
        if !self.closed {
            self.reap.borrow_mut().push(self.id);
        }
    }
}

pub fn build_tcp_listener_class() -> NativeClassBuilder {
    NativeClassBuilder::new("TcpListener", Some("Object"))
        // TcpListener.listen:'host:port' -> a bound listening socket. Port 0 binds an
        // ephemeral port; read the chosen port back with `port`.
        .class_method("listen:", |vm, mc, _r, args| {
            let hostport = arg!(args, String, 0);
            let (host, port) = parse_host_port(hostport.as_str())?;
            match vm.await_io(IoRequest::Listen { host, port })? {
                IoResult::Listening { id, port } => Ok(make_listener(vm, mc, id, port)),
                IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
                other => Err(unexpected("listen:", other)),
            }
        })
        // accept -> block until a peer connects, returning the connected TcpSocket. The
        // caller owns it (close it, scope it, or rely on the reap backstop).
        .instance_method("accept", |vm, mc, receiver, _args| {
            accept_one(vm, mc, receiver)
        })
        // acceptOnce:{|sock| ...} -> accept one connection, run the block with it, and
        // close it on exit (normal/throw/cancel); returns the block's value.
        .instance_method("acceptOnce:", |vm, mc, receiver, args| {
            let conn = accept_one(vm, mc, receiver)?;
            // Extract the block only after the await (a Gc pulled out before a suspend
            // would be live across it), as elsewhere in this file.
            let block = arg!(args, Block, 0);
            scope_socket(vm, mc, conn, block)
        })
        // acceptLoop:{|sock| ...} -> accept connections forever, running the block on each
        // (each closed after the block). The caller breaks out with a non-local return
        // (^^) — which, like a throw or cancel, propagates straight through this loop.
        .instance_method("acceptLoop:", |vm, mc, receiver, args| {
            let block = arg!(args, Block, 0);
            accept_loop(vm, mc, receiver, block)
        })
        // port -> the bound local port (useful after a `:0` ephemeral bind).
        .instance_method("port", |vm, mc, receiver, _args| {
            let port = receiver
                .with_native_state::<NativeListener, _, _>(|l| l.port())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_int(mc, port as i64))
        })
        .instance_method("close", |vm, mc, receiver, _args| {
            reap_listener_handle(vm, mc, receiver);
            Ok(vm.new_nil(mc))
        })
        .instance_method("closed?", |vm, mc, receiver, _args| {
            let closed = receiver
                .with_native_state::<NativeListener, _, _>(|l| l.is_closed())
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bool(mc, closed))
        })
}

/// Accept one connection from the listener `receiver`, returning the connected `TcpSocket`.
fn accept_one<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    let id = open_listener_id(receiver)?;
    match vm.await_io(IoRequest::Accept { id })? {
        IoResult::Connected(conn_id) => Ok(make_socket(vm, mc, "TcpSocket", conn_id)),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(unexpected("accept", other)),
    }
}

/// Accept connections forever, running `block` on each (closing each on exit). Returns only
/// by *propagating* a non-`Ok` from the block — a non-local return (`^^`), a throw, or a
/// cancellation — which unwinds straight through this native loop. The accepted socket is
/// closed before propagating, so resources are released on every exit path.
fn accept_loop<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    block: Gc<'gc, Block<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    loop {
        let conn = accept_one(vm, mc, receiver)?;
        let result = vm.execute_block(mc, block, vec![conn], None);
        reap_handle(vm, mc, conn); // close the accepted socket (no-op if the block consumed it)
        result?; // Ok -> accept the next; Err (^^ / throw / cancel) -> propagate out
    }
}

/// The `StreamId` of an open listener receiver, or a thrown error if it is closed.
fn open_listener_id<'gc>(receiver: Value<'gc>) -> Result<StreamId, QuoinError> {
    let (id, closed) = receiver
        .with_native_state::<NativeListener, _, _>(|l| (l.id(), l.is_closed()))
        .map_err(QuoinError::Other)?;
    if closed {
        return Err(QuoinError::io_closed(
            "listener: operation on a closed listener",
        ));
    }
    Ok(id)
}

/// Build a `TcpListener` handle over `id`, storing the bound `port`, wired to the reap queue.
fn make_listener<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    id: StreamId,
    port: u16,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "TcpListener");
    vm.new_native_state(
        mc,
        class,
        NativeListener {
            id,
            reap: vm.socket_reap.clone(),
            closed: false,
            port,
        },
    )
}

/// Mark a listener closed (idempotent) and enqueue its fd for the driver to reap.
fn reap_listener_handle<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, handle: Value<'gc>) {
    let to_reap = handle
        .with_native_state_mut::<NativeListener, _, _>(mc, |l| {
            if l.mark_closed() { None } else { Some(l.id()) }
        })
        .ok()
        .flatten();
    if let Some(id) = to_reap {
        vm.socket_reap.borrow_mut().push(id);
    }
}
