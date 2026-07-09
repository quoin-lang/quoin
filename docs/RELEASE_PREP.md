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
- [ ] **CWD-coupled stdlib loading.** `src/runner.rs:43-45` loads
  `qnlib/prelude.qn` CWD-relative; `FsResolver` (`src/packages.rs:41-52`)
  hardcodes both roots to `$CWD`. An installed binary fails anywhere else.
  Direction: embed `qnlib/` via `include_dir!` with a disk-path override for
  development.
- [ ] **No LICENSE / no `license` field in any Cargo.toml.** Also missing root
  `description`/`repository`; lint crates say `authors = ["authors go here"]`.
- [ ] **CLI hygiene.** No `--help`/`--version` (treated as filenames); bare `qn`
  runs the dev scratch `qnlib/testscript.qn` and exits 0 even on VM error;
  unknown flags masquerade as missing files. Replace hand-rolled
  `VmRunnerOptions::parse` with a real arg parser (existing QUOIN_TODO item).
- [x] **`qn <file>` always exits 0.** FIXED: the mode drivers gate on a
  `UnitOutcome` (`src/runner.rs`) — an uncaught error exits 1 (a falsy final
  value does not), and `Runtime.exit:` / `Runtime.exit` request a specific
  status. The exit request is uncatchable (`QuoinError::ExitRequested`, modeled
  on `Cancelled`): `finally` blocks run, `catch:` can't swallow it, and a
  `requested_exit` flag on `VmState` makes it process-wide even from a spawned
  task; the runner exits only after the arena drops, so extension/socket
  `Drop`s run. Works in run/test/eval/REPL modes. Tests: `tests/exit_code.rs`.
- [ ] **Uncatchable SIGBUS: cyclic/deep serialization.**
  `var l=#(); l.add:l; JSON.generate:l` crashes even inside `catch:` (exit 138).
  Settled fix: depth cap (~128) in `value_to_data`/`value_to_json`/`encode_dv`.
  Repro: `qnlib/stress/audit/serialize_cycle.qn`.
- [ ] **Uncatchable SIGBUS: `execute_block` native re-entry** (an `each:` body
  re-iterating its receiver; self-sorting comparator). Repro:
  `qnlib/stress/audit/each_reenter.qn`. See QUOIN_TODO for the settled design.
- [ ] **Decide F11** (`#(-1 -2)` parses as `#(-3)`). Currently a documented
  gotcha; leaning toward grammar fix (whitespace-then-`-digit` starts a new
  element). See BUGS.md Finding 11.

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

- [ ] `[IO]Stdin` (line/byte reads — can't write a filter today)
- [ ] `[OS]Env`
- [ ] `Path` (join/dirname/basename/…)
- [ ] `Runtime.exit:`
- [ ] Digests (sha256/blake3/HMAC) — optional; verified absent.
  (UUID/ULID already ship: `src/runtime/ids.rs`, `qnlib/tests/37-ids.qn`.)

## Tier 4 — packaging, CI, docs triage

- [ ] CI: macOS runner, `cargo fmt --check` + clippy, doc-example harness,
  dependency caching, build `crates/adbc`. Swap `cargo test` for
  `cargo nextest run` (see below) — same 425 tests, ~4× less wall time.
- [ ] Release workflow producing binaries (macOS arm64 + Linux x86_64).
- [ ] `CHANGELOG.md`.
- [ ] Status-stamp the `docs/*_ARCH.md` files (some say "not built" for shipped
  work — e.g. DEBUGGER_ARCH — and vice versa); keep them out of user-facing nav.

---

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

**Separately** — a `cargo test` that seems to hang for minutes is blocked on the
cargo **build-directory lock**, not slow tests: a second `cargo` invocation waits
for the first to release `target/debug/.cargo-lock`. A foreground `cargo test`
started behind a backgrounded one measured 277s, essentially all of it waiting.
Never run two cargo commands against `target/` at once. (Measured, not assumed:
rust-analyzer is *not* a meaningful contender — it takes the lock only transiently
at workspace load for `cargo metadata` / proc-macro builds, and holds nothing while
idle or serving queries.)

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
