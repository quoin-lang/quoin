"""quoin-ext — the Python SDK for out-of-process Quoin extensions (Tier 1).

An extension is a separate process the Quoin VM spawns and talks to over a unix domain
socket; this package is the thin Python client an extension links against — the polyglot
counterpart of the Rust ``quoin-ext`` crate, speaking the same wire protocol
(``crates/quoin-ext-proto/PROTOCOL.md``).

Wire format: length-prefixed frames (a little-endian ``u32`` length + that many payload
bytes), each payload one MessagePack array ``[type, field, ...]``. The whole frame is one
``msgpack.packb``/``unpackb`` pass — structured values inside it decode straight to native
Python values (dict/list/str/int/float/bool/None/bytes; ``BigInt`` is MessagePack ext
type 1, ``Decimal`` ext type 2). The C ``msgpack`` package is the SDK's only dependency.

This is at parity with the Rust SDK: a handler reads the call's typed args
(:meth:`Host.handles`/:meth:`Host.resources`/:meth:`Host.releases`/:meth:`Host.arrays`),
issues re-entrant host-ops (:meth:`Host.make_string`, :meth:`Host.call_method`,
:meth:`Host.invoke_block`, …), and returns a scalar ``str``, a :class:`Resource`, or an
:class:`ArrowArray`.
"""

import decimal
import os
import socket
import struct
import threading
import time
import traceback

try:
    import msgpack
except ImportError as _e:
    raise ImportError(
        "quoin_ext requires the 'msgpack' package (the wire protocol is MessagePack): "
        "pip install msgpack"
    ) from _e

#: The protocol version this SDK speaks, exchanged in the manifest handshake (the first
#: frames on a fresh connection). The HOST enforces the match — its error reaches the
#: user, while an error raised here would vanish with the process.
PROTOCOL_VERSION = 2

#: Shared frame-size cap (`quoin-ext-proto`'s ``MAX_FRAME_LEN``): refuse a declared length
#: above this before allocating for it — a corrupt prefix must not drive a huge allocation.
MAX_FRAME_LEN = 256 * 1024 * 1024

# Frame type tags (PROTOCOL.md's message table). Append new types; never renumber.
_T_CALL = 0
_T_CALL_RETURN = 1
_T_CALL_RETURN_ERROR = 2
_T_CALL_RETURN_RESOURCE = 3
_T_CALL_RETURN_ARRAY = 4
_T_CALL_RETURN_DATA = 5
_T_CALL_RETURN_HANDLE = 6
_T_GET_MANIFEST = 7
_T_MANIFEST_RETURN = 8
_T_MAKE_STRING = 9
_T_HANDLE_TO_STRING = 10
_T_RETAIN = 11
_T_RELEASE = 12
_T_CALL_METHOD_ON_HANDLE = 13
_T_INVOKE_BLOCK = 14
_T_INVOKE_BLOCK_RETURN = 15
_T_GET_GLOBAL = 16
_T_MAKE_VALUE = 17
_T_READ_HANDLE = 18
_T_READ_HANDLE_RETURN = 19
_T_HOST_OP_RETURN = 20

__all__ = [
    "serve",
    "read_frame",
    "write_frame",
    "Host",
    "Resource",
    "ReturnHandle",
    "ArrowArray",
    "Extension",
    "PROTOCOL_VERSION",
]


# --------------------------------------------------------------------------------------------
# The MessagePack codec
# --------------------------------------------------------------------------------------------


def _pack_default(o):
    """``msgpack.packb`` hook for the non-native value kinds (PROTOCOL.md §Values): an int beyond
    64 bits -> ext type 1 (ASCII digits); Decimal -> ext type 2 (ASCII decimal string); a
    :class:`Resource` (an ext-side resource id, for the generic-`serve` extensions that manage
    their own registry) -> ext type 3."""
    if isinstance(o, decimal.Decimal):
        return msgpack.ExtType(2, str(o).encode())
    if isinstance(o, int):
        return msgpack.ExtType(1, str(o).encode())
    if isinstance(o, Resource):
        return msgpack.ExtType(3, struct.pack("<Q", o.id))
    raise TypeError(f"cannot serialize {type(o).__name__} as a structured value")


def _ext_hook(code, data):
    if code == 1:
        return int(data.decode())
    if code == 2:
        return decimal.Decimal(data.decode())
    if code == 3:
        # A live-instance reference inside a value: 8-byte LE id (+ class name, host-bound only).
        # On the generic path there is no SDK object table, so surface the raw id as a
        # :class:`Resource` for the handler's own registry.
        (rid,) = struct.unpack_from("<Q", data)
        return Resource(rid)
    raise ValueError(f"extension: unknown value ext type {code}")


def _pack(fields):
    """Encode one frame: the message array, in one codec pass."""
    return msgpack.packb(fields, use_bin_type=True, default=_pack_default)


def _stamp_handler_micros(frame, started):
    """Append the ``handler_micros`` field (append-only evolution, PROTOCOL.md) to a packed
    ``CallReturn*`` terminal: how long this side held the call, in microseconds, from decoding
    the ``Call`` to writing its terminal — nested host round-trips included. The host's
    boundary profiling (``VM.boundaryStats``) splits call cost with it. Every terminal is a
    msgpack fixarray (2-4 fields), so appending is a one-byte header bump."""
    head = frame[0]
    if not 0x90 <= head < 0x9F:
        return frame  # defensive: never touch an unexpected shape
    micros = int((time.perf_counter() - started) * 1_000_000)
    return bytes([head + 1]) + bytes(frame[1:]) + msgpack.packb(micros)


def _unpack(frame):
    """Decode one frame to its message array (embedded structured values come out as native
    Python values in the same pass)."""
    msg = msgpack.unpackb(
        frame, raw=False, strict_map_key=False, ext_hook=_ext_hook, use_list=True
    )
    if not isinstance(msg, list) or not msg:
        raise ValueError("extension: malformed frame (not a message array)")
    return msg


def _fields(msg, expected_type, n, name):
    """The message's fields, checked against the expected type tag and arity. Extra trailing
    fields are allowed and ignored (append-only evolution — PROTOCOL.md §Evolution)."""
    if msg[0] != expected_type:
        raise ValueError(f"extension: expected {name}, got frame type {msg[0]}")
    if len(msg) < 1 + n:
        raise ValueError(f"extension: {name} frame has {len(msg) - 1} field(s), needs {n}")
    return msg[1 : 1 + n]


# --------------------------------------------------------------------------------------------
# Reply value types (what a handler may return)
# --------------------------------------------------------------------------------------------


class Resource:
    """A handler return marking an ext-side resource id the host should hold as an opaque token."""

    def __init__(self, id):
        self.id = id


class ReturnHandle:
    """A handler return marking a host-value handle (from `get_global`/`make_value`/`call_method`)
    to return as the call's live result — the host resolves it to the value."""

    def __init__(self, handle):
        self.handle = handle


class ArrowArray:
    """A bulk numeric column (the data plane): a dtype + a contiguous little-endian buffer (Arrow
    non-nullable primitive layout). Used both for reading an `Array` call arg and returning one.
    On the wire: ``[dtype, length, data]``."""

    FLOAT64 = 0
    INT64 = 1

    def __init__(self, dtype, data):
        self.dtype = dtype
        self.data = data  # bytes

    @property
    def length(self):
        return len(self.data) // 8

    def as_floats(self):
        return list(struct.unpack(f"<{self.length}d", self.data))

    def as_ints(self):
        return list(struct.unpack(f"<{self.length}q", self.data))

    @classmethod
    def from_floats(cls, xs):
        return cls(cls.FLOAT64, struct.pack(f"<{len(xs)}d", *xs))

    @classmethod
    def from_ints(cls, xs):
        return cls(cls.INT64, struct.pack(f"<{len(xs)}q", *xs))

    def _wire(self):
        return [self.dtype, self.length, bytes(self.data)]


# --------------------------------------------------------------------------------------------
# Framing
# --------------------------------------------------------------------------------------------


def _recv_exact(conn, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            if not buf:
                return None
            raise EOFError("extension: connection closed mid-frame")
        buf.extend(chunk)
    return bytes(buf)


def read_frame(conn):
    """Read one length-prefixed frame, or ``None`` on a clean EOF (peer closed between frames)."""
    header = _recv_exact(conn, 4)
    if header is None:
        return None
    (length,) = struct.unpack("<I", header)
    if length > MAX_FRAME_LEN:
        raise ValueError(f"extension: frame length {length} exceeds the {MAX_FRAME_LEN} byte limit")
    if length == 0:
        return b""
    payload = _recv_exact(conn, length)
    if payload is None:
        raise EOFError("extension: connection closed before frame payload")
    return payload


def write_frame(conn, payload):
    """Write ``payload`` as one length-prefixed frame."""
    conn.sendall(struct.pack("<I", len(payload)) + payload)


# --------------------------------------------------------------------------------------------
# Frame encoders / decoders
# --------------------------------------------------------------------------------------------


def _encode_manifest_return(classes, lanes=1):
    """The reply to the host's spawn-time ``GetManifest``: this SDK's protocol version plus the
    classes this extension provides, each ``(name, instance_selectors, class_selectors)``. An
    empty list keeps a generic-handler extension backward-compatible. ``lanes`` is the appended
    lane-count declaration (PROTOCOL.md §Evolution); hosts too old to read it skip it."""
    return _pack(
        [
            _T_MANIFEST_RETURN,
            PROTOCOL_VERSION,
            [[name, list(inst), list(cls)] for (name, inst, cls) in classes],
            max(1, lanes),
        ]
    )


def _encode_call_return_error(message, remote_stack="", pack=_pack):
    """A call failed recoverably: the host raises a catchable Quoin error and the extension keeps
    running. A terminal frame, like the other ``CallReturn*`` replies. ``remote_stack`` is this
    extension's OPAQUE stack blob — its traceback, plus any host segments from failed host-ops,
    in unwind order (PROTOCOL.md); the host displays it fenced, never parses it."""
    return pack([_T_CALL_RETURN_ERROR, message, remote_stack])


def _encode_call_return_resource(resource_id, class_name="", pack=_pack):
    # `class_name` (Phase 3) names the registered class the resource is an instance of, so a method
    # can return an instance of any of the extension's classes (cross-class returns); "" = ExtResource.
    return pack([_T_CALL_RETURN_RESOURCE, resource_id, class_name])


def _encode_reply(reply, pack=_pack):
    if isinstance(reply, Resource):
        return _encode_call_return_resource(reply.id, pack=pack)
    if isinstance(reply, ReturnHandle):
        return pack([_T_CALL_RETURN_HANDLE, reply.handle])
    if isinstance(reply, ArrowArray):
        return pack([_T_CALL_RETURN_ARRAY, reply._wire()])
    if isinstance(reply, str):
        return pack([_T_CALL_RETURN, reply])
    # Anything else (None / bool / int / float / Decimal / bytes / list / dict) is a structured
    # value — under a class extension's `pack`, registered instances nested inside it become
    # live-instance references (ext type 3), so a method can return e.g. a list of instances.
    return pack([_T_CALL_RETURN_DATA, reply])


def _decode_arrow(wire):
    if not isinstance(wire, list) or len(wire) < 3:
        raise ValueError("extension: malformed ArrowArray")
    return ArrowArray(wire[0], wire[2])


def _decode_call(msg):
    """A generic-path ``Call``'s ``(op, arg, handles, resources, releases, arrays, data)``."""
    op, arg, handles, resources, releases, arrays, data, _cn, _recv, _margs = _fields(
        msg, _T_CALL, 10, "Call"
    )
    return (op, arg, handles, resources, releases, [_decode_arrow(a) for a in arrays], data)


def _decode_class_call(msg):
    """A ``Call`` for extension-backed-class dispatch (Phase 3): the selector (``op``), the
    ``class_name`` it routes to, the receiver instance id (``recv``, 0 = class-side), the dropped-
    instance ids (``releases``), and the ordered, tagged method arguments (``method_args``)."""
    op, _arg, _h, _r, releases, _arrays, _data, class_name, recv, method_args = _fields(
        msg, _T_CALL, 10, "Call"
    )
    args = []
    for a in method_args:
        if not isinstance(a, list) or len(a) < 2:
            raise ValueError("extension: malformed method argument")
        kind, payload = a[0], a[1]
        if kind == 0:
            args.append(("data", payload))
        elif kind == 1:
            args.append(("resource", payload))
        elif kind == 2:
            args.append(("handle", payload))
        elif kind == 3:
            args.append(("array", _decode_arrow(payload)))
        else:
            raise ValueError(f"extension: unknown Arg kind {kind}")
    return (op, class_name, recv, releases, args)


# --------------------------------------------------------------------------------------------
# The host-callback client + serve loop
# --------------------------------------------------------------------------------------------


class Host:
    """The host-callback client for the duration of one `Call`. Exposes the call's typed args and
    issues re-entrant host-ops over the connection (each a synchronous round-trip the host
    services while parked on the reply). Mirrors the Rust `Host`."""

    def __init__(self, conn, handles, resources, releases, arrays, data, nested=None):
        self._conn = conn
        self._handles = handles
        self._resources = resources
        self._releases = releases
        self._arrays = arrays
        self._data = data
        # Services a nested ``Call`` (raw frame -> encoded reply) arriving while a host-op
        # awaits its reply — the host re-entering this extension from inside a block/method
        # it is servicing for us. ``None`` (the generic path) answers it with a recoverable
        # error instead. The conversation is a call stack over the socket, strictly LIFO.
        self._nested = nested
        # Host-sent stack segments from FAILED host-ops (a Quoin block that raised),
        # appended to this call's remote_stack when the failure propagates — the
        # cross-process interleave, in unwind order.
        self.error_stacks = []

    # --- the call's arguments ---
    def handles(self):
        return self._handles

    def resources(self):
        return self._resources

    def releases(self):
        return self._releases

    def arrays(self):
        return self._arrays

    def data(self):
        """The structured-value payload passed via `call:with:data:`, as a native Python value
        (dict/list/str/int/float/bool/None/bytes), or `None` if absent."""
        return self._data

    # --- re-entrant host-ops ---
    def make_string(self, value):
        handle, _ = self._host_op([_T_MAKE_STRING, value])
        return handle

    def handle_to_string(self, handle):
        _, s = self._host_op([_T_HANDLE_TO_STRING, handle])
        if s is None:
            raise ValueError("extension: HandleToString reply carried no string")
        return s

    def retain(self, handle):
        self._host_op([_T_RETAIN, handle])

    def release(self, handles):
        self._host_op([_T_RELEASE, list(handles)])

    def call_method(self, receiver, selector, args):
        handle, _ = self._host_op([_T_CALL_METHOD_ON_HANDLE, receiver, selector, list(args)])
        return handle

    def invoke_block(self, block, batches):
        reply = self._round_trip([_T_INVOKE_BLOCK, block, [list(t) for t in batches]])
        results, error = _fields(reply, _T_INVOKE_BLOCK_RETURN, 2, "InvokeBlockReturn")
        if error is not None:
            self._note_remote_stack(reply, 3)
            raise RuntimeError(error)
        return results

    def apply_block(self, block, inputs):
        """Apply a host block to each input value (one batched round-trip), returning one result per
        input as a native Python value — the unary `v map: { |x| … }` mapping form."""
        handles = [self.make_value(d) for d in inputs]
        results = self.invoke_block(block, [[h] for h in handles])
        return [self.read_handle(h) for h in results]

    # --- host reach (Phase 2) ---
    def get_global(self, name):
        """Resolve a name in the host's globals (a class is a class-valued global), returning a
        handle to its value — e.g. `call_method(host.get_global("Array"), "ofFloats:", [list])`."""
        handle, _ = self._host_op([_T_GET_GLOBAL, name])
        return handle

    def make_value(self, obj):
        """Construct any host value from a native Python value, returning a handle to it (for
        building non-string method arguments). The general form of `make_string`."""
        handle, _ = self._host_op([_T_MAKE_VALUE, obj])
        return handle

    def read_handle(self, handle):
        """Project the value behind `handle` to a native Python value — inspect any handle as data
        (the general form of `handle_to_string`)."""
        reply = self._round_trip([_T_READ_HANDLE, handle])
        value, error = _fields(reply, _T_READ_HANDLE_RETURN, 2, "ReadHandleReturn")
        if error is not None:
            self._note_remote_stack(reply, 3)
            raise RuntimeError(error)
        return value

    # --- internals ---
    def _round_trip(self, fields):
        write_frame(self._conn, _pack(fields))
        while True:
            reply = read_frame(self._conn)
            if reply is None:
                raise EOFError("extension: host closed during a host-op")
            msg = _unpack(reply)
            if not (msg and msg[0] == _T_CALL):
                return msg
            # A nested Call: service it (or refuse it recoverably) and keep waiting for
            # our own reply — it always follows the nested call's completion (LIFO).
            started = time.perf_counter()
            if self._nested is None:
                write_frame(
                    self._conn,
                    _stamp_handler_micros(
                        _encode_call_return_error(
                            "nested extension call: this extension's generic handler "
                            "cannot service a re-entrant call"
                        ),
                        started,
                    ),
                )
            else:
                write_frame(
                    self._conn, _stamp_handler_micros(self._nested(reply), started)
                )

    def _host_op(self, fields):
        reply = self._round_trip(fields)
        handle, s, error = _fields(reply, _T_HOST_OP_RETURN, 3, "HostOpReturn")
        if error is not None:
            self._note_remote_stack(reply, 4)
            raise RuntimeError(error)
        return handle, s

    def _note_remote_stack(self, reply, index):
        """Record the appended host stack segment from a failed host-op reply, when the host
        is new enough to send one (append-only field evolution)."""
        if len(reply) > index and reply[index]:
            self.error_stacks.append(reply[index])


def serve(path, handler):
    """Bind a unix socket at ``path``, accept one host connection, and serve calls until the host
    disconnects. Each ``Call`` invokes ``handler(host, op, arg)``; its return value is sent back as
    the call's result: a ``str`` -> scalar, a :class:`Resource`, an :class:`ArrowArray`, or any other
    value (``dict``/``list``/``int``/``float``/``bool``/``None``/``bytes``/``Decimal``) -> a
    structured value that materializes as a nested Quoin Value.

    Blocking and single-connection by design: the extension is its own process and the VM holds
    exactly one connection to it. Returns once the host disconnects.
    """
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(path)
    server.listen(1)
    try:
        conn, _ = server.accept()
        # Unlink once the host is connected: the established connection is unaffected, the
        # protocol never reconnects, and this is the only cleanup that survives a signal death
        # of either process. Mirrors `serve` in the Rust SDK (crates/quoin-ext/src/lib.rs).
        try:
            os.unlink(path)
        except OSError:
            pass
        try:
            while True:
                frame = read_frame(conn)
                if frame is None:
                    break
                msg = _unpack(frame)
                # Phase 3: the host asks for a class manifest once, right after connect. A
                # generic-handler extension provides none; everything else is a Call. The reply
                # carries this SDK's protocol version; the HOST enforces the handshake.
                if msg[0] == _T_GET_MANIFEST:
                    write_frame(conn, _encode_manifest_return([]))
                    continue
                started = time.perf_counter()
                op, arg, handles, resources, releases, arrays, data = _decode_call(msg)
                host = Host(conn, handles, resources, releases, arrays, data)
                # A handler exception becomes a catchable Quoin error; the extension keeps
                # serving. Its traceback (plus any host segments from failed host-ops)
                # becomes the opaque cross-process stack blob.
                try:
                    reply = _encode_reply(handler(host, op, arg))
                except Exception as exc:  # noqa: BLE001 — any handler error maps to a catchable error
                    remote = (
                        f"in {op}\n" + traceback.format_exc() + "".join(host.error_stacks)
                    )
                    reply = _encode_call_return_error(str(exc), remote)
                write_frame(conn, _stamp_handler_micros(reply, started))
        finally:
            conn.close()
    finally:
        server.close()


# --------------------------------------------------------------------------------------------
# Extension-backed classes (Phase 3): the SDK owns the object table.
#
# Provide a Quoin class by registering a plain Python class plus a selector -> callable mapping.
# The SDK keeps the instances (a dict keyed by an opaque id the host holds), answers the spawn-time
# `GetManifest`, and routes each method `Call`. Unlike the Rust SDK there is no explicit `makes`:
# a method that returns an instance of a registered class is detected with `isinstance` and wrapped
# as a new instance; everything else is sent back as structured data.
# --------------------------------------------------------------------------------------------


class _ClassReg:
    """One registered class: the Python type plus its class-side (constructor) and instance-side
    (method) selector tables, mapping a Quoin selector to a Python callable."""

    def __init__(self, name, cls, constructors, methods):
        self.name = name
        self.cls = cls
        self.constructors = constructors  # selector -> (*args) -> instance
        self.methods = methods  # selector -> (instance, *args) -> value-or-instance


class _ObjectTable:
    """The SDK-owned instance table: live instances keyed by an opaque id (the resource id the host
    holds). Ids start at 1, so ``recv == 0`` unambiguously means a class-side send. The lock is
    structural (id allotment + map ops): lane threads share the table, but the host never issues
    two concurrent calls to one instance, so instance state itself needs no locking here."""

    def __init__(self):
        self._objects = {}
        self._next_id = 0
        self._lock = threading.Lock()

    def insert(self, obj):
        with self._lock:
            self._next_id += 1
            self._objects[self._next_id] = obj
            return self._next_id

    def get(self, oid):
        with self._lock:
            return self._objects.get(oid)

    def remove(self, oid):
        with self._lock:
            self._objects.pop(oid, None)


class _HostBlock:
    """A host block passed as a method argument (Phase 3). Call it with one value to apply the block
    to that value over the socket, returning the result as a native Python value — so a handler can
    treat it like an ordinary function (e.g. ``[block(x) for x in self.data]``)."""

    def __init__(self, conn, handle, nested=None, sink=None):
        self._host = Host(conn, [], [], [], [], None, nested=nested)
        if sink is not None:
            # Share the per-call collector: a failed application's host segment must reach
            # the DISPATCH that reports the failure, not die with this block wrapper.
            self._host.error_stacks = sink
        self._handle = handle

    def __call__(self, value):
        return self._host.apply_block(self._handle, [value])[0]


class Extension:
    """A class-providing extension (Phase 3). Register classes with :meth:`register`, then
    :meth:`serve`. The SDK owns the instances, so writing an extension class is just writing a plain
    Python class plus a selector -> method mapping."""

    def __init__(self, lanes=1):
        """``lanes`` declares how many lane connections this extension serves (default 1). A
        count above 1 invites the host to open that many connections and issue calls on all of
        them concurrently — one conversation per lane, each serviced on its own thread — so
        calls to different instances can overlap (the host still serializes calls to any one
        instance). Handlers that block with the GIL released (sockets, files, native kernels)
        genuinely overlap; pure-Python compute should stay at 1. Declaring more than one lane
        asserts your handlers tolerate that concurrency."""
        if not 1 <= lanes <= 1024:
            raise ValueError(f"Extension lanes must be 1..=1024, got {lanes}")
        self._classes = {}  # name -> _ClassReg
        self._lanes = lanes

    def register(self, name, cls, constructors=None, methods=None):
        """Register the Python class ``cls`` as the Quoin class ``name``. ``constructors`` maps
        class-side selectors to callables ``(*args) -> instance``; ``methods`` maps instance-side
        selectors to callables ``(instance, *args) -> value | instance``. Returns ``self`` for
        chaining."""
        self._classes[name] = _ClassReg(name, cls, constructors or {}, methods or {})
        return self

    def serve(self, path):
        """Bind a unix socket at ``path``, accept the host connection(s), and serve until the
        host disconnects: answer the spawn-time ``GetManifest`` from the registered classes, and
        route each method ``Call`` to its handler — materializing returned instances into the
        table. With ``lanes`` above 1, up to ``lanes - 1`` further host connections are accepted
        and each is served on its own (daemon) thread over the shared table; a host too old to
        open them costs nothing — the accept thread idles until the process exits."""
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(path)
        server.listen(max(1, self._lanes))
        table = _ObjectTable()
        registered_types = tuple(reg.cls for reg in self._classes.values())
        try:
            conn, _ = server.accept()
            if self._lanes > 1:
                # The host connects the extra lanes after reading the manifest on the first
                # connection; accept them as they come. Daemon threads: when the first
                # connection closes, `serve` returns and the process exits — an accept
                # thread still parked (an older host never opens extras) must not block that.
                def _accept_lanes():
                    accepted = 1
                    while accepted < self._lanes:
                        try:
                            extra, _ = server.accept()
                        except OSError:
                            break
                        accepted += 1
                        threading.Thread(
                            target=self._serve_conn,
                            args=(extra, table, registered_types),
                            daemon=True,
                        ).start()

                threading.Thread(target=_accept_lanes, daemon=True).start()
            self._serve_conn(conn, table, registered_types)
        finally:
            server.close()

    def _serve_conn(self, conn, table, registered_types):
        """Serve one connection (one lane) to completion: the shared frame loop behind
        :meth:`serve`. Only the first connection ever sees ``GetManifest``, but answering it is
        stateless, so every lane handles it uniformly."""
        try:
            while True:
                frame = read_frame(conn)
                if frame is None:
                    break
                msg = self._unpack_frame(table, frame)
                if msg[0] == _T_GET_MANIFEST:
                    write_frame(
                        conn, _encode_manifest_return(self._manifest(), self._lanes)
                    )
                    continue
                started = time.perf_counter()
                write_frame(
                    conn,
                    _stamp_handler_micros(
                        self._dispatch(conn, msg, table, registered_types), started
                    ),
                )
        finally:
            conn.close()

    # --- the table-aware codec: live-instance references (ext type 3) inside values ---

    def _unpack_frame(self, table, frame):
        """Like the module-level ``_unpack``, but a live-instance reference inside a value
        resolves to the live object from this extension's table — so a data payload (e.g. an
        expression graph) can carry instances directly."""

        def ext_hook(code, data):
            if code == 3:
                (rid,) = struct.unpack_from("<Q", data)
                obj = table.get(rid)
                if obj is None:
                    raise ValueError(f"extension: data references no live instance {rid}")
                return obj
            return _ext_hook(code, data)

        msg = msgpack.unpackb(
            frame, raw=False, strict_map_key=False, ext_hook=ext_hook, use_list=True
        )
        if not isinstance(msg, list) or not msg:
            raise ValueError("extension: malformed frame (not a message array)")
        return msg

    def _pack_frame(self, table, fields):
        """Like the module-level ``_pack``, but an instance of a registered class nested inside a
        value is inserted into the table and crosses as a live-instance reference (ext type 3,
        id + class name) — so a method can return e.g. a list of instances."""

        def default(o):
            name = self._class_name_of(o)
            if name:
                rid = table.insert(o)
                return msgpack.ExtType(3, struct.pack("<Q", rid) + name.encode())
            return _pack_default(o)

        return msgpack.packb(fields, use_bin_type=True, default=default)

    def _manifest(self):
        """``(name, instance_selectors, class_selectors)`` for each registered class.
        Selector lists are sorted — the canonical manifest form (dicts would already give
        insertion order, but the Rust SDK's handler maps have no order, so sorted is the
        one form both SDKs can emit; the host's replay tooling fingerprints wire bytes)."""
        return [
            (reg.name, sorted(reg.methods), sorted(reg.constructors))
            for reg in self._classes.values()
        ]

    def _class_name_of(self, obj):
        """The registered Quoin class name for an instance (so a method returning an instance of any
        registered class is wrapped correctly — cross-class returns), or '' if it isn't registered."""
        for reg in self._classes.values():
            if isinstance(obj, reg.cls):
                return reg.name
        return ""

    def _resolve_args(self, raw_args, table, conn, nested=None, sink=None):
        """Resolve the tagged wire args to native Python values: data passes through (any live-
        instance references inside were already resolved by the table-aware unpack), an
        ext-instance id becomes the live instance, an ``Array`` stays an :class:`ArrowArray`, and
        a handle becomes a callable :class:`_HostBlock`. Order is preserved, so the handler
        receives its arguments positionally."""
        out = []
        for kind, val in raw_args:
            if kind in ("data", "array"):
                out.append(val)
            elif kind == "resource":
                obj = table.get(val)
                if obj is None:
                    raise ValueError(f"argument references no live instance {val}")
                out.append(obj)
            else:  # handle
                out.append(_HostBlock(conn, val, nested=nested, sink=sink))
        return out

    def _dispatch(self, conn, msg, table, registered_types):
        """Route one method ``Call`` to its handler and return the terminal reply frame."""
        pack = lambda fields: self._pack_frame(table, fields)  # noqa: E731 — bound reply codec
        op, class_name, recv, releases, raw_args = _decode_class_call(msg)
        # The host batches dropped instances onto `releases`; free them from the table.
        for rid in releases:
            table.remove(rid)
        reg = self._classes.get(class_name)
        if reg is None:
            raise ValueError(f"no extension-backed class '{class_name}'")
        # Blocks carry the nested-call servicer, so a Quoin block this extension invokes
        # may call back into it (re-entrancy): the nested Call is dispatched right here,
        # against the same table, while the block application awaits its result.
        def nested(frame):
            return self._dispatch(
                conn, self._unpack_frame(table, frame), table, registered_types
            )

        # Host-sent stack segments from failed block applications land here (shared with
        # every _HostBlock this call receives), for the cross-process blob on failure.
        collector = []

        def failure(exc):
            header = (
                f"in {class_name}.{op}\n" if recv == 0 else f"in {class_name}#{op} (instance {recv})\n"
            )
            remote = header + traceback.format_exc() + "".join(collector)
            return _encode_call_return_error(str(exc), remote)

        args = self._resolve_args(raw_args, table, conn, nested=nested, sink=collector)
        if recv == 0:
            # Class-side: usually a constructor building a new instance.
            ctor = reg.constructors.get(op)
            if ctor is None:
                raise ValueError(f"no constructor '{op}' on class '{class_name}'")
            # A handler exception is a *recoverable* error: send it as a `CallReturnError` so the
            # host raises a catchable Quoin error and this extension keeps serving (unlike the
            # routing failures above, which are protocol bugs and propagate). Its traceback —
            # the real thing, Python has it for free — plus any host segments become the
            # opaque cross-process stack blob.
            try:
                obj = ctor(*args)
            except Exception as exc:  # noqa: BLE001 — any handler error maps to a catchable error
                return failure(exc)
            # A class-side selector returning a non-instance replies as data, same as the
            # instance-method rule below — so a "constructor" can return a List of instances
            # (numpy's `meshgrid:with:`) or nothing at all (`seed:`).
            if isinstance(obj, registered_types):
                return _encode_call_return_resource(table.insert(obj), self._class_name_of(obj))
            return _encode_reply(obj, pack=pack)
        method = reg.methods.get(op)
        if method is None:
            raise ValueError(f"no method '{op}' on class '{class_name}'")
        instance = table.get(recv)
        if instance is None:
            raise ValueError(f"no live instance {recv}")
        try:
            result = method(instance, *args)
        except Exception as exc:  # noqa: BLE001 — any handler error maps to a catchable error
            return failure(exc)
        # A returned registered instance becomes a new ext-side object; anything else is data
        # (registered instances nested inside it cross as live references — `_pack_frame`).
        if isinstance(result, registered_types):
            return _encode_call_return_resource(table.insert(result), self._class_name_of(result))
        return _encode_reply(result, pack=pack)
