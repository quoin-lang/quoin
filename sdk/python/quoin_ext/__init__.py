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
import os
import socket
import struct

import flatbuffers

try:
    # Packed-DataValue support: structured payloads as one MessagePack blob per value instead of
    # nested DataValueBox tables (negotiated per connection; see `_packed_available`). msgpack is
    # a C extension, so a packed payload costs one codec pass instead of ~2.5us per node of
    # pure-python flatbuffers table machinery (profiling/wire-encoding/notes.md).
    import msgpack
except ImportError:  # the SDK stays fully functional on the boxed-tree path
    msgpack = None

from . import ext_generated as g

_I64_MIN, _I64_MAX = -(2**63), 2**63 - 1


def _packed_available():
    """True if this process can speak PACKED DataValue payloads: the `msgpack` package imports
    (it is optional — without it the boxed-tree path is used) and packing isn't disabled via
    `QUOIN_EXT_NO_MSGPACK` (a test hook for exercising the fallback)."""
    return msgpack is not None and not os.environ.get("QUOIN_EXT_NO_MSGPACK")


def _pack_default(o):
    """`msgpack.packb` hook for the two non-native DataValue kinds (the wire contract in
    `schema/ext.fbs`): a >64-bit int -> ext type 1 (ASCII digits); Decimal -> ext type 2."""
    if isinstance(o, decimal.Decimal):
        return msgpack.ExtType(2, str(o).encode())
    if isinstance(o, int):
        return msgpack.ExtType(1, str(o).encode())
    raise TypeError(f"cannot serialize {type(o).__name__} as a structured value")


def _ext_hook(code, data):
    if code == 1:
        return int(data.decode())
    if code == 2:
        return decimal.Decimal(data.decode())
    raise ValueError(f"extension: unknown packed DataValue ext type {code}")


def _pack_dv(obj):
    return msgpack.packb(obj, use_bin_type=True, default=_pack_default)


def _unpack_dv(b):
    return msgpack.unpackb(b, raw=False, strict_map_key=False, ext_hook=_ext_hook)


def _read_get_manifest_packed(frame):
    """The `packed_ok` capability the host advertised in its spawn-time `GetManifest`."""
    env = g.Envelope.GetRootAs(frame, 0)
    gm = g.GetManifest()
    t = env.Msg()
    gm.Init(t.Bytes, t.Pos)
    return bool(gm.PackedOk())


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


def _byte_vector(tbl, slot):
    """A `[ubyte]` field as ONE buffer slice. Never read byte vectors through the per-element
    accessor (`x.V(i)` / `a.Data(k)`): each call is ~1µs of pure-python flatbuffers machinery,
    which turned a 32 KB Array argument into 31ms. `slot` mirrors the field's generated
    accessor (`DvBytes.V` -> 4, `ArrowArray.Data` -> 8)."""
    t = tbl._tab
    o = flatbuffers.number_types.UOffsetTFlags.py_type(t.Offset(slot))
    if o == 0:
        return b""
    start = t.Vector(o)
    return bytes(t.Bytes[start : start + t.VectorLen(o)])


# ---------------------------------------------------------------------------------------------
# Hand-rolled FlatBuffers hot paths
#
# Going through the generated pure-python bindings costs ~50 wrapped accessor calls (~0.5-1us
# each) per MINIMAL frame — roughly a third of the whole per-call budget in each direction
# (profiling/wire-encoding/notes.md). Every inbound frame after the manifest is a `Call`, and
# nearly every reply is one of three fixed shapes, so those paths read/write the FlatBuffers
# bytes directly; everything else stays on the generated code. The vtable slots mirror
# `ext_generated.py` (Call.Op -> 4 ... Call.DataPacked -> 24); the end-to-end extension tests
# cover every shape against the host's planus reader.
# ---------------------------------------------------------------------------------------------

_U16 = struct.Struct("<H")
_I32 = struct.Struct("<i")
_U32 = struct.Struct("<I")
_U64 = struct.Struct("<Q")


def _fb_field(buf, tpos, slot):
    """Absolute position of the field at vtable `slot` of the table at `tpos` (0 = absent)."""
    vt = tpos - _I32.unpack_from(buf, tpos)[0]
    if slot >= _U16.unpack_from(buf, vt)[0]:
        return 0
    off = _U16.unpack_from(buf, vt + slot)[0]
    return tpos + off if off else 0


def _fb_indirect(buf, pos):
    """Follow the uoffset at `pos` (a string/vector/table field) to its target position."""
    return pos + _U32.unpack_from(buf, pos)[0]


def _fb_str(buf, pos, default=""):
    if pos == 0:
        return default
    p = _fb_indirect(buf, pos)
    n = _U32.unpack_from(buf, p)[0]
    return buf[p + 4 : p + 4 + n].decode("utf-8")


def _fb_bytes(buf, pos):
    if pos == 0:
        return b""
    p = _fb_indirect(buf, pos)
    n = _U32.unpack_from(buf, p)[0]
    return bytes(buf[p + 4 : p + 4 + n])


def _fb_u64_vec(buf, pos):
    if pos == 0:
        return []
    p = _fb_indirect(buf, pos)
    n = _U32.unpack_from(buf, p)[0]
    return list(struct.unpack_from(f"<{n}Q", buf, p + 4))


def _fb_tables(buf, pos):
    """The table positions of a `[Table]` vector field at `pos`."""
    if pos == 0:
        return []
    p = _fb_indirect(buf, pos)
    n = _U32.unpack_from(buf, p)[0]
    return [_fb_indirect(buf, p + 4 + 4 * j) for j in range(n)]


def _fb_msg_type(frame):
    """The `Envelope.msg` union type of a frame (g.Message.*), without generated-code cost."""
    epos = _U32.unpack_from(frame, 0)[0]
    tf = _fb_field(frame, epos, 4)
    return frame[tf] if tf else g.Message.NONE


def _fb_root_call(frame):
    """The `Call` table position of `frame`, or `None` when the frame isn't a Call envelope."""
    epos = _U32.unpack_from(frame, 0)[0]
    tf = _fb_field(frame, epos, 4)
    if tf == 0 or frame[tf] != g.Message.Call:
        return None
    return _fb_indirect(frame, _fb_field(frame, epos, 6))


def _fb_boxed_dv(buf, pos):
    """Decode the boxed `DataValueBox` at field `pos` via the generated walker (the negotiated
    fallback path — packed peers never hit this)."""
    box = g.DataValueBox()
    box.Init(buf, _fb_indirect(buf, pos))
    return _decode_dv(box)


# Reply frames, laid out by hand (see the layout notes on each). All offsets are u32-aligned;
# vtables sit after their tables (negative soffsets), payload bytes last.

# [root=4][Envelope 4..16: soffset -12, type u8 @8.. wait see below][EV 16..24][inner 24..][...]
# Envelope table: [soffset i32][msg_type u8 + 3 pad][msg uoffset] -> 12 bytes, vtable (8,12,4,8).
_FAST_ENV = _U32.pack(4) + _I32.pack(-12)  # root uoffset; Envelope soffset (vtable at 16)
_FAST_EV = struct.pack("<4H", 8, 12, 4, 8)


def _fast_str_reply(msg_type, s):
    """`CallReturn { result }` / `CallReturnError { message }`: one string field at slot 4.
    Layout: [root:0..4][E:4..16][EV:16..24][C:24..32][CV:32..38][pad:2][str:40..]."""
    sb = s.encode("utf-8")
    return (
        _FAST_ENV
        + bytes((msg_type,))
        + b"\x00\x00\x00"
        + _U32.pack(12)  # msg uoffset @12 -> C at 24
        + _FAST_EV
        + _I32.pack(-8)  # C soffset (vtable at 32)
        + _U32.pack(12)  # result uoffset @28 -> str at 40
        + struct.pack("<3H", 6, 8, 4)
        + b"\x00\x00"
        + _U32.pack(len(sb))
        + sb
        + b"\x00"
    )


def _fast_packed_reply(payload):
    """`CallReturnData { packed }`: one [ubyte] field at slot 6 (slot 4 = the boxed form, absent).
    Layout: [root][E][EV][C:24..32][CV:32..40][vec:40..]."""
    return (
        _FAST_ENV
        + bytes((g.Message.CallReturnData,))
        + b"\x00\x00\x00"
        + _U32.pack(12)
        + _FAST_EV
        + _I32.pack(-8)  # C soffset (vtable at 32)
        + _U32.pack(12)  # packed uoffset @28 -> vec at 40
        + struct.pack("<4H", 8, 8, 0, 4)  # vt_len, tbl_len, value(4)=absent, packed(6)=@4
        + _U32.pack(len(payload))
        + payload
    )


def _fast_resource_reply(resource_id, class_name):
    """`CallReturnResource { resource u64 @4, class_name str @6 }`. The u64 sits at C+8 (an
    8-aligned buffer position); an empty class_name is omitted, like the generated encoder.
    Layout: [root][E][EV][C:24..40][CV:40..48][str:48..]."""
    if class_name:
        sb = class_name.encode("utf-8")
        return (
            _FAST_ENV
            + bytes((g.Message.CallReturnResource,))
            + b"\x00\x00\x00"
            + _U32.pack(12)
            + _FAST_EV
            + _I32.pack(-16)  # C soffset (vtable at 40)
            + _U32.pack(20)  # class_name uoffset @28 -> str at 48
            + _U64.pack(resource_id)  # @32, 8-aligned
            + struct.pack("<4H", 8, 16, 8, 4)
            + _U32.pack(len(sb))
            + sb
            + b"\x00"
        )
    return (
        _FAST_ENV
        + bytes((g.Message.CallReturnResource,))
        + b"\x00\x00\x00"
        + _U32.pack(12)
        + _FAST_EV
        + _I32.pack(-16)  # C soffset (vtable at 40)
        + b"\x00\x00\x00\x00"  # pad so the u64 lands 8-aligned at C+8
        + _U64.pack(resource_id)
        + struct.pack("<4H", 8, 16, 8, 0)
    )


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
        return _byte_vector(as_table(g.DvBytes), 4)
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


def _encode_make_value(obj, packed=False):
    b = flatbuffers.Builder(64)
    if packed:
        v = b.CreateByteVector(_pack_dv(obj))
        g.MakeValueStart(b)
        g.MakeValueAddPacked(b, v)
        return _envelope(b, g.Message.MakeValue, g.MakeValueEnd(b))
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
    return _fast_str_reply(g.Message.CallReturn, result)


def _encode_call_return_error(message):
    """A call failed recoverably: the host raises a catchable Quoin error and the extension keeps
    running. A terminal frame, like the other ``CallReturn*`` replies."""
    return _fast_str_reply(g.Message.CallReturnError, message)


def _encode_call_return_resource(resource_id, class_name=""):
    # `class_name` (Phase 3) names the registered class the resource is an instance of, so a method
    # can return an instance of any of the extension's classes (cross-class returns); "" = ExtResource.
    return _fast_resource_reply(resource_id, class_name)


def _encode_call_return_array(array):
    b = flatbuffers.Builder(64)
    a = _build_arrow(b, array)
    g.CallReturnArrayStart(b)
    g.CallReturnArrayAddArray(b, a)
    return _envelope(b, g.Message.CallReturnArray, g.CallReturnArrayEnd(b))


def _encode_call_return_data(obj, packed=False):
    if packed:
        return _fast_packed_reply(_pack_dv(obj))
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
    g.ManifestReturnAddPackedOk(b, _packed_available())
    return _envelope(b, g.Message.ManifestReturn, g.ManifestReturnEnd(b))


def _encode_reply(reply, packed=False):
    if isinstance(reply, Resource):
        return _encode_call_return_resource(reply.id)
    if isinstance(reply, ReturnHandle):
        return _encode_call_return_handle(reply.handle)
    if isinstance(reply, ArrowArray):
        return _encode_call_return_array(reply)
    if isinstance(reply, str):
        return _encode_call_return(reply)
    # Anything else (None / bool / int / float / Decimal / bytes / list / dict) is a structured value.
    return _encode_call_return_data(reply, packed)


# decoders -----------------------------------------------------------------------------------


def _msg(buf, expected, name):
    env = g.Envelope.GetRootAs(buf, 0)
    if env.MsgType() != expected:
        raise ValueError(f"extension: expected {name}, got msg type {env.MsgType()}")
    return env.Msg()


def _decode_call(buf):
    c = _fb_root_call(buf)
    if c is None:
        raise ValueError(f"extension: expected Call, got msg type {_fb_msg_type(buf)}")
    arrays = []
    for a in _fb_tables(buf, _fb_field(buf, c, 14)):
        dt = _fb_field(buf, a, 4)
        arrays.append(ArrowArray(buf[dt] if dt else 0, _fb_bytes(buf, _fb_field(buf, a, 8))))
    pb = _fb_field(buf, c, 24)  # data_packed
    df = _fb_field(buf, c, 16)  # boxed data (the fallback representation)
    if pb:
        data = _unpack_dv(_fb_bytes(buf, pb))
    elif df:
        data = _fb_boxed_dv(buf, df)
    else:
        data = None
    return (
        _fb_str(buf, _fb_field(buf, c, 4)),
        _fb_str(buf, _fb_field(buf, c, 6)),
        _fb_u64_vec(buf, _fb_field(buf, c, 8)),
        _fb_u64_vec(buf, _fb_field(buf, c, 10)),
        _fb_u64_vec(buf, _fb_field(buf, c, 12)),
        arrays,
        data,
    )


def _decode_class_call(buf):
    """Decode a `Call` for extension-backed-class dispatch (Phase 3): the selector (`op`), the
    `class_name` it routes to, the receiver instance id (`recv`, 0 = class-side), the dropped-
    instance ids (`releases`), and the ordered, tagged method arguments (`method_args`)."""
    table = _msg(buf, g.Message.Call, "Call")
    call = g.Call()
    call.Init(table.Bytes, table.Pos)
    releases = [call.Releases(j) for j in range(call.ReleasesLength())]
    args = []
    for j in range(call.MethodArgsLength()):
        a = call.MethodArgs(j)
        kind = a.Kind()
        if kind == g.ArgKind.Data:
            pb = _byte_vector(a, 10)  # Arg.packed
            if pb:
                args.append(("data", _unpack_dv(pb)))
            else:
                box = a.Data()
                args.append(("data", _decode_dv(box) if box is not None else None))
        elif kind == g.ArgKind.Resource:
            args.append(("resource", a.Id()))
        else:  # Handle
            args.append(("handle", a.Id()))
    return (_text(call.Op()), _text(call.ClassName()), call.Recv(), releases, args)


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

    def __init__(self, conn, handles, resources, releases, arrays, data, packed=False):
        self._conn = conn
        self._handles = handles
        self._resources = resources
        self._releases = releases
        self._arrays = arrays
        self._data = data
        self._packed = packed

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
        handle, _ = self._host_op(_encode_get_global(name))
        return handle

    def make_value(self, obj):
        """Construct any host value from a native Python value, returning a handle to it (for
        building non-string method arguments). The general form of `make_string`."""
        handle, _ = self._host_op(_encode_make_value(obj, self._packed))
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
        pb = _byte_vector(r, 8)  # ReadHandleReturn.packed
        if pb:
            return _unpack_dv(pb)
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
        # Packed-DataValue negotiation: send packed only if the host advertised it AND msgpack
        # is available here (the host always accepts both representations).
        packed = False
        try:
            while True:
                frame = read_frame(conn)
                if frame is None:
                    break
                # Phase 3: the host asks for a class manifest once, right after connect. A
                # generic-handler extension provides none; everything else is a Call.
                if _fb_msg_type(frame) == g.Message.GetManifest:
                    packed = _read_get_manifest_packed(frame) and _packed_available()
                    write_frame(conn, _encode_manifest_return([]))
                    continue
                op, arg, handles, resources, releases, arrays, data = _decode_call(frame)
                host = Host(conn, handles, resources, releases, arrays, data, packed)
                # A handler exception becomes a catchable Quoin error; the extension keeps serving.
                try:
                    reply = _encode_reply(handler(host, op, arg), packed)
                except Exception as exc:  # noqa: BLE001 — any handler error maps to a catchable error
                    reply = _encode_call_return_error(str(exc))
                write_frame(conn, reply)
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


class _HostBlock:
    """A host block passed as a method argument (Phase 3). Call it with one value to apply the block
    to that value over the socket, returning the result as a native Python value — so a handler can
    treat it like an ordinary function (e.g. ``[block(x) for x in self.data]``)."""

    def __init__(self, conn, handle, packed=False):
        self._host = Host(conn, [], [], [], [], None, packed)
        self._handle = handle

    def __call__(self, value):
        return self._host.apply_block(self._handle, [value])[0]


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
            # Packed-DataValue negotiation, exactly as in the generic `serve`.
            packed = False
            try:
                while True:
                    frame = read_frame(conn)
                    if frame is None:
                        break
                    if _fb_msg_type(frame) == g.Message.GetManifest:
                        packed = _read_get_manifest_packed(frame) and _packed_available()
                        write_frame(conn, _encode_manifest_return(self._manifest()))
                        continue
                    write_frame(
                        conn, self._dispatch(conn, frame, table, registered_types, packed)
                    )
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

    def _class_name_of(self, obj):
        """The registered Quoin class name for an instance (so a method returning an instance of any
        registered class is wrapped correctly — cross-class returns), or '' if it isn't registered."""
        for reg in self._classes.values():
            if isinstance(obj, reg.cls):
                return reg.name
        return ""

    def _resolve_args(self, raw_args, table, conn, packed=False):
        """Resolve the tagged wire args to native Python values: data passes through, an ext-instance
        id becomes the live instance, and a handle becomes a callable :class:`_HostBlock`. Order is
        preserved, so the handler receives its arguments positionally."""
        out = []
        for kind, val in raw_args:
            if kind == "data":
                out.append(val)
            elif kind == "resource":
                obj = table.get(val)
                if obj is None:
                    raise ValueError(f"argument references no live instance {val}")
                out.append(obj)
            else:  # handle
                out.append(_HostBlock(conn, val, packed))
        return out

    def _dispatch(self, conn, frame, table, registered_types, packed=False):
        """Route one method ``Call`` to its handler and return the terminal reply frame."""
        op, class_name, recv, releases, raw_args = _decode_class_call(frame)
        # The host batches dropped instances onto `releases`; free them from the table.
        for rid in releases:
            table.remove(rid)
        reg = self._classes.get(class_name)
        if reg is None:
            raise ValueError(f"no extension-backed class '{class_name}'")
        args = self._resolve_args(raw_args, table, conn, packed)
        if recv == 0:
            # Class-side: a constructor builds a new instance.
            ctor = reg.constructors.get(op)
            if ctor is None:
                raise ValueError(f"no constructor '{op}' on class '{class_name}'")
            # A handler exception is a *recoverable* error: send it as a `CallReturnError` so the
            # host raises a catchable Quoin error and this extension keeps serving (unlike the
            # routing failures above, which are protocol bugs and propagate).
            try:
                obj = ctor(*args)
            except Exception as exc:  # noqa: BLE001 — any handler error maps to a catchable error
                return _encode_call_return_error(str(exc))
            return _encode_call_return_resource(table.insert(obj), self._class_name_of(obj))
        method = reg.methods.get(op)
        if method is None:
            raise ValueError(f"no method '{op}' on class '{class_name}'")
        instance = table.get(recv)
        if instance is None:
            raise ValueError(f"no live instance {recv}")
        try:
            result = method(instance, *args)
        except Exception as exc:  # noqa: BLE001 — any handler error maps to a catchable error
            return _encode_call_return_error(str(exc))
        # A returned registered instance becomes a new ext-side object; anything else is data.
        if isinstance(result, registered_types):
            return _encode_call_return_resource(table.insert(result), self._class_name_of(result))
        return _encode_reply(result, packed)
