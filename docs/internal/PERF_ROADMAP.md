# Performance roadmap: past the interpreter floor

*Status (verified 2026-07-09 at `dbe188d`): **SHIPPED, and largely executed.** `perf/next-tier`
is merged: the PGO/LTO pipeline is `[profile.release]` (`lto = "fat"`, `codegen-units = 1`) with
the recipe in `profiling/pgo-lto/`, and the expanded cross-language suite is `bench/` (see
`bench/CROSS.md`). Most of the ranked portfolio below has since shipped as its own arc —
speculative AOT, materialization, the outcall seam, direct calls — each with a doc of its own.
This remains the live roadmap; check each item against its arc doc before planning from it.*

*Written 2026-07-04, branched from main @ `bfdf478`. Synthesizes the full
profiling history (`profiling/*/notes.md`), FUTURE_ARCH.md, and a survey of
the VM hot core into a ranked project portfolio.*

## Where the VM stands

Two optimization waves took Quoin from ~9–44× slower than CPython to
~2.4–6.7× (fib(32) untyped: ~6.7× after inline-cache v2):

- **Allocation/caching wave** (June): unboxed scalars, dispatch cache,
  FxHash, instruction-borrow, lazy send allocs, superinstructions —
  cumulatively −60–75% on the canonical trio (fib/sieve/btrees).
- **Structural wave** (July): typed devirt for Int/Double/List/Map,
  control-flow inlining (~2× typed fib), sealed-class method inlining
  (1.2–2.2× per shape), inline cache v2 (~23% untyped fib), FxHash class
  maps (~21% fib), GC pacing (~9% trees).

The current profile (post-cheap-dispatch, main @ `fc362a6`) splits the world
in two:

| benchmark | dispatch/interp | alloc + GC | reading |
|---|---|---|---|
| fib(20) | 73.5% | 11.8% | at the interpreter floor |
| sieve(10k) | 83.7% | 0.7% | at the interpreter floor |
| btrees(10) | 25.1% | 22.2% | the one alloc-bound bench |

**The floor argument:** when 73–84% of time *is* the dispatch loop, no
further shaving inside the loop moves the needle. What remains is (a) execute
fewer interpreted instructions, (b) execute natively, or (c) attack the
alloc/GC side. Dead ends are documented and mostly still valid: frame/args
pooling (mimalloc already pools; gc_arena forbids reusing `Gc` allocations),
Set devirt (membership needs dispatched `==:`), stop-the-world GC (slower).
One overturned ruling is a standing caution: the inline cache *lost* in June
and *won* in July after devirt changed what reaches dispatch — "ruled out"
entries carry conditions.

## Tier 1 — bounded wins not yet tried (days each)

*Status update: ALL FOUR SHIPPED on this branch. Measured (plain-LTO A/B,
min of 5): 1a = ~24% average (PGO dominant); 1b = btrees −15.7%, richards
−6.0%; 1c = richards −4.6%, btrees −2.2%; 1d = neutral-to-positive
(byte-hash probe gone from the profile). Cumulative over the LTO baseline:
btrees −17.3%, richards −7.6%; loop benches (fib/sieve) showed a
small +2–4% delta vs branch start — investigated and resolved: the
IC-probe indirection was the suspect, the Frame-hoisted cache cell
(shipped) measured neutral, and under PGO every build of the branch
converges — the delta was persistent code-layout noise, which PGO
recovers, not an algorithmic cost. Two methodology findings now govern all A/Bs: fat-LTO
rebuilds of identical source differ ±1.3% per bench, and PGO-training
variance is ±2–8% — so plain-LTO A/B + same-source control is the
standard, PGO corroboration only. One durable design lesson: per-ip caches
and instruction fusion interact — `SendField` (field load + send in one
instruction) can host only one cache entry per ip (details in
`profiling/field-slot-cache/notes.md`).*

### 1a. Build pipeline: fat LTO + codegen-units=1 + PGO  ← this branch

The release profile was stock (no LTO, 16 codegen units). Dispatch-heavy
interpreters are the canonical beneficiaries of LTO+PGO (CPython ships with
both for ~10–15%). Shipped here:

- `[profile.release] lto = "fat", codegen-units = 1` in Cargo.toml.
  No `panic = "abort"`: quoin-syntax's resilient parse relies on
  `catch_unwind`.
- `scripts/build-pgo.sh`: instrumented build → train on `bench/qn/*.qn` →
  merge with llvm-profdata → rebuild. Produces `target/release/qn-pgo`.

Measured on the new suite: **~24% average whole-process win** for a
build-config-only change — see the results table at the end of this doc.
PGO dwarfs LTO (~19% marginal vs ~5.5%): branch layout in the giant
`dispatch_one` match is exactly what PGO optimizes. All future
cross-language and before/after comparisons should use the PGO binary.

### 1b. Closure-template sharing

Every evaluation of a `{...}` literal clones `param_syms: Vec<Symbol>`,
`param_types: Vec<String>`, the name, and source info into a fresh
`Gc<Block>` (vm.rs, `Push(Constant::Block)`). Split `Block` into an immutable
shared template + a thin `{template, parent_env}` closure and creation
becomes one small alloc. **Compounding bonus:** the per-Block inline cache
moves to the template, so ICs persist across block re-materialization — the
exact instability that made per-call-site caching miss 176/176 times in the
2b-B spike. Lands where the old trio never looked: combinator-heavy idiomatic
code (`bench/qn/combinators.qn`).

### 1c. Field-access caching

Every `LoadField`/`StoreField` re-hashes the field name into the class's
`field_slots` FxHashMap even though the slot index is fixed per class. Either
bake slot indices at compile time for sealed classes, or add a per-instruction
field cache (class-ptr guard → slot index), same shape as the IC. The trees
object path also still has a stray SipHash map (~1.4%, flagged twice in
profiling notes, never fixed).

### 1d. Symbol-keyed method tables

Method tables are `FxHashMap<String, Value>` hashed by selector *bytes*,
while selectors everywhere else are interned pointer-hashable `Symbol`s.
Switching the tables makes every cold hierarchy lookup pointer-hash and
removes selector-String clones on method definition. fxhash-class-maps
already proved this path matters (~21% fib from the hasher swap alone).

## Tier 2 — the marquee bet: AOT-compile the typed subset

*Full design now in `docs/internal/AOT_ARCH.md` (grounded in the current runtime:
candidate selection in the compiler, `template_id`-keyed code registry,
`Callable::AotFn`, fuel checkpoints via the established native-suspend
mechanism, and the resume-segment GC rule). The sketch below is the
original scoping, kept for context.*

FUTURE_ARCH's endgame, sharpened. Python/Ruby need speculative JITs because
they must *discover* types at runtime; Quoin *proves* them at compile time —
typed params are guaranteed by dispatch (no entry guards), sealed classes are
permanently monomorphic, numeric builtins are sealed at startup. The
typed-devirt pipeline already produces what a codegen backend wants: flat,
inlined, devirtualized sequences of typed ops. Native compilation is the only
thing that breaks the 73–84% floor.

**v0 scope, avoiding all three open questions in FUTURE_ARCH:**

- Compile only sealed methods with fully-typed params/returns whose bodies
  pass `escapes_inlined_frame` and lower entirely to existing devirt ops plus
  calls to other such methods (List `at:`/`at:put:` etc. become runtime
  helper calls into the VM's own ops). Scalars only: `i64`/`f64`/`bool` in
  registers, no `Value`, no allocation. fib qualifies outright; sieve's inner
  loop qualifies via the helper calls.
- **GC interaction: none in v0.** The rooting/stack-map problem only exists
  when native frames hold `Gc` pointers. Scalar-only frames hold none.
- **Preemption is nearly free.** Native code runs *inside the corosensei
  fiber*, and the scheduler's design rule is "no `Gc` live across a suspend."
  A scalar-only native frame satisfies that trivially, so a fuel counter at
  loop back-edges / call entries can `yielder.suspend(CooperativeYield)` from
  native code and resume mid-frame. No OSR, no deopt machinery.
- **Boundary:** dispatch resolves to a `Callable::NativeCompiled` variant;
  entry unboxes args (already type-guaranteed by dispatch), exit boxes one
  return `Value`. v0 refuses to compile methods that make dynamic sends and
  grows from there.

Backend: Cranelift (lean, in-process, JIT+AOT capable, pure Rust) over LLVM.
Later slices: `Value`-carrying locals with a rooting scheme, field access on
sealed instances, then the collector revisit if/when allocation dominates
native profiles. The differential-testing discipline extends naturally —
compiled op semantics come from the same `devirt_ops.rs` verbs.

**Consequence:** if this is the direction, *skip the register VM and threaded
dispatch.* Their wins (30–50% instruction count; 15–20% dispatch mechanism)
apply only to code that stays interpreted — months of rewrite for the cold
dynamic tail.

## Tier 3 — structural projects that pay either way

### 3a. Escape-analysis stack environments

Every call GC-allocates an `EnvFrame` + vars Vec; pooling is impossible under
gc_arena, so the fix is *not allocating*. After control-flow + method
inlining, hot method bodies increasingly contain **no block literals at
all** — and a body with no blocks provably never captures its environment.
Those frames can live in a stack slab in `VmState`. Kills the last per-call
allocation, cuts GC pressure globally, and shrinks the call overhead AOT
boundary crossings will also pay. Caveats: the debugger's eval-in-frame needs
an env-materialization path; this is the "large dual-representation change"
the frame-pool notes warned about — but it is the fix they identified.

### 3b. String representation

Undiscussed in any prior doc and probably a top-3 lever for real workloads:
every string is **two GC allocations** (Object wrapper in a `RefLock` + inner
`Gc<String>`), equality is a full byte compare. Invisible in fib/sieve/btrees;
the whole game for a web-framework language doing JSON, headers, templating.
Options from cheap (single-allocation representation) to fancier (small-string
inlining). `bench/qn/strings.qn` / `maps.qn` / `json.qn` now measure it.

### Deferred, with reasons

- **Phase 3 unboxed structs** — bounded ~1.2× (FUTURE_ARCH), the
  inline-fields cancellation effect applies (bigger objects → more GC), and
  gc_arena fights the layout. Fold into the collector revisit AOT will force.
- **Generational GC** — real but enormous, and its design couples to AOT's
  rooting decisions. Wrong order to do first.
- **Register VM / threaded dispatch** — see Tier 2 consequence.
- **More superinstructions / devirt** — mined out per profiling notes.

## Tier 4 — fix the evidence base  ← this branch

Until now every conclusion rested on fib, sieve, and binary-trees — none of
which exercise polymorphic dispatch, strings, Maps, the pure-Quoin combinator
library, or native↔Quoin conversion. This branch adds `bench/`: nine
whole-process, checksum-verified benchmarks (the canonical trio plus
richards, combinators, strings, maps, json) and an A/B runner
(`bench/run.py`). See `bench/README.md`.

Still missing, next in line: an HTTP/web macro-benchmark once the web
framework (PR #46) merges — that's the workload users will actually feel —
and cross-language ports (Python/Ruby) of the six new benches to re-baseline
the "vs CPython" multiplier beyond fib.

## Sequencing

1. **Now (this branch):** PGO/LTO + benchmark expansion; re-baseline
   everything on both.
2. **Next:** Tier 1 bundle (closure templates, field caching, Symbol tables)
   — likely 15–30% aggregate on idiomatic code, and it cleans the profile
   signal before the big bet.
3. **The bet:** Cranelift AOT v0 on the scalar typed subset, grown slice by
   slice — with 3a/3b as independent parallel tracks.

## Measured results (this branch)

Whole-process, min of 5 runs via `bench/run.py`, Apple Silicon (M-series),
release builds. Baseline = pre-LTO release @ `bfdf478`; PGO trained on the
bench suite via `scripts/build-pgo.sh`.

| bench | baseline | +LTO | +LTO+PGO | total Δ |
|---|---|---|---|---|
| btrees | 1.493s | 1.388s | 1.175s | −21.3% |
| combinators | 0.723s | 0.687s | 0.565s | −21.9% |
| fib_typed | 0.646s | 0.626s | 0.468s | −27.6% |
| fib_untyped | 0.713s | 0.670s | 0.545s | −23.6% |
| json | 0.318s | 0.283s | 0.242s | −23.9% |
| maps | 0.410s | 0.393s | 0.314s | −23.4% |
| richards | 0.819s | 0.777s | 0.598s | −27.0% |
| sieve | 0.918s | 0.908s | 0.626s | −31.8% |
| strings | 0.371s | 0.347s | 0.302s | −18.6% |

Held-out validation (a workload PGO never trained on): `qn test` — 1432
tests all pass on the PGO binary, ~9% faster wall. Raw data + binaries in
`profiling/pgo-lto/` (local, gitignored).

### Branch total (baseline @ `bfdf478` → all of Tier 1, PGO build)

| bench | baseline | final | Δ |
|---|---|---|---|
| btrees | 1.507s | 0.893s | −40.7% |
| combinators | 0.729s | 0.553s | −24.2% |
| fib_typed | 0.646s | 0.450s | −30.4% |
| fib_untyped | 0.717s | 0.533s | −25.6% |
| json | 0.321s | 0.242s | −24.6% |
| maps | 0.411s | 0.299s | −27.1% |
| richards | 0.817s | 0.509s | −37.8% |
| sieve | 0.923s | 0.559s | −39.4% |
| strings | 0.374s | 0.290s | −22.5% |

Average ≈ −30% whole-process, min of 5, every checksum verified.
