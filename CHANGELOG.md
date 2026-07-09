# Changelog

All notable changes to Quoin are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

Quoin is pre-1.0. Minor versions may make breaking language changes; each one is called out
under **Changed**, with the migration.

## [0.1.0] — unreleased

<!-- Date this heading when the release is tagged. -->

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
  implicitly declare.
- Optional, gradual type annotations, checked by `qn check` and used by the optimizer. Nullable
  types (`Integer?`), generic collections (`List(Int)`), and block types.
- Literals for lists `#(1 2 3)`, maps `#{'a': 1}`, sets `#<1 2 3>`, symbols `#name`, and regular
  expressions. String interpolation is `%'total: %{a + b}'`. Comments start with `"`.
- Keyword-message selectors, including variadic ones.
- Errors are objects: typed `Error` subclasses, raised and caught by type, with multi-catch.
- `use` loads files explicitly — script-relative (`self:`), by glob, or by package.
- Fibers, generators, and lazy iteration; `^>` yields a value from a fiber.

### Tooling

- `qn FILE` runs a program; `qn -e EXPR` evaluates one expression.
- `qn test [DIR]` runs the test suites in a directory, with `--coverage[=lcov|cobertura]`.
- `qn repl` — an interactive loop with editing, history, syntax highlighting, `$`-commands, and
  tab completion.
- `qn check` type-checks without running.
- `qn fmt` formats source. It re-parses its own output and refuses to write anything that would
  change the meaning of the program.
- `qn debug` — breakpoints, stepping, frame inspection, and evaluation in a frame, with
  `--break-on-throw` / `--break-on-uncaught`. `qn debug --dap` speaks the Debug Adapter Protocol,
  for editor integration.
- `qn highlight` prints syntax-highlighted source.

### Standard library

- Collections: `List`, `Map`, `Set`, `Bytes`, ranges, and a shared iteration protocol.
- Numbers: `Integer`, `Double`, `BigInteger`, `BigDecimal`, `Math`, `Statistics`.
- Time: `Instant`, `Duration`, `DateTime`, `Timestamp`, `TimeZone`.
- Data formats: `JSON`, `YAML`, `TOML`, `CSV`, `MessagePack`, `Base64`, `Hex`.
- Text: `String`, `Symbol`, `Regex`.
- Identifiers: `UUID`, `ULID`.
- I/O: `[IO]File`, `[IO]Folder`, `[IO]Stdin`, and byte/string streams over files and sockets.
- OS: `[OS]Path` (lexical path manipulation), `[OS]Env` (read-only process environment).
- Networking: `TcpSocket`, `TlsSocket`, `TcpListener`, an `[HTTP]` client, and `[HTTP]Server`.
- The `[Web]` framework: routing, requests and responses, and a worker pool.
- Concurrency: `Task`, `Async` (`sleep:`, `timeout:do:`, `gather:`), CSP `Channel`s, worker
  isolates, and a compute-offload pool for CPU-bound native work.

I/O is asynchronous and cooperative: a read parks the task, it does not block the scheduler.

### Extensions

- Extensions run out-of-process and speak a MessagePack wire protocol over a unix socket, so a
  crash or a hang in an extension cannot take the VM with it.
- SDKs for Rust and Python, at parity. An extension can provide real Quoin classes, hold
  resources, and call back into the host mid-call.
- An extension is packaged as a folder with an `extension.toml` manifest, loaded with
  `use <name>:*`.
- Two ship with the source tree: `adbc` (SQLite and PostgreSQL, via Apache Arrow ADBC) and
  `numpy`.

### Performance

- The typed subset is compiled to native code ahead of time. This is on by default;
  `QN_AOT=0` disables it, and the interpreter path is always available.
- Untyped code is compiled speculatively from observed types, guarded and deoptimized on
  mismatch.
- Inline caches, devirtualized arithmetic and collection operations, and generics-aware dispatch.
- Cross-language comparisons are tracked in `bench/CROSS.md`; the environment variables that
  tune or disable each tier are in `docs/ENV_FLAGS.md`.

### Known limitations

- **Files are read-only.** `[IO]File` opens a file for reading and metadata; there is no API to
  create or write one. The only writable handles are stdout and stderr.
- The extension SDK crates (`quoin-ext`, `quoin-ext-proto`) are not published to crates.io, so a
  third-party extension must vendor them. File-descriptor passing and a WASM tier are designed
  but not built.
- The debugger pauses the whole VM: there is no per-task debugging, and no watchpoints.
- The language reference (`docs/language/`) does not yet cover the whole shipped surface.
