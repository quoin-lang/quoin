# Changelog

All notable changes to Quoin are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

Quoin is pre-1.0. Minor versions may make breaking language changes; each one is called out
under **Changed**, with the migration.

## [Unreleased]

### Added

- Extensions (experimental): the Rust SDK reaches resources-in-data parity with the Python
  SDK. A handler can return a structured `Value` tree carrying new live instances
  (`Value::instance`, e.g. a List of instances), register class-side selectors that return
  values rather than instances (`ClassBuilder::class_method`), and resolve live-instance
  references nested inside data arguments (`Host::instance`). No wire change — trees lower
  to the existing live-instance references (protocol v2, ext type 3) before encoding.

### Changed

- A `%'…'` interpolation literal is now lowered to string concatenation at compile time, so
  `%{…}` expressions see the full enclosing scope — including instance variables, which the
  old runtime recompilation silently read as nil (`%'%{@name}'` rendered empty). Methods
  containing interpolation literals are also no longer excluded from ahead-of-time
  compilation. Migration: a malformed `%{…}` in a literal is now a compile-time parse error
  instead of a runtime-catchable `ParseError`; sending `%` to a *computed* string keeps the
  reflective runtime path and its catchable `ParseError`.

### Fixed

- The reflective path (`%` sent to a computed string) now sees the caller's `self` too:
  `%{@ivar}`, `%{self}`, and `%{.send}` resolve against the calling method's receiver
  instead of silently reading nil — the interpolated unit compiles like `eval:self:`,
  without the top-level `self = nil` default that shadowed the caller's binding.

## [0.1.0] — 2026-07-12

The first release of Quoin: a small, dynamically-typed, object-oriented language in the
Smalltalk tradition — everything is an object, everything happens by sending messages, and
control flow is blocks responding to messages. It runs on a stack-based bytecode VM written in
Rust, with a tracing garbage collector and stackful coroutines.

`qn` is a single self-contained binary. The shipping standard library is compiled into it, so it
runs from any directory with nothing else installed.

### Language

- Objects, classes, and single inheritance, with instance variables (`@name`), class-side methods
  (`.meta`), and mixins.
- Blocks as first-class objects. `^` returns from the block; `^^` returns from the enclosing
  method.
- Declarations are strict: `var` for a mutable local, `let` for a binding. Assignment does not
  implicitly declare, and reading an unbound name raises `NameError` rather than yielding `nil`.
- Optional, gradual type annotations, checked by `qn check` and used by the optimizer. Nullable
  types (`Integer?`), generic collections (`List(Int)`), and block types.
- Literals for lists `#(1 2 3)`, maps `#{'a': 1}`, sets `#<1 2 3>`, symbols `#name`, and regular
  expressions. String interpolation is `%'total: %{a + b}'`. Comments start with `"`.
- Keyword-message selectors, including variadic ones.
- Errors are objects: typed `Error` subclasses, raised and caught by type, with multi-catch.
- `Class.exists?:#Name` asks whether a class is defined, without reading the name.
- `use` loads files explicitly — script-relative (`self:`), by glob, or by package.
- Fibers, generators, and lazy iteration; `^>` yields a value from a fiber.
- Placeholder statements for unfinished code: `...` raises `NotImplementedError`, `!!!` raises
  `UnreachableError`, and `???` warns and continues.

### Tooling

- `qn FILE` runs a program; `qn -e EXPR` evaluates one expression.
- `qn test [DIR]` runs the test suites in a directory, with `--coverage[=lcov|cobertura]` and
  `setup:`/`teardown:` and `setupAll:`/`teardownAll:` lifecycle hooks.
- `qn repl` — an interactive loop with editing, history, syntax highlighting, `$`-commands, and
  tab completion.
- `qn check` type-checks without running.
- `qn doc` generates API documentation for the current project — classes, methods, extensions,
  and commands — with `--check` to run every documented example and `--md` to render Markdown to
  HTML.
- `qn fmt` formats source. It re-parses its own output and refuses to write anything that would
  change the meaning of the program.
- `qn debug` — breakpoints, stepping, frame inspection, and evaluation in a frame, with
  `--break-on-throw` / `--break-on-uncaught`. `qn debug --dap` speaks the Debug Adapter Protocol,
  for editor integration.
- `qn highlight` prints syntax-highlighted source.

### Standard library

- Collections: `List`, `Map`, `Set`, `Bytes`, ranges, and a shared iteration protocol.
- Numbers: `Integer`, `Double`, `BigInteger`, `BigDecimal`, `Math`, `Statistics`.
- Time: `Instant`, `Duration`, `DateTime`, `Timestamp`, `TimeZone`, civil `Date` and `Time`, and
  `Span`.
- Data formats: `JSON`, `YAML`, `TOML`, `CSV`, `MessagePack`, `Base64`, `Hex`. A value's `asData`
  method controls how it serializes.
- Archives: `[Archive]Tar` and `[Archive]Zip`, read and write, with streaming gzip.
- Text: `String`, `Symbol`, `Regex`, and `Match` (named and positional capture groups).
- Cryptography: `[Crypto]Digest` (SHA-256/512/1, MD5, BLAKE3), `[Crypto]Hmac`, and
  `[Crypto]Random`.
- Identifiers: `UUID`, `ULID`.
- I/O: `[IO]File`, `[IO]Folder`, `[IO]Stdin`, and byte/string streams over files and sockets.
  Files are read *and* written: `[IO]File.create:` / `append:` return a buffered stream, with
  `[IO]File.write:to:` / `append:to:` / `read:` for the one-shot cases, plus `delete:`,
  `rename:to:`, `exists?:` and `[IO]Folder.create:` / `delete:`.
- OS: `[OS]Path` (lexical path manipulation), `[OS]Env` (read-only process environment), and
  `[OS]Process` for running subprocesses without a shell (`run:` / `start:`).
- Terminal: `Term` renders inline `[red bold]…[/]` markup to ANSI (stripping it when stdout is not
  a terminal), and `Log` provides leveled logging with lazy message blocks.
- Networking: `TcpSocket`, `TlsSocket`, `TcpListener`, `DNS` (the system resolver), an `[HTTP]`
  client, `[HTTP]Server`, and a `WebSocket` client.
- The `[Web]` framework: routing, requests and responses, and a worker pool.
- Concurrency: `Task`, `Async` (`sleep:`, `timeout:do:`, `gather:`), CSP `Channel`s, worker
  isolates, and a compute-offload pool for CPU-bound native work.
- Metaprogramming: `[Lang]Parser` and `[Lang]Node` expose the parser and AST as Quoin objects;
  `[Lang]Rewrite` makes span-precise source edits.

I/O is asynchronous and cooperative: a read or a write parks the task, it does not block the
scheduler.

File writes are **buffered** (16 KiB) and reach the disk on `flush!`, on `close`, or when the
program ends. Socket writes are **not** buffered, because a server writes a response and then
waits for the client; `flush!` is a no-op there, so the same code works over both.

### Extensions (experimental)

An out-of-process extension mechanism exists and is used internally, but is **not** a supported,
installable surface in v0.1 — the SDK crates are unpublished and the packaging and install story
lands post-v0.1.

- Extensions run out-of-process and speak a MessagePack wire protocol over a unix socket, so a
  crash or a hang in an extension cannot take the VM with it.
- SDKs for Rust and Python, at parity. An extension can provide real Quoin classes, hold
  resources, and call back into the host mid-call.
- An extension is packaged as a folder with an `extension.toml` manifest, loaded with
  `use <name>:*`.
- `adbc` (SQLite and PostgreSQL, via Apache Arrow ADBC) and `numpy` ship in the source tree as
  in-tree examples, not distributable packages.

### Performance

- The typed subset is compiled to native code ahead of time. This is on by default;
  `QN_AOT=0` disables it, and the interpreter path is always available.
- Untyped code is compiled speculatively from observed types, guarded and deoptimized on
  mismatch.
- Inline caches, devirtualized arithmetic and collection operations, and generics-aware dispatch.
- Cross-language comparisons are tracked in `bench/CROSS.md`; the environment variables that
  tune or disable each tier are in `docs/internal/ENV_FLAGS.md`.

### Known limitations

- A buffered file write stream is flushed on `close`, on `flush!`, and when the program ends —
  but **not on signal death**, exactly as in C. `[IO]File.write:to:` avoids the question.
- The extension SDK crates (`quoin-ext`, `quoin-ext-proto`) are not published to crates.io, so a
  third-party extension must vendor them. File-descriptor passing and a WASM tier are designed
  but not built.
- The debugger pauses the whole VM: there is no per-task debugging, and no watchpoints.
- The language reference (`docs/language/`) does not yet cover the whole shipped surface.
