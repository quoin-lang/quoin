# quoin-ext

The extension-side SDK for writing **out-of-process [Quoin](https://github.com/quoin-lang/quoin) extensions** in Rust.

A Quoin extension is an ordinary program in its own process. The VM spawns it, connects
over a unix domain socket, and from then on method calls cross the socket as
length-prefixed MessagePack frames (protocol contract: `quoin-ext-proto/PROTOCOL.md`).
This crate is the thin client an extension links against — it is **not** linked into the
VM, and it needs no part of it. The process boundary is the point: an extension can crash,
leak, or link anything (database drivers, native math libraries) without ever taking the
VM down; a crash mid-call surfaces as a catchable Quoin error.

Two ways to write one:

- **Extension-backed classes** (the main event): your extension *provides real Quoin
  classes*. You register plain Rust types with selectors; the VM installs the classes at
  spawn from a manifest handshake, and `Vector.ofFloats: #(1 2 3)` in Quoin dispatches to
  your Rust method. The SDK owns the instances in an id-keyed object table; the host holds
  opaque ids and frees them automatically as its references are collected.
- **A generic handler** (`serve`): a bare `op`/`arg` request loop for service-shaped
  extensions, with a self-managed resource registry.

## Quick start: a class-providing extension

```rust,no_run
use quoin_ext::{DataValue, Extension, Value};

struct Greeter { name: String }

fn main() {
    // The VM passes the socket path to rendezvous on as the first argument.
    let path = std::env::args().nth(1).expect("usage: greeter <socket-path>");

    let mut ext = Extension::new();
    ext.class::<Greeter>("Greeter", |c| {
        // Class-side constructor: `Greeter named: 'World'` -> a live instance.
        c.constructor("named:", |_host, args| {
            match args.first().and_then(|a| a.data()) {
                Some(DataValue::Str(s)) if !s.is_empty() => Ok(Greeter { name: s.clone() }),
                _ => Err("a greeter needs a name".into()), // -> a catchable Quoin error
            }
        });
        // Instance method: any `Into<Reply>` return — a String, a DataValue/Value tree, …
        c.method("greet", |g, _host, _args| Ok(format!("Hello, {}!", g.name)));
        // A method that MAKES a new instance (of this or any registered class).
        c.makes("twin", |g, _host, _args| Ok(Greeter { name: g.name.clone() }));
        // A class-side selector returning a value rather than an instance.
        c.class_method("defaultName", |_host, _args| Ok("World"));
        // Resources-in-data: live instances nested inside a structured return.
        c.class_method("pair:", |_host, args| {
            let Some(DataValue::Str(s)) = args.first().and_then(|a| a.data()) else {
                return Err("pair: expects a name".into());
            };
            Ok(Value::List(vec![
                Value::instance(Greeter { name: s.clone() }),
                Value::instance(Greeter { name: format!("{s} II") }),
            ]))
        });
    });
    ext.serve(&path).expect("serve loop");
}
```

And the Quoin side:

```text
Extension.spawn:'path/to/greeter';       "* installs the manifest's classes as globals
var g = Greeter.named:'World';
g.greet.print;                           "* -> Hello, World!
g.twin.greet.print;                      "* -> Hello, World!
Greeter.defaultName.print;               "* -> World
(Greeter.pair:'Ada').each:{ |p| p.greet.print };
{ Greeter.named:'' }.catch:{ |e| e.message.print };   "* recoverable, ext stays alive
```

A complete, runnable version of this walkthrough lives in [`examples/greeter.rs`](examples/greeter.rs)
(build with `cargo build -p quoin-ext --example greeter`); its e2e-tested twin is the
`ext_vector` fixture (`tests/fixtures/ext_vector.rs`, driven by `tests/extension.rs`).

## Method arguments: `Arg`

Each handler receives its Quoin arguments positionally as `&[Arg]`:

- `arg.data()` — a structured [`DataValue`] (nil, booleans, integers, floats, strings,
  bytes, big integers, decimals, lists, maps). A `DataValue::Resource` leaf *inside* a
  list/map is a live-instance reference; resolve it with `host.instance::<T>(leaf)`.
- `arg.object::<T>()` — another of *this extension's* live instances, passed directly
  (`va dot: vb`). The receiver itself can't also appear as an argument (its `&mut` is
  exclusive for the call).
- `arg.handle()` — a host value, most commonly a **block**. Run it with
  `host.apply_block(block, &inputs)` — one round-trip for the whole batch.
- `arg.array()` — a bulk [`ArrowArray`] column (the data plane): one contiguous buffer,
  never exploded per element.

## Return values: `Reply` and `Value`

Handlers return any `Into<Reply>`:

- `String`/`&str` → a Quoin String.
- [`DataValue`] / [`Value`] → a structured value. `Value` is `DataValue` plus
  `Value::instance(obj)` leaves — **new live instances of registered classes, nested
  anywhere** (a `List` of instances, a `Map` containing them). Lowering is atomic: the
  whole tree is validated before any instance is registered, so an unregistered type is a
  recoverable error that leaks nothing.
- `Reply::Array(ArrowArray)` → a bulk column; `Reply::Handle(h)` → a live host value the
  extension holds (from `get_global`/`make_value`/`call_method`).
- A `makes`/`constructor` return → a new instance of the dispatching (or any registered)
  class.

Errors: every handler returns `HandlerResult<T>` — `Err` becomes a **catchable** Quoin
error carrying your message, and the extension keeps serving. The error also carries a
**cross-process stack blob**: the SDK renders your error's `source()` chain under a
dispatch-frame line (`in Vector#at: (instance 3): …`), interleaved with any Quoin
segments from blocks this call invoked, in unwind order. Quoin code reads it as
`ex.remoteStack`, and the default traceback printer shows it fenced (`--- in extension
---`) at the failing call — so an extension author sees both sides of the story with no
handler code at all. Reserve panics/exits for genuine bugs; the VM isolates those too,
but the connection dies with the process.

Re-entrancy: a block your handler invokes may call **back into this extension** — the
nested call is serviced while your `apply_block`/`invoke_block` waits (strictly LIFO on
the one socket), bounded by the host's nesting depth cap. One Rust-specific limit: a
nested call addressed to the *outer call's receiver* (or one of its instance arguments)
reports "no live instance" — they're taken out of the object table for the handler's
`&mut` — while different instances, class-side sends, and the whole generic surface work.

## Reaching back into the host

Mid-call, the `&mut Host` argument issues re-entrant host-ops (each a synchronous
round-trip the VM services while the caller is parked):

- `get_global("Array")` — resolve a host class/global to a handle.
- `make_value(dv)` / `read_handle(h)` — build any host value from a `DataValue` /
  project one back.
- `call_method(recv, "ofFloats:", &[arg])` — send a Quoin message to a handle.
- `invoke_block(block, &batches)` / `apply_block(block, &inputs)` — run a host block
  over a batch in one round-trip.
- `retain(h)` / `release(&[h])` — handles are call-local by default (swept when the call
  returns); retain one to hold it across calls.

`host.releases()` lists ext-side resource ids the host has dropped — class extensions are
reaped automatically; generic `serve` handlers with self-managed registries should free
them at the top of each call.

## Packaging

Hand-spawning a binary path is the development loop. To ship, an extension becomes a
folder the VM loads with `use name:*`:

```text
greeter/
  quoin.toml    # [extension] command = "bin/greeter"  + namespace = "Greet"
  bin/greeter       # the built binary (or command = "python3", args = ["main.py"])
  init.qn           # optional Quoin glue run after the classes install
```

Install a package with `qn pkg install <dir>` — it lands in `$QUOIN_HOME/packages/`
(default `~/.quoin/packages/`), a built-in search root, and any `[bin]` manifest entries
link into `$QUOIN_HOME/bin/` for your `PATH`. Ad-hoc folders also resolve from
`./quoin_packages/<name>/` or `$QUOIN_PATH`. A package's classes
always install **namespaced** (`[Greet]Greeter`) — packages cannot claim bare globals.
Design and details: `docs/internal/EXT_PACKAGING.md`.

[`examples/Quernfile.qn`](examples/Quernfile.qn) is a copyable recipe that generates the
package above with [quern](https://github.com/quoin-lang/quern), the Quoin task runner:
`quern` in that directory compiles the release binary and assembles `dist/greeter/`,
mtime-skipping both steps when nothing changed.

## Wire compatibility

The protocol (v2) is append-only MessagePack, version-checked at the spawn-time manifest
handshake; `quoin-ext-proto` is dependency-free and `PROTOCOL.md` is the contract. A
Python SDK with the same surface lives at `sdk/python/quoin_ext` — the VM cannot tell
Rust and Python extensions apart. This crate is currently consumed as an in-tree path
dependency; publishing to crates.io is planned.
