# Environment variables

*Status (verified 2026-07-09 at `dbe188d`): **REFERENCE**, audited against every `env::var` call
in `src/` and `crates/`.*

The `QN_*` flags are internal tuning, debugging, and test knobs (`src/tuning.rs`'s header: "for
testing and debugging the VM, not user-facing") — programs should never need one for correct
behavior, and disabling any of them is always semantics-preserving.

The `QUOIN_*` variables are different in kind: they are **user-facing**, they appear in
`qn --help`, and they *do* change behavior. See [User-facing](#user-facing) at the bottom. This
file used to claim it listed "every environment variable the VM reads" while omitting them and
the three `QN_DIRECT_*` knobs; that claim is now scoped rather than repeated.

All `QN_*` flags are **read once on first use and cached for the life of the
process** — changing one mid-run has no effect, and checking one on a hot path
is one predicted branch.

Two value conventions coexist (noted per flag below):

- **truthy** (`env_flag` in `src/tuning.rs`): the flag is ON when set to
  anything except `""`, `0`, `false`, or `no` (case-insensitive) — so an
  explicit `QN_FOO=0` reads as off rather than surprise-enabling the knob.
- **exact match**: the flag compares against one literal value (e.g.
  `QN_AOT=0`, `QN_AOT_VERBOSE=1`). Anything else is the default.

## Summary

| flag | values (default) | what it does |
|---|---|---|
| `QN_AOT` | `0` disables (on) | AOT kill switch |
| `QN_AOT_WARM` | integer (8) | lazy-compilation warmth threshold |
| `QN_AOT_VERBOSE` | `1` (off) | per-method compile/refuse/promote lines |
| `QN_AOT_STATS` | set & ≠`0` (off) | speculation + compile totals at exit |
| `QN_AOT_DUMP` | selector or `1` (off) | bytecode + CLIF dump |
| `QN_AOT_SPEC_MAX` | integer (∞) | bisect: promote only tids ≤ N |
| `QN_AOT_SPEC_ONLY` | CSV of tids (all) | bisect: promote only listed tids |
| `QN_DIRECT_WARM` | integer (unset = off) | direct-call tier: site hits before retranslation |
| `QN_DIRECT_ONLY` | CSV of tids (all) | bisect: only these callers bake direct edges |
| `QN_DIRECT_MAX` | integer (∞) | bisect: cap how many callers may bake, process-wide |
| `QN_DIRECT_NULL` | `1` (off) | test hook: retranslate even with no baked edges |
| `QN_GC_STRESS` | truthy (off) | collect on every VM step |
| `QN_GC_SLEEP` | float (4.0) | GC pacing sleep factor |
| `QN_SCHED_STRESS` | seed or truthy (off) | randomized preemptive scheduling |
| `QN_BATCH` | integer ≥1 (256) | steps per cooperative yield |
| `QN_BATCH_STATS` | truthy (off) | per-batch wall/alloc summary |
| `QN_COMPUTE_THREADS` | integer (cores−2) | compute-offload pool size; `0` disables |
| `QN_COMPUTE_MIN` | bytes (262144) | offload gate: smaller inputs run inline |
| `QN_EXT_HANDSHAKE_TIMEOUT_MS` | integer ≥1 (10000) | extension spawn handshake timeout |
| `QN_NO_BANNER` | truthy (off) | suppress the REPL greeting |
| `QN_PROMPT` | string (`qn> `) | REPL prompt override |

## Compute offload

### `QN_COMPUTE_THREADS` / `QN_COMPUTE_MIN`

The C1 compute-offload pool (docs/CONCURRENCY_ARCH.md §4): gated CPU-bound
native ops on detached buffers — today the `Bytes` codec family
(`encodeGz`/`decodeGz`/`encodeDeflate`/`decodeDeflate`/`decodeZstd`) — run on
a small fixed thread pool while the calling task parks like an IO wait, so
concurrent tasks overlap (`Async.gather:` over 8 × 4 MB encodes measured
4.4× faster). The round trip is a flat ~10 µs and a single op never wins
from offloading, so inputs under `QN_COMPUTE_MIN` run inline
(default 262 KiB ≈ where the serial tax reaches noise);
`QN_COMPUTE_THREADS=0` is the kill switch. Counters surface as
`VM.stats` → `'compute'`.

## AOT compilation

### `QN_AOT`
**`0` disables; anything else (or unset) leaves it on.** Read in
`src/tuning.rs` (`aot_enabled`). The kill switch for the whole native tier
(docs/AOT_ARCH.md), default ON since v0.3 (PR #52). The interpreter path is
untouched either way — the compiled registry is a pure overlay — so disabling
is always safe, and `QN_AOT=0` vs default is the standing parity axis the
corpus runs under.

### `QN_AOT_WARM`
**Integer, default 8.** Read in `src/codegen/mod.rs` (`warm_threshold`). The
lazy-compilation warmth threshold shared by block templates (B3a) and
speculative methods (S1): a unit compiles on its Nth invocation/observation.
`QN_AOT_WARM=1` compiles everything on first use — the corpus's
maximal-speculation stress mode, and the way to make small repros compile
deterministically. Speculated returns still wait for one return observation
regardless (S2), so deep recursion promotes correctly even at 1.

### `QN_AOT_VERBOSE`
**Exactly `1`.** Read in `src/codegen/mod.rs` (compile/refuse) and `src/vm.rs`
(`spec_promote`). Prints one stderr line per outcome: `compiled template N`,
`refused <selector>: <reason>`, `promoted <selector> (tid N)`. The first tool
to reach for when asking "did this method compile, and if not why not".

### `QN_AOT_STATS`
**Set and not `0`.** Read in `src/runner_driver.rs` (`maybe_print_spec_stats`).
After the main task finishes, prints the speculative-AOT summary: pending/
observing counts, promotions, the top observed kind-profiles, and the
process-wide `N compiled, M refused` totals. The same aggregates — plus the
per-kind refusal/skip breakdown and the per-member drill-down — are available
to programs as `VM.stats` / `VM.aotRefusals` (src/runtime/vm_stats.rs), so a
test can assert "this method compiled" without scraping stderr.

### `QN_AOT_DUMP`
**A selector name, or `1`.** Read in `src/codegen/translate.rs`. With a
selector (e.g. `QN_AOT_DUMP=work:`), dumps that member's post-inlining
bytecode (with candidate metadata: purity, ret shape, spec flags) and its
final CLIF IR at compile time. With `1`, dumps the CLIF for every compiled
member (no bytecode). Selector names with `:` are fine — quote in the shell.

### `QN_AOT_SPEC_MAX` / `QN_AOT_SPEC_ONLY`
**Integer / CSV of template ids.** Read in `src/vm.rs` (`spec_promote`). The
speculative-promotion bisection hooks: `SPEC_MAX=N` promotes only template ids
≤ N; `SPEC_ONLY=12,340` promotes only the listed ids (get ids from
`QN_AOT_VERBOSE`). Everything else stays interpreted. The bisect-then-shrink
loop over these found every S1 seam bug; they gate promotion only — classic
annotated candidates and block templates compile regardless.

## Direct calls

### `QN_DIRECT_WARM`

Site-hit threshold for the direct-call tier's retranslation queue
(docs/DIRECT_CALLS_ARCH.md §3.3): a warm AOT-IC site that reaches this
many consecutive fast-path hits queues its CALLER for retranslation at
the next driver boundary. **Unset or `0` = the tier is off.** D3b shipped
the baked direct edges, but the gate measured net-negative, so the tier
stays off by default rather than earning one. `QN_DIRECT_WARM=1` forces
retranslation on the first warm hit — the stress/bisect setting.

### `QN_DIRECT_ONLY` / `QN_DIRECT_MAX` / `QN_DIRECT_NULL`

**CSV of template ids / integer / exactly `1`.** Read in `src/codegen/mod.rs`
(`direct_allows`, `direct_budget_allows`, `direct_null_forced`). The D3b bisect
hooks, landed with the feature under the same S1 discipline as
`QN_AOT_SPEC_*`: `ONLY` limits which callers may bake direct edges, `MAX`
caps how many may do so process-wide, and `NULL=1` is the test hook that
retranslates a queued caller even when nothing got baked — the D3a
null-retranslation contract. Production skips an empty bake, because
recompiling without edges buys nothing and costs fresh code placement
(measured +2–3% on the hot benches).

## Stress modes

These exist to surface bug classes the normal schedule hides; the corpus is
expected green under both, and both force `QN_BATCH` to 1.

### `QN_GC_STRESS`
**Truthy.** Read in `src/tuning.rs` (`gc_stress`). Collects on **every** VM
step instead of every 10, so a value reachable only through a Rust stack
across a park/yield boundary is collected immediately and surfaces as a crash
or spurious `nil` instead of a one-in-a-million heisenbug (the
`no_gc_across_yield` lint's subject, enforced end-to-end). Also pins GC
pacing to gc-arena's conservative default (`QN_GC_SLEEP` is ignored) so loose
throughput pacing can't mask a tracing bug.

### `QN_SCHED_STRESS`
**A `u64` seed, or any truthy value for the default seed.** Read in
`src/tuning.rs` (`sched_stress`). Stresses the task scheduler two ways: the
driver preempts the running task at every cooperative-yield boundary (forcing
the full `save_task_context`/`load_task_context` round-trip per step — this is
what catches per-task state missing from the swap protocol, twice now), and
picks the next ready task at random instead of FIFO (randomizing gather-child
and I/O-wakeup ordering). Seeded for reproducibility; suites are expected to
stay green across a sweep of seeds.

## Performance tuning

### `QN_BATCH`
**Integer ≥ 1, default 256.** Read in `src/tuning.rs` (`step_batch`). VM
instructions executed per cooperative-yield boundary. The yield is a coroutine
switch back to the driver (which re-enters the GC arena), so batching amortizes
it (~2× on compute-bound programs, B0). I/O parks and guest-fiber yields
suspend deeper in `step` and are unaffected, so responsiveness is preserved.
Also feeds the compiled tier's fuel budget (checkpoints per
`docs/AOT_ARCH.md` §5). Forced to 1 under either stress mode. The tuning
harness lives in `profiling/batch-sweep/`.

### `QN_GC_SLEEP`
**Float, default 4.0.** Read in `src/vm.rs` (`gc_pacing`). gc-arena pacing
`sleep_factor` — how much allocation headroom the collector grants between
collection work (PR #37). Higher = fewer collections, more memory; the default
was chosen by measurement. Ignored under `QN_GC_STRESS` (see above).

### `QN_BATCH_STATS`
**Truthy.** Read in `src/tuning.rs` (`batch_stats`). Makes `run_vm_loop`
accumulate per-batch wall time and GC bytes allocated and print a one-line
summary at finish — the measurement side of `QN_BATCH` tuning. Costs two
metric reads per batch when on.

## Extensions

### `QN_EXT_HANDSHAKE_TIMEOUT_MS`
**Integer ms ≥ 1, default 10000.** Read in `src/tuning.rs`
(`ext_handshake_timeout_ms`). How long extension spawn waits for the
`GetManifest` reply before failing. The handshake runs before any user
`Async.timeout:` can wrap it, so a silent extension binary would otherwise
park the spawning task forever. Tests lower it to exercise the timeout path
fast.

## REPL

### `QN_NO_BANNER`
**Truthy.** Read in `src/runner_repl.rs`. Suppresses the REPL greeting line
(`QN_NO_BANNER=0` still shows it — same truthiness convention as the tuning
knobs). For scripted/embedded REPL sessions.

### `QN_PROMPT`
**Any string, default `qn> `.** Read in `src/runner_repl.rs`. Overrides the
REPL prompt.

## User-facing

Not tuning knobs: these change what the program does, and `qn --help` documents them.

### `QUOIN_STDLIB`
**A directory.** Read in `src/packages.rs` (`STDLIB_ENV`). Loads the stdlib from disk instead of
the copy `build.rs` embedded in the binary. `.cargo/config.toml` sets it (`relative = true`) for
every cargo-run build, which preserves the "edit a `.qn`, no rebuild" loop and is the only way to
reach the units deliberately *not* embedded — the language's own `qnlib/tests/`, `benchmark.qn`,
and the `use`-fixtures. A bare `./target/debug/qn` uses the embedded copy.

### `QUOIN_PATH`
**Colon-separated directories.** Read in `src/packages.rs`. Extra roots searched for extension
packages, on top of `quoin_packages/` under the CWD (`docs/EXT_PACKAGING.md`).

### `QUOIN_ADBC_<NAME>_PATH`
**A driver path.** Read in `crates/adbc/src/main.rs`. Per-driver override for the ADBC extension,
which otherwise finds drivers through its manifest directory.

## Adding a flag

Put the accessor in `src/tuning.rs` (or next to its sole consumer if it is
subsystem-specific), use `env_flag` for booleans so `=0` means off, cache it
in a `OnceLock`, and add it to this file and the summary table above.

To re-audit this file, diff the table against every `env::var` / `var_os` call in `src/` and
`crates/`. Two names look like flags but are not read: `QN_GC_STEPS` (a comment in `tuning.rs`
for an unimplemented knob) and `QN_FOO` (the doc-comment example on `env_flag`).
