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
    "Extension",
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


def _build_class_decl(b, name, instance_selectors, class_selectors):
    """Build one `ClassDecl` table; returns its offset. All strings and vectors are created before
    the table is opened (FlatBuffers forbids creating an object while a vector is open)."""
    name_off = b.CreateString(name)
    inst_offs = [b.CreateString(s) for s in instance_selectors]
    g.ClassDeclStartInstanceSelectorsVector(b, len(inst_offs))
    for o in reversed(inst_offs):
        b.PrependUOffsetTRelative(o)
    inst_vec = b.EndVector()
    cls_offs = [b.CreateString(s) for s in class_selectors]
    g.ClassDeclStartClassSelectorsVector(b, len(cls_offs))
    for o in reversed(cls_offs):
        b.PrependUOffsetTRelative(o)
    cls_vec = b.EndVector()
    g.ClassDeclStart(b)
    g.ClassDeclAddName(b, name_off)
    g.ClassDeclAddInstanceSelectors(b, inst_vec)
    g.ClassDeclAddClassSelectors(b, cls_vec)
    return g.ClassDeclEnd(b)


def _encode_manifest_return(classes):
    """The reply to the host's spawn-time ``GetManifest`` (Phase 3): the classes this extension
    provides, each ``(name, instance_selectors, class_selectors)``. An empty list keeps a
    generic-handler extension backward-compatible (the host reads an absent vector as "no classes")."""
    b = flatbuffers.Builder(64)
    decl_offs = [_build_class_decl(b, name, inst, cls) for (name, inst, cls) in classes]
    g.ManifestReturnStartClassesVector(b, len(decl_offs))
    for o in reversed(decl_offs):
        b.PrependUOffsetTRelative(o)
    classes_vec = b.EndVector()
    g.ManifestReturnStart(b)
    g.ManifestReturnAddClasses(b, classes_vec)
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


def _decode_class_call(buf):
    """Decode a `Call` for extension-backed-class dispatch (Phase 3): the selector (`op`), the
    `class_name` it routes to, the receiver instance id (`recv`, 0 = class-side), the dropped-
    instance ids (`releases`), and the method arguments (a `DvList` in `data`)."""
    table = _msg(buf, g.Message.Call, "Call")
    call = g.Call()
    call.Init(table.Bytes, table.Pos)
    releases = [call.Releases(j) for j in range(call.ReleasesLength())]
    box = call.Data()
    data = _decode_dv(box) if box is not None else None
    return (_text(call.Op()), _text(call.ClassName()), call.Recv(), releases, data)


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
                    write_frame(conn, _encode_manifest_return([]))
                    continue
                op, arg, handles, resources, releases, arrays, data = _decode_call(frame)
                host = Host(conn, handles, resources, releases, arrays, data)
                write_frame(conn, _encode_reply(handler(host, op, arg)))
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
    holds). Ids start at 1, so ``recv == 0`` unambiguously means a class-side send."""

    def __init__(self):
        self._objects = {}
        self._next_id = 0

    def insert(self, obj):
        self._next_id += 1
        self._objects[self._next_id] = obj
        return self._next_id

    def get(self, oid):
        return self._objects.get(oid)

    def remove(self, oid):
        self._objects.pop(oid, None)


class Extension:
    """A class-providing extension (Phase 3). Register classes with :meth:`register`, then
    :meth:`serve`. The SDK owns the instances, so writing an extension class is just writing a plain
    Python class plus a selector -> method mapping."""

    def __init__(self):
        self._classes = {}  # name -> _ClassReg

    def register(self, name, cls, constructors=None, methods=None):
        """Register the Python class ``cls`` as the Quoin class ``name``. ``constructors`` maps
        class-side selectors to callables ``(*args) -> instance``; ``methods`` maps instance-side
        selectors to callables ``(instance, *args) -> value | instance``. Returns ``self`` for
        chaining."""
        self._classes[name] = _ClassReg(name, cls, constructors or {}, methods or {})
        return self

    def serve(self, path):
        """Bind a unix socket at ``path``, accept one host connection, and serve until it
        disconnects: answer the spawn-time ``GetManifest`` from the registered classes, and route
        each method ``Call`` to its handler — materializing returned instances into the table."""
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(path)
        server.listen(1)
        table = _ObjectTable()
        registered_types = tuple(reg.cls for reg in self._classes.values())
        try:
            conn, _ = server.accept()
            try:
                while True:
                    frame = read_frame(conn)
                    if frame is None:
                        break
                    if g.Envelope.GetRootAs(frame, 0).MsgType() == g.Message.GetManifest:
                        write_frame(conn, _encode_manifest_return(self._manifest()))
                        continue
                    write_frame(conn, self._dispatch(frame, table, registered_types))
            finally:
                conn.close()
        finally:
            server.close()

    def _manifest(self):
        """``(name, instance_selectors, class_selectors)`` for each registered class."""
        return [
            (reg.name, list(reg.methods.keys()), list(reg.constructors.keys()))
            for reg in self._classes.values()
        ]

    def _dispatch(self, frame, table, registered_types):
        """Route one method ``Call`` to its handler and return the terminal reply frame."""
        op, class_name, recv, releases, data = _decode_class_call(frame)
        # The host batches dropped instances onto `releases`; free them from the table.
        for rid in releases:
            table.remove(rid)
        reg = self._classes.get(class_name)
        if reg is None:
            raise ValueError(f"no extension-backed class '{class_name}'")
        # The host packs the method arguments as a `DvList` (decoded to a Python list).
        args = data if isinstance(data, list) else ([] if data is None else [data])
        if recv == 0:
            # Class-side: a constructor builds a new instance.
            ctor = reg.constructors.get(op)
            if ctor is None:
                raise ValueError(f"no constructor '{op}' on class '{class_name}'")
            return _encode_call_return_resource(table.insert(ctor(*args)))
        method = reg.methods.get(op)
        if method is None:
            raise ValueError(f"no method '{op}' on class '{class_name}'")
        instance = table.get(recv)
        if instance is None:
            raise ValueError(f"no live instance {recv}")
        result = method(instance, *args)
        # A returned registered instance becomes a new ext-side object; anything else is data.
        if isinstance(result, registered_types):
            return _encode_call_return_resource(table.insert(result))
        return _encode_reply(result)
