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

import decimal
import socket
import struct

import flatbuffers

from . import ext_generated as g

_I64_MIN, _I64_MAX = -(2**63), 2**63 - 1


def _encode_dv(builder, obj):
    """Encode a native Python value as a `DataValueBox`, returning its offset. Children are built
    first (FlatBuffers is bottom-up). None/bool/int/float/Decimal/str/bytes/list/dict are supported;
    `dict` keys are stringified."""
    kind_type, kind_off = _encode_dv_kind(builder, obj)
    g.DataValueBoxStart(builder)
    g.DataValueBoxAddVType(builder, kind_type)
    g.DataValueBoxAddV(builder, kind_off)
    return g.DataValueBoxEnd(builder)


def _encode_dv_kind(b, obj):
    K = g.DataValueKind
    if obj is None:
        g.DvNullStart(b)
        return (K.DvNull, g.DvNullEnd(b))
    if isinstance(obj, bool):  # before int — bool is a subclass of int
        g.DvBoolStart(b)
        g.DvBoolAddV(b, obj)
        return (K.DvBool, g.DvBoolEnd(b))
    if isinstance(obj, int):
        if _I64_MIN <= obj <= _I64_MAX:
            g.DvIntStart(b)
            g.DvIntAddV(b, obj)
            return (K.DvInt, g.DvIntEnd(b))
        s = b.CreateString(str(obj))
        g.DvBigIntStart(b)
        g.DvBigIntAddV(b, s)
        return (K.DvBigInt, g.DvBigIntEnd(b))
    if isinstance(obj, float):
        g.DvFloatStart(b)
        g.DvFloatAddV(b, obj)
        return (K.DvFloat, g.DvFloatEnd(b))
    if isinstance(obj, decimal.Decimal):
        s = b.CreateString(str(obj))
        g.DvDecimalStart(b)
        g.DvDecimalAddV(b, s)
        return (K.DvDecimal, g.DvDecimalEnd(b))
    if isinstance(obj, str):
        s = b.CreateString(obj)
        g.DvStrStart(b)
        g.DvStrAddV(b, s)
        return (K.DvStr, g.DvStrEnd(b))
    if isinstance(obj, (bytes, bytearray)):
        v = b.CreateByteVector(bytes(obj))
        g.DvBytesStart(b)
        g.DvBytesAddV(b, v)
        return (K.DvBytes, g.DvBytesEnd(b))
    if isinstance(obj, (list, tuple)):
        offs = [_encode_dv(b, it) for it in obj]
        g.DvListStartItemsVector(b, len(offs))
        for o in reversed(offs):
            b.PrependUOffsetTRelative(o)
        items = b.EndVector()
        g.DvListStart(b)
        g.DvListAddItems(b, items)
        return (K.DvList, g.DvListEnd(b))
    if isinstance(obj, dict):
        entry_offs = []
        for key, val in obj.items():
            value_off = _encode_dv(b, val)  # build the value box first
            key_off = b.CreateString(str(key))
            g.DvEntryStart(b)
            g.DvEntryAddKey(b, key_off)
            g.DvEntryAddValue(b, value_off)
            entry_offs.append(g.DvEntryEnd(b))
        g.DvMapStartEntriesVector(b, len(entry_offs))
        for o in reversed(entry_offs):
            b.PrependUOffsetTRelative(o)
        entries = b.EndVector()
        g.DvMapStart(b)
        g.DvMapAddEntries(b, entries)
        return (K.DvMap, g.DvMapEnd(b))
    raise TypeError(f"cannot serialize {type(obj).__name__} as a structured value")


def _decode_dv(box):
    """Decode a `DataValueBox` reader into a native Python value."""
    K = g.DataValueKind
    t = box.VType()
    if t in (K.NONE, K.DvNull):
        return None
    tbl = box.V()

    def as_table(cls):
        x = cls()
        x.Init(tbl.Bytes, tbl.Pos)
        return x

    if t == K.DvBool:
        return as_table(g.DvBool).V()
    if t == K.DvInt:
        return as_table(g.DvInt).V()
    if t == K.DvBigInt:
        return int(_text(as_table(g.DvBigInt).V()))
    if t == K.DvFloat:
        return as_table(g.DvFloat).V()
    if t == K.DvDecimal:
        return decimal.Decimal(_text(as_table(g.DvDecimal).V()))
    if t == K.DvStr:
        return _text(as_table(g.DvStr).V())
    if t == K.DvBytes:
        x = as_table(g.DvBytes)
        return bytes(x.V(i) for i in range(x.VLength()))
    if t == K.DvList:
        x = as_table(g.DvList)
        return [_decode_dv(x.Items(i)) for i in range(x.ItemsLength())]
    if t == K.DvMap:
        x = as_table(g.DvMap)
        out = {}
        for i in range(x.EntriesLength()):
            e = x.Entries(i)
            out[_text(e.Key())] = _decode_dv(e.Value())
        return out
    raise ValueError(f"extension: unknown DataValue kind {t}")

__all__ = [
    "serve",
    "read_frame",
    "write_frame",
    "Host",
    "Resource",
    "ReturnHandle",
    "ArrowArray",
]


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


def _encode_get_global(name):
    b = flatbuffers.Builder(64)
    s = b.CreateString(name)
    g.GetGlobalStart(b)
    g.GetGlobalAddName(b, s)
    return _envelope(b, g.Message.GetGlobal, g.GetGlobalEnd(b))


def _encode_make_value(obj):
    b = flatbuffers.Builder(64)
    box = _encode_dv(b, obj)
    g.MakeValueStart(b)
    g.MakeValueAddValue(b, box)
    return _envelope(b, g.Message.MakeValue, g.MakeValueEnd(b))


def _encode_read_handle(handle):
    b = flatbuffers.Builder(64)
    g.ReadHandleStart(b)
    g.ReadHandleAddHandle(b, handle)
    return _envelope(b, g.Message.ReadHandle, g.ReadHandleEnd(b))


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


def _encode_call_return_data(obj):
    b = flatbuffers.Builder(64)
    box = _encode_dv(b, obj)
    g.CallReturnDataStart(b)
    g.CallReturnDataAddValue(b, box)
    return _envelope(b, g.Message.CallReturnData, g.CallReturnDataEnd(b))


def _encode_call_return_handle(handle):
    b = flatbuffers.Builder(64)
    g.CallReturnHandleStart(b)
    g.CallReturnHandleAddHandle(b, handle)
    return _envelope(b, g.Message.CallReturnHandle, g.CallReturnHandleEnd(b))


def _encode_manifest_return():
    """The reply to the host's spawn-time ``GetManifest`` (Phase 3). A generic-handler extension
    provides no classes, so this sends an empty manifest — an absent ``classes`` vector, which the
    host reads as "no provided classes", keeping the generic ``serve`` backward-compatible."""
    b = flatbuffers.Builder(32)
    g.ManifestReturnStart(b)
    return _envelope(b, g.Message.ManifestReturn, g.ManifestReturnEnd(b))


def _encode_reply(reply):
    if isinstance(reply, Resource):
        return _encode_call_return_resource(reply.id)
    if isinstance(reply, ReturnHandle):
        return _encode_call_return_handle(reply.handle)
    if isinstance(reply, ArrowArray):
        return _encode_call_return_array(reply)
    if isinstance(reply, str):
        return _encode_call_return(reply)
    # Anything else (None / bool / int / float / Decimal / bytes / list / dict) is a structured value.
    return _encode_call_return_data(reply)


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
    box = call.Data()
    data = _decode_dv(box) if box is not None else None
    return (_text(call.Op()), _text(call.Arg()), handles, resources, releases, arrays, data)


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

    def __init__(self, conn, handles, resources, releases, arrays, data):
        self._conn = conn
        self._handles = handles
        self._resources = resources
        self._releases = releases
        self._arrays = arrays
        self._data = data

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

    # --- host reach (Phase 2) ---
    def get_global(self, name):
        """Resolve a name in the host's globals (a class is a class-valued global), returning a
        handle to its value — e.g. `call_method(host.get_global("Array"), "ofFloats:", [list])`."""
        handle, _ = self._host_op(_encode_get_global(name))
        return handle

    def make_value(self, obj):
        """Construct any host value from a native Python value, returning a handle to it (for
        building non-string method arguments). The general form of `make_string`."""
        handle, _ = self._host_op(_encode_make_value(obj))
        return handle

    def read_handle(self, handle):
        """Project the value behind `handle` to a native Python value — inspect any handle as data
        (the general form of `handle_to_string`)."""
        reply = self._round_trip(_encode_read_handle(handle))
        env = g.Envelope.GetRootAs(reply, 0)
        if env.MsgType() != g.Message.ReadHandleReturn:
            raise ValueError(f"extension: expected ReadHandleReturn, got {env.MsgType()}")
        r = g.ReadHandleReturn()
        t = env.Msg()
        r.Init(t.Bytes, t.Pos)
        err = _opt_text(r.Error())
        if err is not None:
            raise RuntimeError(err)
        box = r.Value()
        return _decode_dv(box) if box is not None else None

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
        try:
            while True:
                frame = read_frame(conn)
                if frame is None:
                    break
                # Phase 3: the host asks for a class manifest once, right after connect. A
                # generic-handler extension provides none; everything else is a Call.
                if g.Envelope.GetRootAs(frame, 0).MsgType() == g.Message.GetManifest:
                    write_frame(conn, _encode_manifest_return())
                    continue
                op, arg, handles, resources, releases, arrays, data = _decode_call(frame)
                host = Host(conn, handles, resources, releases, arrays, data)
                write_frame(conn, _encode_reply(handler(host, op, arg)))
        finally:
            conn.close()
    finally:
        server.close()
