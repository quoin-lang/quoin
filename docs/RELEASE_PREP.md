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

DONE 2026-07-10 — nine parts (§1–48), 78 runnable examples / 122 assertions in
the book plus 18/36 in the README, all CI-enforced by `qn doc --check`. The
original assessment for the record: coverage stopped at ~40% and most examples
predated strict `var`/`let`.

- [x] **Fix stale examples.** The harness found exactly 18; all fixed — several
  were real rot beyond staleness (a semantically broken `case:` example, a
  `select:`-chain precedence trap, examples relying on pre-seal behavior).
- [x] **Doc-example harness**: `qn doc --check` (one engine, two corpora — the
  book's ```quoin fences and the stdlib doc-comment examples). `"* -> value`
  annotations assert exact results; CI runs both on every push. Book:
  78 examples / 122 annotations; stdlib: 404 / 471; all green.
- [x] **Async/concurrency section** — Part V §18–21: tasks + the cooperative
  scheduler, Async, channels, workers/Parallel/Plan, park-don't-block.
- [x] **Networking/web section** — new Part VI §22–27, ending in an in-process
  `handle:`-tested app; loopback-only runnable examples.
- [x] **Type-system section** — new Part VII §28–35, with real captured checker
  diagnostics; writing it found the `T?`-param dispatch bug and the checker
  drifts recorded in Tier 4b.
- [x] **Tooling section** — new Part VIII §36–43, every behavior probed (incl.
  `qn doc --check` documenting the harness that checks the book).
- [x] **Data formats / numbers / time** — re-scoped (2026-07-10 decision):
  the generated API reference covers per-class API at 100%, so Part IX teaches
  each area in a paragraph with 1–2 verified examples and points at `qn doc` /
  `$doc` by name (linking deferred to the docs-publishing section). Extension
  usage: a Part IX pointer; full treatment stays out of v0.1 scope with the
  extension-install story.
- [x] **Small**: all folded into the Part IX re-scope — bytes/codecs and
  UUID/ULID paragraphs, stdlib map rewritten accurate to today's qnlib,
  Array/Timer/VM/streams folded in, internal pointers stripped.
- [x] **README**: quickstart, the full 10-verb CLI table, documentation
  pointers (book + `qn doc`/`$doc`), parser path fixed, and the language tour
  repaired under the harness — 18 runnable examples, 36 exact-output
  assertions, CI-checked like the book. The repair found the FRONT-PAGE example
  had been silently broken (`%{@name}` interpolation does not see instance
  variables — it printed `Mr `), a `.finally:` chain that never caught, and a
  reversed `~` matcher; all verified fixes. Tier 2 is COMPLETE.

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

## Docs publishing — generate everything as HTML, publish to the website

Decided 2026-07-10: the release ships browsable docs on the project website
(the `quoin-lang` org), not just in-repo markdown. Deferred alongside the CI
workflows (needs the org + hosting), but the shape is recorded now so the
Tier 2 work builds toward it:

- [ ] **One generated site from two sources.** The API reference is already
  HTML (`qn doc`); the language book (`docs/language/*.md`) needs a
  markdown→HTML render. Reuse the doc generator's page chrome and the shared
  code stylesheet (`highlighter::code_stylesheet`) so book pages and reference
  pages read as one site — fenced `quoin` blocks in the book render through
  `highlight_to_html` exactly like reference examples.
- [ ] **Publish pipeline**: a workflow that runs `qn doc --out site/reference
  --json`, renders the book into `site/`, and deploys (GitHub Pages or the
  org's host — decide at org move). The `--json` model also uploads, as the
  machine-readable contract.
- [ ] **Cross-linking between book and reference** — deferred with the hosting
  decision (2026-07-10): until URLs exist, the book references classes by
  name and points readers at `qn doc`; once the site exists, linkify.
- [ ] Doc-example checking (`qn doc --check`, Tier 2) runs in the publish
  pipeline too: nothing ships with a broken example.

## Tier 4 — packaging, CI, docs triage

- [x] **Extension socket files must always be cleaned up on process exit.** FIXED
  (`f4f9c91`) by the preferred fix below: both SDKs now `unlink` the path
  immediately after `accept()`. Tests in `tests/extension_socket.rs` assert the
  path is gone while the extension is live and connected, and that SIGKILL on
  the host strands nothing; they were verified to fail with the two unlinks
  reverted. The host also gained the two missing `remove_file` calls on its
  connect-failure arms. Original analysis retained:
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

- [x] **CI tests only the root package.** FIXED (`0c02adb`): the workflow now
  passes `--workspace --exclude no_gc_across_yield --exclude no_borrow_across_yield`
  to both `build` and `test`. Measured at the fix: `cargo test` ran **453 of 585**
  tests and reported green; `--workspace` runs all 585. The 132 it skipped were
  all of `quoin-syntax`, `quoin-fmt`, `quoin-ext` and `quoin-ext-proto` —
  including every parser test written for the `#(-1 -2)` fix.
- [ ] **DEFERRED until the repo moves org.** CI: macOS runner, `cargo fmt --check`
  + clippy, doc-example harness, dependency caching, build `crates/adbc`. Swap
  `cargo test` for `cargo nextest run` (see below) — ~4× less wall time.
- [ ] **DEFERRED until the repo moves org.** Release workflow producing binaries
  (macOS arm64 + Linux x86_64). Whenever it is written: it must smoke-test the
  built binary **from outside the source tree** (`cd $(mktemp -d) && qn -e …`),
  because that is the only place `QUOIN_STDLIB` is unset and the embedded stdlib
  is actually exercised. Prefer the runner's `gh` CLI over a third-party action so
  the org move costs nothing. `ubuntu-22.04` for a glibc old enough to be useful.
- [x] `CHANGELOG.md` (`55bade1`). Heading is dated at tag time.
- [x] Status-stamp the docs (`bfd59ca`). Every file under `docs/` now opens with a
  Status line from a fixed vocabulary, verified against the tree rather than from
  memory, and `docs/README.md` splits the user-facing reference from the internal
  design notes. Five docs made **false** claims — `DEBUGGER_ARCH` ("No debugger
  code exists yet"), `EXT_PACKAGING` ("not built"), `DIRECT_CALLS_ARCH` and
  `WINDOW_ARENA_ARCH` ("no slices implemented"), `TYPED_DEVIRT_ARCH` ("before any
  VM code is written"). Three more had a lead sentence lagging their own body.
  Nothing claimed shipped for work that was not built. `ENV_FLAGS.md` claimed to
  list "every environment variable the VM reads" and omitted six, including the
  user-facing `QUOIN_STDLIB` and `QUOIN_PATH`.

## Tier 4a — found while writing the release notes (2026-07-09)

Each of these was found by checking a claim against the binary instead of trusting
it. None is fixed.

- [x] **A compile error in a script file panics.** FIXED (`0e467c7`). The three
  `panic!("Compilation error: …")` sites sat inside `arena.mutate_root`, whose
  closure returned `()`; they now return `Result<(), String>`. All four entry
  points (`qn FILE`, `-e`, `check`, `benchmark`) print `Compile error: …` and
  exit 1. Tests in `tests/exit_code.rs` assert the message, the exit code, and
  the *absence* of `panicked` / `RUST_BACKTRACE`.


- [x] **Reading an undeclared identifier silently yields `nil`.** FIXED — it now
  raises a catchable `NameError`, in both the interpreter (`src/vm.rs`) and
  compiled code (`load_global` in `src/codegen/helpers.rs`), verified to agree.

  A compile-time check is impossible: `use` executes at run time, so a unit cannot
  see the globals its own `use` will define, and a method may name a class defined
  later in the file. Both would be false positives. At run time everything is bound.

  Measured before changing anything: instrumenting every missing-global read showed
  **exactly one** across the whole 1862-assertion suite, and zero in the prelude and
  the benchmarks. That one was `TSCNC_ReqBad.defined?` — the "is this class defined?"
  idiom, which only worked *because* a missing name read as nil. It is replaced by
  `Class.exists?:#Name` (`src/runtime/class.rs`), which asks the question directly;
  `Object#defined?` is unchanged for nil-testing a value. Tests: `qnlib/tests/60-names.qn`.

- [x] **No file-write API.** FIXED. `[IO]File.create:` / `append:` return a
  writable `ByteStream` over the same async backend a socket uses (a new
  `IoRequest::OpenFileWrite`, `async_fs::OpenOptions`); `StringStream` gained
  `write:` / `writeln:` / `flush!`; and `qnlib/core/06-io.qn` adds the one-shot
  `[IO]File.write:to:` / `append:to:` / `read:`. Also `delete:`, `rename:to:`,
  `exists?:`, `[IO]Folder.create:` / `delete:` (synchronous, like `open:`'s
  metadata read). Tests: `qnlib/tests/61-file-write.qn`, `tests/file_write.rs`.

  **Buffering.** File write streams buffer 16 KiB; sockets stay write-through,
  because `[HTTP]Server` writes a response and then waits for the client, and a
  buffered socket write would stall it (the socket test hangs if you buffer them —
  verified). `flush!` is a no-op on a write-through stream, so the same code runs
  over both. `close` flushes.

  **Exit flush.** A stream the program never closed is flushed by the driver when
  the program ends — after normal completion, after `Runtime.exit:`, and after an
  uncaught error — because a `Drop` may not perform async I/O. It also fires per
  REPL line, so `take_pending_writes` drains buffers *without* untracking streams
  that are still open. Signal death still loses the buffer, as in C.

  Also removed: `NativeIoHandleWrapper::File`, a blocking `write_all` on the
  scheduler thread that no Quoin code could reach (only a Rust test constructed it).

## Tier 4b — found while building the file-write path

- [ ] **The backend allocates and zeroes a fresh buffer on every `Read`.**
  `io_backend.rs`: `let mut buf = vec![0u8; max];` per fill, then truncate, copy
  into the stream's `rbuf`, and free. Measured cost: reading a 64 MiB file, the
  overhead above the 4.5 GB/s floor is +9% at a 16 KiB fill and +4% at 32 KiB, but
  it *rises again* to +26% at 64 KiB, where the per-read allocation crosses the
  allocator's large-object threshold. Reuse one scratch buffer per stream and the
  curve should keep improving; then revisit `IO_BUFFER_BYTES` (32-64 KiB) for file
  streams specifically. The measurement and the reasoning are recorded on
  `IO_BUFFER_BYTES` in `src/runtime/streams.rs`.

- [ ] **Compile errors carry no line/column.** `compile_program` returns a bare
  `String`, unlike parse errors and checker diagnostics, which have spans.

- [x] **Integer overflow panics the VM (debug) / wraps (release).** FIXED:
  overflow now raises a catchable `ArithmeticError("Integer overflow")` in every
  tier — `int_bin` (interpreter + devirtualized superinstructions) uses checked
  ops, and the AOT codegen's `emit_int_bin` emits `sadd/ssub/smul_overflow` with
  a cold bail to the new `TAG_INT_OVERFLOW`. `i64::MIN / -1` (the one
  overflowing quotient, previously deliberately wrapped to match the compiled
  `ineg`) raises too; `MIN % -1` stays 0. The `codegen/tests.rs` parity sweep
  holds the two implementations together over the MAX/MIN edges, and its oracle
  lost its "wherever the reference can't panic" carve-out — `int_bin` can no
  longer panic on any input. Tests: `qnlib/tests/63-overflow.qn`,
  `tests/overflow_aot.rs` (a warmed compiled method overflows catchably; AOT-off
  agrees byte for byte).

  Measured free (interleaved A/B, release, min of 7 pairs/program): fib_typed
  +0.14%, fib_untyped +0.20%, richards −0.61%; worst first reading (strings
  +2.15%) shrank to +0.32% at 15 pairs — layout noise. The overflow flag is what
  the hardware computes anyway; the bail branch is never taken.

  Deliberately NOT taken: promotion to BigInteger on overflow (Smalltalk-style).
  That is a language-semantics decision with a value-representation cost on the
  hottest paths; raising keeps v0.1 honest and leaves promotion open.

- [ ] **Non-trailing splat destructuring binds wrong.** Found making the book's
  §4 example runnable (2026-07-10), confirmed by hand: `var a *_ z = #(1 2 3 4 5)`
  binds `z = 3` (should be 5), and `var *init last = #(1 2)` binds `init` to the
  WHOLE list while `last = 2` — the splat greedily takes everything to the end and
  post-splat targets still bind positionally from the start. Trailing splats
  (`var p q *rest = …`) are correct. No test coverage exists for non-trailing
  splats. Either fix the compiler's binding plan or restrict the grammar to
  trailing splats; the book's §4 prose currently soft-claims "any position" and
  shows only verified trailing forms — align it with whichever way this goes.

- [x] **Top-level method definitions die with "Cannot extend sealed class
  [/]Nil".** FIXED with option 1, the targeted compile-time diagnostic:
  `greet -> { 42 }` at unit top level (outside any class body, outside any
  block, when top-level `self` is the nil default) is rejected at compile time
  with the actual fix in the message ("methods live in classes… or bind a
  block: `var greet = { … }`"). The two legitimate shapes are preserved and
  tested: method definitions in BLOCK position (`.test:name -> { … }` — the
  test DSL itself) still create Method values against the runtime `self`, and
  `Runtime.eval:'…' self:obj` still defines on the receiver's eigenclass
  (`compile_program_with`'s `define_self` flag distinguishes the cases).
  Tests: tests/exit_code.rs (all three entry points), 05-classes.qn.

- [x] **Add `Block#finally:`.** DONE: `{ … }.finally:{ cleanup }` =
  `catch:finally:` with an empty handler list — the cleanup runs and whatever
  unwound (value, throw, cancellation, exit, `^^`) propagates unchanged; an
  error from the cleanup overrides a normal result but never masks a
  cancellation. Registered in `is_catch_family` (the AOT `^^`-parity gate
  requires listing any new absorber). The three `catch:{|e| e.throw} finally:`
  call sites in `qnlib/core/06-io.qn` — the idiom that motivated this — are now
  bare `.finally:`. Grammar note: dot form only (`{…}.finally:{…}`); a
  space-form keyword send does not chain off a bare block literal. Tests in
  02-blocks.qn: success / throw-reraise-order / typed rethrow visibility /
  cleanup-error override / `^^`.

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
