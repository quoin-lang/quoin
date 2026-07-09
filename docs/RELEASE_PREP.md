# Release prep — v0.1.0 "public preview"

Working checklist for bringing Quoin to a releasable state. Assessment date
2026-07-08, on `chore/release-prep` (stacked on `fix/bug-hunt`, PR #77).

## Goal: definition of done

A stranger with no context can:

1. **Install and run from anywhere** — `qn --version`, `qn --help`, `qn script.qn`,
   `qn repl` work outside the repo checkout.
2. **Trust the process boundary** — no known pure-Quoin program can kill the
   process uncatchably, and `qn script.qn` exits non-zero on failure.
3. **Read complete, runnable docs** — the language reference covers the shipped
   surface, and every example in it runs.
4. **Know their legal footing** — LICENSE + crate metadata.
5. **Get started from the README** — install, quickstart, CLI verb list, link to
   the reference.

Out of scope for v0.1 (document as experimental): the extension system as an
*installable* surface (`quoin_packages/adbc` points at a relative pre-built
binary; `numpy` assumes an ambient Python env), the Python SDK on PyPI,
LSP/VSCode tooling, Windows.

## Tier 1 — engineering blockers

- [x] **`.new` on native classes mints poison shells** — FIXED (survey + as-built
  record below): every builder-registered native class now either constructs for
  real, is `abstract!` (namespace classes + `Object`), or refuses `new`/`new:`
  with a typed `ClassError` naming its real constructors. Tests:
  `qnlib/tests/54-native-new.qn`.
- [x] **CWD-coupled stdlib loading.** FIXED — see the as-built record below. The
  shipping stdlib subset is compiled into the binary, `use self:…` resolves
  against the entry script's directory, and `qn test [DIR]` runs the *caller's*
  suites.
- [x] **No LICENSE / no `license` field in any Cargo.toml.** FIXED: dual
  MIT OR Apache-2.0, taken from the placeholder `quoin` crate that reserves the
  name on crates.io (`github.com/quoin-lang/quoin`, which this VM is destined to
  become). `LICENSE-MIT` + `LICENSE-APACHE` at the root, a `[workspace.package]`
  block every member inherits (`crates/adbc` repeats it — separate workspace),
  root `description`/`keywords`/`categories`/`readme`, and the lint crates'
  `authors = ["authors go here"]` placeholder removed. README has a License
  section. Verified: `cargo package --list` ships both licenses and the 142
  `qnlib/` files `build.rs` needs.
- [x] **CLI hygiene.** `-h/--help` and `-V/--version` exist; bare `qn` prints
  usage instead of running the dev scratch `qnlib/testscript.qn`; an unknown
  flag is an error (exit 2) rather than a filename.
- [x] Replace the hand-rolled `VmRunnerOptions::parse` with `clap` (QUOIN_TODO
  item). `--help`/`--version`/usage errors are answered by the parser.
  **One behavior change:** a hyphen-leading argument meant for the *program* now
  needs `--` (`qn app.qn -- --verbose`); previously any unrecognized flag was
  passed through. `--coverage` uses `require_equals` so a bare `--coverage` can
  never swallow the next positional as its format.
- [x] **`qn <file>` always exits 0.** FIXED: the mode drivers gate on a
  `UnitOutcome` (`src/runner.rs`) — an uncaught error exits 1 (a falsy final
  value does not), and `Runtime.exit:` / `Runtime.exit` request a specific
  status. The exit request is uncatchable (`QuoinError::ExitRequested`, modeled
  on `Cancelled`): `finally` blocks run, `catch:` can't swallow it, and a
  `requested_exit` flag on `VmState` makes it process-wide even from a spawned
  task; the runner exits only after the arena drops, so extension/socket
  `Drop`s run. Works in run/test/eval/REPL modes. Tests: `tests/exit_code.rs`.
- [x] **Uncatchable SIGBUS: cyclic/deep serialization.** FIXED:
  `MAX_SERIALIZE_DEPTH = 128` threaded through `value_to_data` and `value_to_json`
  (`src/runtime/data_value.rs`), so every serializer refuses a cyclic or enormous
  value with a catchable `ValueError` instead of aborting the process. 128 is
  `serde_json`'s own parse limit, so nothing that round-tripped before regressed.
  `write_dv` stays infallible — the producer cap guards it. Tests:
  `qnlib/tests/55-serialize-depth.qn`.
- [x] **Uncatchable SIGBUS: `execute_block` native re-entry** (an `each:` body
  re-iterating its receiver; a self-nesting `catch:`). FIXED: `execute_block`
  measures the *remaining coroutine stack* (`ensure_stack_headroom`, 2 MiB margin
  of 16 MiB) rather than capping depth, so deep-but-finite generator pipelines keep
  working. Raises the new typed `StackError`, which the `MAX_NATIVE_REENTRY` error
  now uses too (it was a bare String — uncatchable by `catch:{|e:Error|}`).
  Measured free: `profiling/execute-block-watermark/notes.md`. Tests:
  `qnlib/tests/56-stack-reentry.qn`.
- [x] **F11** (`#(-1 -2)` parsed as `#(-3)`). FIXED — and it was never only about
  minus: `+`, `-` and `%` are all both prefix and infix, so `#(1 +2)` was `#(3)`
  and `#( 'a' %'b' )` was one element. Inside a collection literal such an
  operator is a prefix starting a new element exactly when it is detached from
  its left operand and glued to its right one; every other spacing is infix, so
  `#(5-3)` is still `#(2)`. Done in `parse_literal_elements`, not the grammar
  (pest has no lookbehind). See BUGS.md Finding 11.

## Tier 2 — language reference (`docs/language/`)

The structure (Parts I–VI + appendices, Rules-box format) is right; coverage
stops at ~40% of the shipped language and most multi-line examples predate
strict `var`/`let` and no longer compile.

- [ ] **Fix stale examples** (~18 snippets across `02`–`06` use implicit
  declaration `x = 5`). Small, mechanical, do first.
- [ ] **Doc-example harness**: extract fenced blocks from `docs/language/` and
  run them under `qn` in CI, so examples can't rot again.
- [ ] **Async/concurrency section** (Part V is *titled* Concurrency and covers
  none of it): `Task`, `Async` (gather/timeout/sleep), `Channel`, `Parallel`,
  `Plan`. Large.
- [ ] **Networking/web section**: sockets, streams, `TcpServer`, `[HTTP]`
  client/server, `[Web]App`. Large.
- [ ] **Type-system section**: nullable `Integer?` + narrowing, generics
  `List(Integer)`/`.elementType`, `Block(args ^Ret)`, `^Ret` header form, the
  gradual checker / `qn check`. Large.
- [ ] **Tooling section** (new): CLI verbs, REPL, debugger + DAP, `qn fmt`,
  coverage. Medium.
- [ ] **Data formats** (JSON/MessagePack/CSV/TOML/YAML), **numbers**
  (Math/BigInteger/BigDecimal/Statistics), **time** (Duration/Instant/
  Timestamp/DateTime/TimeZone), **extension usage**. Medium each.
- [ ] **Small**: Bytes + codecs (base64/hex/gz/zstd), UUID/ULID, refresh stdlib
  map §22 (stops at `06-io`; core runs `00`–`11` + `net/` + `web/`), fold
  Array/Timer/VM/streams into §18, strip internal pointers (BUGS.md/QUOIN_TODO
  references).
- [ ] **README**: install/quickstart, full verb list (lists 4 of ~10), license
  section, fix stale `src/parser/pest/` path (parser lives in
  `crates/quoin-syntax`).

## Tier 3 — "first real script" stdlib gaps

- [x] `[IO]Stdin` — `readLine`, `eachLine:`, `readAll`, `stream`, `byteStream`.
  A `blocking::Unblock` over `std::io::stdin()` registered as a `StreamId`, so
  reads **park** on the scheduler and reuse the `StringStream` protocol. A class
  rather than a prelude constant (opening stdin is an `await_io`, and the prelude
  also runs under `qn benchmark`, which has no scheduler), and the stream is
  memoized because it buffers. `tests/io_stdin.rs`, `qnlib/tests/59-io-stdin.qn`.
- [x] `[OS]Env` — read-only (`at:`, `at:ifAbsent:`, `contains?:`, `keys`, `each:`,
  `count`, `asMap`). No `at:put:`: edition 2024 makes `std::env::set_var` `unsafe`,
  and the mutation half mainly serves subprocess spawning, deferred past v0.1.
- [x] `[OS]Path` (join/dirname/basename/extension/stem/normalize/absolute?) —
  `src/runtime/os.rs`, `qnlib/tests/57-os-path.qn`.
- [x] `Runtime.exit:`
- [ ] Digests (sha256/blake3/HMAC) — optional; verified absent.
  (UUID/ULID already ship: `src/runtime/ids.rs`, `qnlib/tests/37-ids.qn`.)

## Tier 4 — packaging, CI, docs triage

- [ ] **Extension socket files must always be cleaned up on process exit.**
  `/tmp/quoin-ext-<pid>-<n>.sock` litters `/tmp` — **63 stale files** on the dev
  box, dating back four days.

  *Measured, not assumed* (2026-07-09): the graceful paths are already clean —
  normal completion, `Runtime.exit:`, and an uncaught error (exit 1) each leak
  **zero**, because `NativeExtension::drop` (`src/runtime/extension.rs:371`)
  removes the file and the arena drops before the runner exits. What leaks is
  every *signal* death: `SIGTERM`, `SIGINT` and `SIGKILL` each strand exactly one
  socket. **`SIGINT` is the common case** — a user pressing Ctrl-C on a script
  that loaded an extension. (Historically SIGBUS did it too; those two crash
  families are now fixed.) The orphaned *child* exits on its own when the peer
  closes, so only the filesystem entry is stranded.

  **Preferred fix — unlink after accept.** The *child* binds the socket and the
  host connects (`extension.rs:1238-1262`), so the standard Unix idiom applies:
  once `accept()` returns, the path has served its purpose and the child can
  `unlink` it immediately; the established connection is unaffected, and the
  protocol is a single long-lived stream with no reconnect
  (`extension.rs:286`). Then **no exit path of either process can leak**, without
  an atexit hook, a signal handler, or a sweep. Two lines, in both SDKs:
  `crates/quoin-ext/src/lib.rs:316-317` and
  `sdk/python/quoin_ext/__init__.py:434-437` — neither unlinks today.

  Still needed as a belt: the child can die *between* bind and accept, so keep the
  host's existing `remove_file` on its handshake-failure paths. A startup sweep of
  `/tmp/quoin-ext-<pid>-*.sock` for pids that are no longer alive would also mop
  up files stranded by older builds. Supersedes the narrower QUOIN_TODO item
  ("Extension socket files leak on abnormal *host* exit"), which assumed a sweep
  or a process-scoped temp dir was the only option.

- [ ] **CI tests only the root package.** The root `Cargo.toml` is both a
  `[package]` and a `[workspace]`, so CI's `cargo test` (and a bare
  `cargo nextest run`) silently skips **132 tests** in `quoin-syntax`,
  `quoin-fmt`, `quoin-ext` and `quoin-ext-proto` — 429 run of 561. The `cargo nt`
  alias now passes `--workspace` (excluding the two dylint crates, which need
  nightly `rustc-private`); CI must do the same. Also: `cargo run -- test` in CI
  needs the `qnlib/tests` argument since `qn test [DIR]` landed.
- [ ] CI: macOS runner, `cargo fmt --check` + clippy, doc-example harness,
  dependency caching, build `crates/adbc`. Swap `cargo test` for
  `cargo nextest run` (see below) — ~4× less wall time.
- [ ] Release workflow producing binaries (macOS arm64 + Linux x86_64).
- [ ] `CHANGELOG.md`.
- [ ] Status-stamp the `docs/*_ARCH.md` files (some say "not built" for shipped
  work — e.g. DEBUGGER_ARCH — and vice versa); keep them out of user-facing nav.

---

## Relocatable stdlib loading (2026-07-09)

**The binary is self-contained.** `build.rs` compiles the *shipping* stdlib subset
into `qn` (`src/stdlib.rs`): `qnlib/{core,net,web}/` plus `prelude.qn` and
`test.qn` — ~142 KB. `qn -e`, scripts, and `qn test` now work from any directory.

**The rest of `qnlib/` is a source-tree feature** and is only reachable from a disk
stdlib: the language's own `tests/`, `benchmark.qn`, and the `usetest/`/`cyc/`/
`useself/` `use`-fixtures. That is enforced by construction — they are not in the
embed list, so nothing can accidentally ship them.

- `QUOIN_STDLIB=DIR` reads the stdlib from disk instead of the embedded copy.
  `.cargo/config.toml` sets it (`relative = true` → workspace root) for every
  cargo-run build, which preserves the "edit a `.qn`, no rebuild" loop and lets
  `cargo run -- test qnlib/tests` reach the fixtures. A bare `./target/debug/qn`
  uses the embedded copy — pass `QUOIN_STDLIB=qnlib` to run the language's suite.
- `build.rs` emits `rerun-if-changed` per embedded file *and* per directory, so an
  edited or added `.qn` rebuilds. Verified: two consecutive `cargo build`s, the
  second a 0.08s no-op.

**`self:` is script-relative.** `VmOptions.self_root` is the entry script's
directory (`run`, `debug`, and a worker child's unit), so `qn /srv/app/main.qn`
resolves `use self:lib/…` under `/srv/app` regardless of the invoking CWD. The
script-less modes (`repl`, `-e`, `test`, `benchmark`) stay CWD-relative.
Extension package roots stay **CWD**-anchored on purpose (`FsResolver::package_roots`):
extensions are deferred past v0.1, and following the script would silently move
where a script finds `quoin_packages/`.

**`qn test [DIR]` runs the caller's suites** (default `tests`). The entry unit is
synthesized (`use test` + a glob of DIR + `[Test]Main.run`), never shipped;
`qnlib/main.qn` was folded into `test.qn` as `[Test]Main`. A missing directory, an
empty one, or a failing suite each exit 1 — a zero-test run must never green a CI
pipeline. This repo's own suite is now `qn test qnlib/tests` (CI updated).

**`qn benchmark`** needs a source tree (`benchmark.qn` is not embedded) and says so.

## Test-suite wall time (2026-07-08)

`cargo test` = **22.3s** wall / 37.9s user on an 18-core box — it runs the 46 test
binaries **sequentially**, parallelizing only *within* each binary, and most hold
1–5 mostly-idle socket/worker tests. Sum of binary runtimes is 15.6s; running them
all concurrently floors at ~3.3s.

`cargo nextest run` (each test its own process, parallel across binaries) =
**4.8s**, 425/425, stable across repeated runs. Aliased as `cargo nt`. There are
**zero doctests**, so nextest loses no coverage. Parallelism plateaus at the core
count (`-j 4` 11.7s · `-j 8` 7.4s · `-j 18` 4.8s · `-j 32` 4.6s); the floor is now
the slowest single test (`direct_calls`, 2.9s).

Checked and clear for concurrent execution: all test ports are ephemeral (`:0`),
extension sockets are pid-tagged (`extension.rs`), temp files are uniquified, and
no test writes into the repo tree.

**macOS gotcha — the first nextest run after a rebuild pays a Gatekeeper tax.**
nextest enumerates tests by exec'ing all 46 test binaries with `--list`, and the
first exec of each freshly linked binary is assessed by `syspolicyd`. Measured
here: **357s wall for 5.2s of tests**, with 30+ `deps/*-<hash> --list` processes
alive for minutes, zero rustc, no build lock held, and `syspolicyd` at ~34% CPU.
The immediately following run: **6s**, `syspolicyd` at 0% — the assessment is
cached per binary until it is relinked. `cargo test` hides the cost by exec'ing
binaries one at a time as it runs them. Fix: add the terminal to System Settings →
Privacy & Security → **Developer Tools** (attribution happens at process launch, so
the terminal must be restarted). Do **not** kill a slow run: the assessments only
cache once they complete, so killing restarts the whole scan.

**Also** — a `cargo test` that seems to hang for minutes may instead be blocked on
the cargo **build-directory lock**: a second `cargo` invocation waits for the first
to release `target/debug/.cargo-lock`. A foreground `cargo test` started behind a
backgrounded one measured 277s, essentially all of it waiting. Never run two cargo
commands against `target/` at once. (Measured, not assumed: rust-analyzer is *not*
a meaningful contender — it takes the lock only transiently at workspace load for
`cargo metadata` / proc-macro builds, and holds nothing while idle or serving
queries.)

Distinguishing the three: `pgrep -fl cargo|rustc|nextest`, `lsof
target/debug/.cargo-lock`, and `ps -eo pcpu,comm | grep syspolicyd`.

## `.new` on native classes: survey (2026-07-08)

Mechanism: `Callable::New`/`NewNoBlock` (`src/dispatch.rs:124-167`) mint a plain
object with **no `NativeState` payload** for any class. List/Map/Set/Bytes/
Channel were fixed by giving them explicit class-side `new` methods that win
lookup before the fallback; every other payload-backed builtin still falls
through to a poison shell whose first payload-touching method fails with the
internal `"Not a native state of the requested type"`.

Probe: `X.new` then `.s` on the result, across all 54 registered native classes
(plus payload-method spot-checks on `Fiber`/`Task`/`Instant`).

**Functional `.new` (keep):** `Object` (plain object), `List` → `#()`, `Map` →
`#{}`, `Set` → `#<>`, `Bytes` → `Bytes[0]`, `Channel` (unbuffered channel).

**Poison shell, fails on first payload method (fix):** everything else —

| Group | Classes |
|---|---|
| errors on `.s` already | `Boolean` `Integer` `Double` `String` `Symbol` `Array` `KeyValuePair` `UUID` `ULID` `BigDecimal` `BigInteger` `Duration` `Timestamp` `TimeZone` `DateTime` `[IO]File` `[IO]Handle` |
| latent (generic `X{}` render, fails on payload methods) | `Block` `Regex` `Fiber` `Task` `Method` `Timer` `Instant` `TcpSocket` `TlsSocket` `TcpListener` `ByteStream` `StringStream` `Worker` `WorkerService` `Extension` `[HTTP]Parser` `[IO]Folder` `ANSI` |
| namespace façades (instance is meaningless) | `JSON` `MessagePack` `CSV` `TOML` `YAML` `Base64` `Hex` `Math` `Async` `Runtime` `VM` |

### Fix (as built)

All three instantiation paths funnel through `ensure_instantiable`
(`src/vm.rs`): `Callable::New` / `NewNoBlock` (`src/dispatch.rs`) and the M2
fused-instantiation verdict — so one check covers interpreter + AOT (the
verdict is computed via `ensure_instantiable`, and the flags are set once at
registration, so no IC invalidation is needed).

1. `NativeClassBuilder` carries a `NativeNewPolicy` (`src/value.rs`), **default
   `Refuse(None)`** — safe-by-construction for future native classes. Declared
   per class: `.abstract_class()` (sets `is_abstract`) or
   `.construct_with("use UUID.generateV4 / …")`.
2. `Class` gains `native_new_refusal: Option<&'static str>`;
   `ensure_instantiable` raises a typed `ClassError`:
   `Cannot construct X with new — <hint>` (generic hint if the class named
   none). Abstract classes keep the standard
   `Cannot instantiate abstract class X`.
3. **Abstract**: `Object` plus the namespace façades (`Math` `JSON`
   `MessagePack` `CSV` `TOML` `YAML` `Base64` `Hex` `Async` `Runtime` `VM`
   `Timer` `[HTTP]Parser`). **Hints**: everything else (value classes point at
   literals; payload classes at their class-side constructors). **Unchanged**:
   `List`/`Map`/`Set`/`Bytes`/`Channel` (own class-side `new` wins lookup;
   their policy still governs `new:{}`), user-defined classes (never built via
   the builder), `ANSI` (a qnlib class).
4. Out of scope: user *subclasses* of native classes keep today's shell
   behavior (separate, pre-existing gap).
