# The Quoin extension wire protocol (version 2)

The byte-level contract between the Quoin VM (the **host**) and an out-of-process
extension, in either implementation language. The reference codecs are
`src/codec.rs` (Rust, hand-rolled, dependency-free) and the Python SDK
(`sdk/python/quoin_ext`, on the C `msgpack` package). Architecture rationale lives in
`docs/internal/FUTURE_EXT_ARCH.md`; this file is only the wire.

Version 1 was a FlatBuffers envelope with negotiated MessagePack payloads; it is retired.
Version 2 is MessagePack end to end: any language with a MessagePack library can
implement an SDK from this page.

## Transport and framing

- One unix-domain stream socket; the host spawns the extension process and passes the
  socket path as the final `argv` entry.
- One frame = a little-endian `u32` byte length, then that many bytes of payload.
- A declared length above **256 MiB** (`MAX_FRAME_LEN`) must be refused before allocating.
- The conversation is strict request/response — one frame in flight per direction. Bytes
  past the end of a frame are a desync and must be an error, not silently dropped.

## Frames

A frame payload is one MessagePack **array** `[type, field, ...]`. `type` is an unsigned
integer; the fields are positional, per the table below. `u64` fields are MessagePack
unsigned ints; `str`/`bin`/`bool`/arrays/maps are their native MessagePack forms;
"`x | nil`" marks an optional field carried as MessagePack nil when absent.

| type | message | direction | fields after the type tag |
|---|---|---|---|
| 0 | Call | host → ext | `op:str, arg:str, handles:[u64], resources:[u64], releases:[u64], arrays:[ArrowArray], data:Value|nil, class_name:str, recv:u64, method_args:[Arg]` |
| 1 | CallReturn | ext → host | `result:str, handler_micros:u64`† |
| 2 | CallReturnError | ext → host | `message:str, remote_stack:str, handler_micros:u64`† |
| 3 | CallReturnResource | ext → host | `resource:u64, class_name:str, handler_micros:u64`† |
| 4 | CallReturnArray | ext → host | `array:ArrowArray, handler_micros:u64`† |
| 5 | CallReturnData | ext → host | `value:Value, handler_micros:u64`† |
| 6 | CallReturnHandle | ext → host | `handle:u64, handler_micros:u64`† |
| 7 | GetManifest | host → ext | `version:u32` |
| 8 | ManifestReturn | ext → host | `version:u32, classes:[ClassDecl]` |
| 9 | MakeString | ext → host | `value:str` |
| 10 | HandleToString | ext → host | `handle:u64` |
| 11 | Retain | ext → host | `handle:u64` |
| 12 | Release | ext → host | `handles:[u64]` |
| 13 | CallMethodOnHandle | ext → host | `receiver:u64, selector:str, args:[u64]` |
| 14 | InvokeBlock | ext → host | `block:u64, batches:[[u64]]` |
| 15 | InvokeBlockReturn | host → ext | `results:[u64], error:str|nil, remote_stack:str` |
| 16 | GetGlobal | ext → host | `name:str` |
| 17 | MakeValue | ext → host | `value:Value` |
| 18 | ReadHandle | ext → host | `handle:u64` |
| 19 | ReadHandleReturn | host → ext | `value:Value, error:str|nil, remote_stack:str` |
| 20 | HostOpReturn | host → ext | `handle:u64, str:str|nil, error:str|nil, remote_stack:str` |
| 21 | CallReturnChannel | worker → host | `chan:u64, handler_micros:u64`† — Quoin worker peers only (a shipped channel endpoint, docs/internal/ACTOR_OBJECTS.md §6); extensions never produce or receive it |

† `handler_micros` is an appended field (see Evolution) on every `CallReturn*`
terminal: the wall time the peer spent servicing the call, in microseconds, from
decoding the `Call` to writing its terminal — nested host round-trips included.
Producers SHOULD send it (both SDKs do); a decoder treats an absent field as 0 =
not reported. The host's boundary profiling (`VM.boundaryStats`) uses it to split
a call's cost into queue-wait / transport / remote-handler shares. In the Rust
crate it travels OUT-OF-BAND of `Msg` as `ReplyMeta`
(`encode_with_meta`/`decode_frame_with_meta`), so message construction stays
meta-free. Note the append-only consequence: the first appended position on
these terminals is now claimed and **typed** — a frame carrying a non-uint there
is malformed (exactly as a non-str in `CallReturnError`'s `remote_stack` position
always was); further unknown extras after it are still skipped.

Composite fields (also MessagePack arrays):

- **ArrowArray** = `[dtype:u64, length:u64, data:bin]` — a bulk numeric column (the data
  plane): `dtype` 0 = float64, 1 = int64; `data` is the contiguous little-endian value
  buffer (Arrow non-nullable primitive layout); `length` is the element count (derivable
  from `data` for these fixed-width types, carried for Arrow-C-Data-Interface forward
  compat).
- **Arg** = `[kind:u64, payload]` — one ordered, tagged method argument for an
  extension-backed-class send (`Call.method_args`): kind 0 = **Data** (payload is a
  Value), 1 = **Resource** (payload is an ext-instance's object-table id, so a method can
  take another of the extension's objects), 2 = **Handle** (payload is a host-value
  handle for a block or other non-data host object the extension drives via
  InvokeBlock / CallMethodOnHandle), 3 = **Array** (payload is an inline ArrowArray —
  the data plane as a method argument).
- **ClassDecl** = `[name:str, instance_selectors:[str], class_selectors:[str]]` — one
  extension-provided class; the host installs a real Quoin class named `name` whose
  selectors dispatch over the socket (instance-side on instances, class-side on the
  class).

## Values

A **Value** is the wire form of a structured Quoin value (nested data, not live
objects):

- nil / bool / int64 / float64 / str / bin / array / map are native MessagePack. Map keys
  must be strings.
- **BigInt** is ext type **1**, payload = the ASCII decimal digits.
- **Decimal** is ext type **2**, payload = the ASCII decimal string.
- **Resource** is ext type **3**, payload = an 8-byte little-endian object-table id
  followed by the UTF-8 class name — a *live extension instance* inside a value.
  Host → ext, the class name is empty (the extension resolves the id in its own table);
  ext → host, it names the registered class so the host wraps the id as the right
  installed class (e.g. a method returning a list of instances). Resource references are
  only meaningful between the host and the one extension that owns the ids: the host
  refuses to send another extension's instance, and refuses resources entirely on the
  re-entrant host-op channels (MakeValue / ReadHandleReturn values are plain data).
- A uint64 above `i64::MAX` (marker `0xcf`) should be accepted as a BigInt, not rejected —
  a C-side packer may emit it.

Decoders must enforce a **64-level nesting-depth cap** on values from the peer (the host
must never stack-overflow on a buggy extension's payload — an uncatchable abort would
defeat out-of-process isolation), reject trailing bytes after a frame, and cap
pre-allocations by the remaining buffer size (a lying length prefix must not drive a
multi-GB allocation).

`Call.data` is `Value | nil`, where nil means "no data payload"; a Quoin nil inside a
data payload only occurs nested (the top-level `Some(Null)` collapses to nil — the two
are indistinguishable on every SDK surface).

## Conversation shape

1. On connect, the host sends `GetManifest` with its protocol version. The extension
   replies `ManifestReturn` with its own version and its provided classes (empty list if
   it only serves the generic `call:with:` surface). **Each side must refuse a version it
   does not speak with a clear error naming both versions** — this is the first exchange,
   so a mismatch is caught before any other frame is interpreted.
2. Thereafter the host sends `Call` frames. The extension answers each with exactly one
   terminal (`CallReturn`, `CallReturnError`, `CallReturnResource`, `CallReturnArray`,
   `CallReturnData`, `CallReturnHandle`), optionally preceded by any number of re-entrant
   host-ops (types 9–14, 16–18), each of which the host answers (`HostOpReturn`,
   `InvokeBlockReturn`, `ReadHandleReturn`) before reading on.
   **Nesting**: while the extension awaits a host-op reply, the host may send a nested
   `Call` — the host re-entering the extension from inside a block/method the extension
   is having it run. The extension services it (answering with its own terminal) before
   reading on for the pending reply; frames are strictly LIFO, a call stack over the
   socket. The host caps the nesting depth and refuses deeper re-entry with a catchable
   error, so mutual recursion cannot exhaust either process.
3. Two id spaces: **handles** are host-side value ids (call-local unless `Retain`ed;
   a block argument arrives in `Call.handles`); **resources** are ext-side object-table
   ids (the host holds them opaquely and returns dropped ones in the next `Call.releases`
   — the batched reap).

## Errors and the cross-process stack blob

Every error-bearing reply carries an appended `remote_stack: str` (empty = none): an
**opaque, human-oriented stack blob**. Producers SHOULD fill it with their language's
conventional stack rendering — a Python traceback, a Rust error chain under a synthesized
dispatch-frame line, the host's rendered Quoin frames — and, when the failure came from a
deeper layer (a failed host-op, a failed nested call), append that layer's blob below
their own segment, preserving **unwind order**. Consumers MUST NOT parse the blob: the
host displays it fenced in tracebacks and surfaces it to Quoin code as `ex.remoteStack`;
it caps the size on ingest and sanitizes control characters at display (untrusted foreign
text). An empty/omitted blob degrades to message-only behavior, so old peers interoperate
unchanged.

## Evolution

- Message fields are **append-only**: decoders read the fields they know and must
  **skip** any well-formed trailing fields (so are the composite arrays `ArrowArray`,
  `Arg`, `ClassDecl`). Never reorder or remove fields; never renumber frame types.
- Adding a frame type or appending fields does **not** bump the protocol version; an
  unknown frame type is a hard error (the handshake catches genuine mismatches).
- Bump `PROTOCOL_VERSION` only for changes an existing decoder cannot skip.
