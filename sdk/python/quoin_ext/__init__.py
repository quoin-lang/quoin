"""quoin-ext — the Python SDK for out-of-process Quoin extensions (Tier 1).

An extension is a separate process the Quoin VM spawns and talks to over a unix domain
socket; this package is the thin Python client an extension links against — the polyglot
counterpart of the Rust ``quoin-ext`` crate, speaking the same ``ext.fbs`` wire protocol.

Wire format: length-prefixed frames (a little-endian ``u32`` length + that many payload
bytes), each payload a FlatBuffers ``Message`` union. The bindings in ``ext_generated.py``
are produced from the shared schema with ``flatc`` (checked in, like the Rust side's planus
output)::

    flatc --python --gen-onefile -o sdk/python/quoin_ext crates/quoin-ext-proto/schema/ext.fbs

``flatc`` is only needed to *regenerate* after a schema change; using the SDK needs only the
pure-Python ``flatbuffers`` runtime (``pip install flatbuffers``).

This is at parity with the Rust SDK: a handler reads the call's typed args
(:meth:`Host.handles`/:meth:`Host.resources`/:meth:`Host.releases`/:meth:`Host.arrays`),
issues re-entrant host-ops (:meth:`Host.make_string`, :meth:`Host.call_method`,
:meth:`Host.invoke_block`, …), and returns a scalar ``str``, a :class:`Resource`, or an
:class:`ArrowArray`.
"""

import socket
import struct

import flatbuffers

from . import ext_generated as g

__all__ = ["serve", "read_frame", "write_frame", "Host", "Resource", "ArrowArray"]


# --------------------------------------------------------------------------------------------
# Reply value types (what a handler may return)
# --------------------------------------------------------------------------------------------


class Resource:
    """A handler return marking an ext-side resource id the host should hold as an opaque token."""

    def __init__(self, id):
        self.id = id


class ArrowArray:
    """A bulk numeric column (the data plane): a dtype + a contiguous little-endian buffer (Arrow
    non-nullable primitive layout). Used both for reading an `Array` call arg and returning one."""

    FLOAT64 = g.ArrowDType.Float64
    INT64 = g.ArrowDType.Int64

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
# FlatBuffers codec helpers
# --------------------------------------------------------------------------------------------


def _text(b):
    return b.decode("utf-8") if b is not None else ""


def _opt_text(b):
    return b.decode("utf-8") if b is not None else None


def _u64_vector(builder, start_vector, items):
    """Build a ``[uint64]`` field vector with `start_vector(builder, n)`; returns the offset."""
    start_vector(builder, len(items))
    for x in reversed(items):
        builder.PrependUint64(x)
    return builder.EndVector()


def _envelope(builder, msg_type, msg_off):
    g.EnvelopeStart(builder)
    g.EnvelopeAddMsgType(builder, msg_type)
    g.EnvelopeAddMsg(builder, msg_off)
    builder.Finish(g.EnvelopeEnd(builder))
    return bytes(builder.Output())


def _build_arrow(builder, array):
    """Build an ``ArrowArray`` table; returns its offset. (Call before opening any other table.)"""
    data_off = builder.CreateByteVector(array.data)
    g.ArrowArrayStart(builder)
    g.ArrowArrayAddDtype(builder, array.dtype)
    g.ArrowArrayAddLength(builder, array.length)
    g.ArrowArrayAddData(builder, data_off)
    return g.ArrowArrayEnd(builder)


# host-op request encoders (ext -> host) -----------------------------------------------------


def _encode_make_string(value):
    b = flatbuffers.Builder(64)
    v = b.CreateString(value)
    g.MakeStringStart(b)
    g.MakeStringAddValue(b, v)
    return _envelope(b, g.Message.MakeString, g.MakeStringEnd(b))


def _encode_handle_to_string(handle):
    b = flatbuffers.Builder(64)
    g.HandleToStringStart(b)
    g.HandleToStringAddHandle(b, handle)
    return _envelope(b, g.Message.HandleToString, g.HandleToStringEnd(b))


def _encode_retain(handle):
    b = flatbuffers.Builder(64)
    g.RetainStart(b)
    g.RetainAddHandle(b, handle)
    return _envelope(b, g.Message.Retain, g.RetainEnd(b))


def _encode_release(handles):
    b = flatbuffers.Builder(64)
    vec = _u64_vector(b, g.ReleaseStartHandlesVector, handles)
    g.ReleaseStart(b)
    g.ReleaseAddHandles(b, vec)
    return _envelope(b, g.Message.Release, g.ReleaseEnd(b))


def _encode_call_method(receiver, selector, args):
    b = flatbuffers.Builder(64)
    sel = b.CreateString(selector)
    argvec = _u64_vector(b, g.CallMethodOnHandleStartArgsVector, args)
    g.CallMethodOnHandleStart(b)
    g.CallMethodOnHandleAddReceiver(b, receiver)
    g.CallMethodOnHandleAddSelector(b, sel)
    g.CallMethodOnHandleAddArgs(b, argvec)
    return _envelope(b, g.Message.CallMethodOnHandle, g.CallMethodOnHandleEnd(b))


def _encode_invoke_block(block, batches):
    b = flatbuffers.Builder(64)
    hl_offsets = []
    for tuple_handles in batches:
        hv = _u64_vector(b, g.HandleListStartHandlesVector, tuple_handles)
        g.HandleListStart(b)
        g.HandleListAddHandles(b, hv)
        hl_offsets.append(g.HandleListEnd(b))
    g.InvokeBlockStartBatchesVector(b, len(hl_offsets))
    for off in reversed(hl_offsets):
        b.PrependUOffsetTRelative(off)
    batchvec = b.EndVector()
    g.InvokeBlockStart(b)
    g.InvokeBlockAddBlock(b, block)
    g.InvokeBlockAddBatches(b, batchvec)
    return _envelope(b, g.Message.InvokeBlock, g.InvokeBlockEnd(b))


# call-return encoders (ext -> host) ---------------------------------------------------------


def _encode_call_return(result):
    b = flatbuffers.Builder(64)
    r = b.CreateString(result)
    g.CallReturnStart(b)
    g.CallReturnAddResult(b, r)
    return _envelope(b, g.Message.CallReturn, g.CallReturnEnd(b))


def _encode_call_return_resource(resource_id):
    b = flatbuffers.Builder(64)
    g.CallReturnResourceStart(b)
    g.CallReturnResourceAddResource(b, resource_id)
    return _envelope(b, g.Message.CallReturnResource, g.CallReturnResourceEnd(b))


def _encode_call_return_array(array):
    b = flatbuffers.Builder(64)
    a = _build_arrow(b, array)
    g.CallReturnArrayStart(b)
    g.CallReturnArrayAddArray(b, a)
    return _envelope(b, g.Message.CallReturnArray, g.CallReturnArrayEnd(b))


def _encode_reply(reply):
    if isinstance(reply, Resource):
        return _encode_call_return_resource(reply.id)
    if isinstance(reply, ArrowArray):
        return _encode_call_return_array(reply)
    return _encode_call_return("" if reply is None else str(reply))


# decoders -----------------------------------------------------------------------------------


def _msg(buf, expected, name):
    env = g.Envelope.GetRootAs(buf, 0)
    if env.MsgType() != expected:
        raise ValueError(f"extension: expected {name}, got msg type {env.MsgType()}")
    return env.Msg()


def _decode_call(buf):
    table = _msg(buf, g.Message.Call, "Call")
    call = g.Call()
    call.Init(table.Bytes, table.Pos)
    handles = [call.Handles(j) for j in range(call.HandlesLength())]
    resources = [call.Resources(j) for j in range(call.ResourcesLength())]
    releases = [call.Releases(j) for j in range(call.ReleasesLength())]
    arrays = []
    for j in range(call.ArraysLength()):
        a = call.Arrays(j)
        data = bytes(a.Data(k) for k in range(a.DataLength()))
        arrays.append(ArrowArray(a.Dtype(), data))
    return (_text(call.Op()), _text(call.Arg()), handles, resources, releases, arrays)


def _decode_host_op_return(buf):
    table = _msg(buf, g.Message.HostOpReturn, "HostOpReturn")
    r = g.HostOpReturn()
    r.Init(table.Bytes, table.Pos)
    return (r.Handle(), _opt_text(r.Str()), _opt_text(r.Error()))


def _decode_invoke_block_return(buf):
    table = _msg(buf, g.Message.InvokeBlockReturn, "InvokeBlockReturn")
    r = g.InvokeBlockReturn()
    r.Init(table.Bytes, table.Pos)
    results = [r.Results(j) for j in range(r.ResultsLength())]
    return (results, _opt_text(r.Error()))


# --------------------------------------------------------------------------------------------
# The host-callback client + serve loop
# --------------------------------------------------------------------------------------------


class Host:
    """The host-callback client for the duration of one `Call`. Exposes the call's typed args and
    issues re-entrant host-ops over the connection (each a synchronous round-trip the host
    services while parked on the reply). Mirrors the Rust `Host`."""

    def __init__(self, conn, handles, resources, releases, arrays):
        self._conn = conn
        self._handles = handles
        self._resources = resources
        self._releases = releases
        self._arrays = arrays

    # --- the call's arguments ---
    def handles(self):
        return self._handles

    def resources(self):
        return self._resources

    def releases(self):
        return self._releases

    def arrays(self):
        return self._arrays

    # --- re-entrant host-ops ---
    def make_string(self, value):
        handle, _ = self._host_op(_encode_make_string(value))
        return handle

    def handle_to_string(self, handle):
        _, s = self._host_op(_encode_handle_to_string(handle))
        if s is None:
            raise ValueError("extension: HandleToString reply carried no string")
        return s

    def retain(self, handle):
        self._host_op(_encode_retain(handle))

    def release(self, handles):
        self._host_op(_encode_release(list(handles)))

    def call_method(self, receiver, selector, args):
        handle, _ = self._host_op(_encode_call_method(receiver, selector, list(args)))
        return handle

    def invoke_block(self, block, batches):
        reply = self._round_trip(_encode_invoke_block(block, [list(t) for t in batches]))
        results, error = _decode_invoke_block_return(reply)
        if error is not None:
            raise RuntimeError(error)
        return results

    # --- internals ---
    def _round_trip(self, frame_bytes):
        write_frame(self._conn, frame_bytes)
        reply = read_frame(self._conn)
        if reply is None:
            raise EOFError("extension: host closed during a host-op")
        return reply

    def _host_op(self, frame_bytes):
        handle, s, error = _decode_host_op_return(self._round_trip(frame_bytes))
        if error is not None:
            raise RuntimeError(error)
        return handle, s


def serve(path, handler):
    """Bind a unix socket at ``path``, accept one host connection, and serve calls until the host
    disconnects. Each ``Call`` invokes ``handler(host, op, arg)``; its return value — a ``str``
    (scalar), a :class:`Resource`, or an :class:`ArrowArray` — is sent back as the call's result.

    Blocking and single-connection by design: the extension is its own process and the VM holds
    exactly one connection to it. Returns once the host disconnects.
    """
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(path)
    server.listen(1)
    try:
        conn, _ = server.accept()
        try:
            while True:
                frame = read_frame(conn)
                if frame is None:
                    break
                op, arg, handles, resources, releases, arrays = _decode_call(frame)
                host = Host(conn, handles, resources, releases, arrays)
                write_frame(conn, _encode_reply(handler(host, op, arg)))
        finally:
            conn.close()
    finally:
        server.close()
