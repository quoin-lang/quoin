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

Scope (Slice 7 skeleton): the scalar ``Call`` -> ``CallReturn`` path — the polyglot transport
proof. Handle / resource / array arguments, re-entrant host-ops, and bulk columns mirror the
Rust SDK and arrive in later slices.
"""

import socket
import struct

import flatbuffers

from . import ext_generated as g

__all__ = ["serve", "read_frame", "write_frame"]


def _recv_exact(conn, n):
    """Read exactly ``n`` bytes. ``None`` on a clean EOF at a frame boundary; raises if the
    peer closes mid-frame."""
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


def _decode_call(buf):
    """Decode a ``Call`` frame into ``(op, arg)``. (The skeleton ignores handle/resource/array
    args; they decode to empty here.)"""
    env = g.Envelope.GetRootAs(buf, 0)
    if env.MsgType() != g.Message.Call:
        raise ValueError(f"extension: expected a Call frame, got msg type {env.MsgType()}")
    table = env.Msg()
    call = g.Call()
    call.Init(table.Bytes, table.Pos)
    return (_text(call.Op()), _text(call.Arg()))


def _encode_call_return(result):
    """Encode an ``Envelope`` carrying a scalar ``CallReturn``."""
    builder = flatbuffers.Builder(64)
    result_off = builder.CreateString(result)
    g.CallReturnStart(builder)
    g.CallReturnAddResult(builder, result_off)
    cr = g.CallReturnEnd(builder)
    g.EnvelopeStart(builder)
    g.EnvelopeAddMsgType(builder, g.Message.CallReturn)
    g.EnvelopeAddMsg(builder, cr)
    env = g.EnvelopeEnd(builder)
    builder.Finish(env)
    return bytes(builder.Output())


def _text(b):
    """A FlatBuffers string accessor (``bytes`` or ``None``) as a ``str``."""
    return b.decode("utf-8") if b is not None else ""


def serve(path, handler):
    """Bind a unix socket at ``path``, accept one host connection, and serve scalar calls until
    the host disconnects. Each ``Call`` invokes ``handler(op, arg)`` (both ``str``); its return
    value (a ``str``) is sent back as a ``CallReturn``.

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
                op, arg = _decode_call(frame)
                write_frame(conn, _encode_call_return(handler(op, arg)))
        finally:
            conn.close()
    finally:
        server.close()
