# quoin-ext (Python SDK)

The Python client for writing out-of-process Quoin extensions (Tier 1; see
`docs/FUTURE_EXT_ARCH.md`). It speaks the same `ext.fbs` FlatBuffers wire protocol as the
Rust `quoin-ext` crate, so a Quoin program spawns and talks to a Python extension exactly
as it would a Rust one — this is the polyglot payoff of the out-of-process design.

## Using it

```python
import quoin_ext

def handler(op, arg):       # op, arg are str; return a str
    return arg.upper() if op == "upper" else arg

quoin_ext.serve(sys.argv[1], handler)   # argv[1] is the socket path the VM passes
```

See `examples/ext_echo.py`. Runtime dependency: the pure-Python `flatbuffers` package
(`pip install flatbuffers`, or `pip install -r requirements.txt`). No `flatc` needed to
*use* the SDK.

## Scope

Slice 7 is the **scalar `Call` → `CallReturn`** path — the polyglot transport proof. The
richer surface the Rust SDK already has (handle / resource / `Array` arguments, re-entrant
host-ops, batched callbacks) is not yet wired in Python; it grows from the same generated
bindings.

## Regenerating the bindings

`quoin_ext/ext_generated.py` is generated from the shared schema with `flatc` (checked in,
like the Rust side's planus output). After editing `crates/quoin-ext-proto/schema/ext.fbs`:

```sh
flatc --python --gen-onefile -o sdk/python/quoin_ext crates/quoin-ext-proto/schema/ext.fbs
```

`flatc` (e.g. `brew install flatbuffers`) is only needed for regeneration, not to use the SDK.
