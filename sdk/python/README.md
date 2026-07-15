# quoin-ext (Python SDK)

The Python client for writing out-of-process Quoin extensions (Tier 1; see
`docs/internal/FUTURE_EXT_ARCH.md`). It speaks the same MessagePack wire protocol
(`crates/quoin-ext-proto/PROTOCOL.md`) as the Rust `quoin-ext` crate, so a Quoin program
spawns and talks to a Python extension exactly as it would a Rust one — this is the
polyglot payoff of the out-of-process design.

## Using it

```python
import quoin_ext

def handler(host, op, arg):   # op, arg are str
    return arg.upper() if op == "upper" else arg

quoin_ext.serve(sys.argv[1], handler)   # argv[1] is the socket path the VM passes
```

See `examples/ext_echo.py`. Runtime dependency: the `msgpack` package
(`pip install msgpack`, or `pip install -r requirements.txt`) — the whole wire is
MessagePack, one `packb`/`unpackb` per frame.

## Scope

At parity with the Rust SDK: the generic `call:with:` surface (handle / resource / `Array`
arguments, structured `data:` payloads, re-entrant host-ops incl. batched block callbacks)
plus extension-backed classes (Phase 3) via `quoin_ext.Extension` — register a plain Python
class with selector→callable tables and the host installs it as a real Quoin class. The
in-repo `quoin_packages/numpy` extension is the flagship consumer.

## Lanes: serving calls concurrently

`quoin_ext.Extension(lanes=n)` (1–1024, default 1) declares that this extension serves up
to `n` connections concurrently — a lane-aware host opens that many and issues calls on
all of them, each served on its own thread over the shared instance table. The host still
serializes the calls *to any one instance*; the concurrency is across instances. Worth
declaring when your handlers block with the GIL released — database drivers, sockets,
files, native kernels — where threads genuinely overlap; pure-Python compute gains nothing
and should stay at 1. Old hosts ignore the declaration and open one connection.

## The wire

There are no generated bindings and no codegen step: a frame is a length-prefixed
MessagePack array, built and parsed by this package directly. The byte-level contract —
message table, value mapping (BigInt = ext type 1, Decimal = ext type 2), framing, and the
append-only evolution rules — is `crates/quoin-ext-proto/PROTOCOL.md`.
