# AOT native compilation of the typed subset (Tier 2)

*Status: v0.0–v0.3 SHIPPED — **AOT is ON by default** as of v0.3 (the
soak); `QN_AOT=0` is the kill switch, always safe (the registry is a pure
overlay and the interpreter path is untouched). Default-on costs ~1–2ms of
startup on a candidate-free program; the default bench experience is
fib_typed ~0.026s and sieve ~0.123s with everything else at baseline.
History: v0.0–v0.2 shipped behind `QN_AOT=1` (then default off). v0.0/v0.1:
candidate collection, Cranelift codegen (`src/codegen/`), the registry +
`Callable::AotCall`, fuel checkpoints (prologue + loop back-edges), depth
guard, per-task counters — fib_typed 0.654s → 0.023s (~28×). v0.2 (revised
scope, wider than the original sketch): slot-window frames (every GC value
lives on `vm.stack`, rooted by construction — registers stay scalar),
object params/returns (List/Map/String), List helper ops, string constants,
`LoadGlobal`, truthiness lowering for dynamic conditions, checked Dyn→scalar
narrowing (returns and accumulators), and **outcalls** — dynamic sends
leaving the compiled world through `call_method` native re-entry
(depth-guarded, suspension-safe, thrown-value-transparent). Scalar-pure
siblings keep the direct-call fast path (translation-verified, since direct
callees share the caller's slot base). Corpus + both stress modes green in
both modes; differential tests in `src/codegen/tests.rs`; parity suites in
`qnlib/tests/40-aot-parity.qn`.*

*(G3 update: with checked generic collections shipped, sieve now COMPILES —
`List(Boolean)` proves the element read Boolean-or-nil, the dynamic branch
compiles to a nil→MNU stub, and the cold path below is never translated.
0.97s → 0.12s, ~7.9×. The paragraph below is kept as the original
motivation record.)*

*The honest v0.2 finding: sieve as written did NOT compile — its
`(list.at:p).if:{…}` lowers to `BranchIfNotBool` whose cold path
re-materializes the arm as a **capturing closure** (`SendConst(Block…)`),
and materializing an env-capturing block from a compiled frame is
deopt-grade machinery this design forswears. The method is refused with an
actionable message and stays interpreted. This is the concrete motivation
for checked generic collections: a proven `List(Boolean)` element type
removes the dynamic branch (and its cold block) at the compiler level, and
sieve then compiles with no new runtime machinery. Open: v0.3 (default-on
soak), checked generics (own design pass). Follows `docs/PERF_ROADMAP.md`
Tier 2; builds on the typed-devirt tier and the closure-template
infrastructure.*

## 1. Why, and why Quoin can

The interpreter is at its floor: 73–84% of fib/sieve time *is* the dispatch
loop, and the cheap-dispatch + devirt + inlining + Tier 1 work has mined the
in-loop wins. Past the floor there are two roads: execute fewer interpreted
instructions (register VM — months of rewrite that only helps code that
stays interpreted), or execute natively.

Python and Ruby need speculative JITs because they must *discover* types at
runtime: profile, guess, guard, deoptimize. Quoin **proves** types at compile
time:

- A typed parameter is guaranteed by dispatch itself — `|n: Integer|` is only
  selected when the argument is Integer-assignable, so inside the body the
  param *is* its type, no entry guard needed (the typed-devirt boundary
  contract).
- A `sealed!` class has a permanently frozen method table and can't be
  subclassed (`ensure_not_sealed`, vm.rs:2172; subclass check vm.rs:4178).
  Numeric and collection builtins are sealed at startup by the prelude
  (`qnlib/prelude.qn:11-23`) — a language guarantee, not a runtime query.
- The compiler already lowers proven-type operations to devirtualized ops
  whose semantics live in one shared module (`src/devirt_ops.rs`), used by
  both the interpreter arms and the native methods they shadow.

So typed, sealed methods can be compiled to native code **ahead of
execution, with no speculation, no type feedback, no deoptimization**. The
interpreter remains the home of everything dynamic. Devirtualization was the
down-payment; this is the payout — and a fully-inlined, fully-devirtualized
typed method body is already a flat sequence of typed ops, which is exactly
the input a codegen backend wants.

Backend: **Cranelift** (`cranelift-jit` + `cranelift-frontend` +
`cranelift-module`), not LLVM — lean, in-process, pure Rust, fast compile
times, and we need in-process codegen anyway (see §4.1 on timing).

## 2. Ground rules (what keeps this sound and small)

1. **No speculation, no deopt.** A method is compiled only when its types
   are *proven* (sealed receiver class, typed params, statically-typed
   body). If it can't be proven, it isn't compiled — the interpreter runs
   it, exactly as today. "Bail" is a compile-time refusal, never a runtime
   transition out of native code with live state.
2. **`devirt_ops.rs` is the single semantics source.** Compiled code calls
   (or emits inline code differentially tested against) the same verbs the
   interpreter uses: `int_bin` (wrapping add/sub/mul; only zero divisor
   raises `ArithmeticError` — `i64::MIN / -1` wraps by design,
   devirt_ops.rs:43), `double_bin` (never raises; `/`/`%` produce inf/NaN),
   `list_get`/`list_set`/`map_get`. The drift-guard discipline from the
   devirt tier extends unchanged.
3. **Behavior-identical or not compiled.** Same results, same errors, same
   error *types*, same observable scheduling granularity (§5). The corpus,
   the stress modes, and the bench checksums are the contract.
4. **The interpreter is not a fallback tier — it's the other half.** Sends
   from compiled code to anything non-compiled go through the ordinary
   dispatch path (v0: such methods simply aren't compiled; v1+: a boundary
   call, §6.3).

## 3. The v0 compilable subset

A method is a v0 candidate iff, at compile time (all of this is knowledge
the compiler already has — `ClassCtx.sealed`, `static_type`,
`escapes_inlined_frame`):

- Its class is sealed in-unit (`is_sealed_marker`,
  src/compiler/class_info.rs:12) or is a startup-sealed builtin.
- Every parameter is annotated with a scalar type: `Integer`, `Double`, or
  `Boolean`. (Unannotated params default to `"Object"` — not a candidate.)
- It has a declared scalar return type (`^Integer` etc.). The annotation
  lives only on the AST (`BlockNode.return_type`) and the checker's
  `ClassTable` — it is *not* in `StaticBlock` or any runtime metadata — which
  is one of the two reasons candidate selection must happen in the compiler,
  not at `.sealed!` time.
- The body, after control-flow inlining and self-send inlining, consists
  solely of: scalar locals (`var`/`let` with proven scalar types), the
  devirtualized Int/Double ops, comparisons, inlined `if:`/`if:else:`/
  `whileDo:` control flow (`Jump`/`IfJump`/`ElseJump`), block-returns `^`,
  and **self-calls to v0-candidate methods of the same sealed class**
  (mutual recursion within the compiled set is fine — they become direct
  native calls).
- No block literals survive inlining, no `^^`/`^>` (same
  `escapes_inlined_frame` gate the inliner uses), no field access, no
  allocation, no sends to anything outside the compiled set.

fib qualifies outright. sieve's inner loops qualify in v0.1 when
`ListGet`/`ListSet` join as runtime helper calls (§7). Everything else —
untyped code, open classes, fields, strings, blocks-as-values — stays
interpreted, forever if need be.

**What v0 deliberately excludes** (and where it goes): field access on
sealed instances (v1, needs the object header/borrow story), `Value`-typed
locals and GC-pointer-carrying frames (v1+, needs §5.2's rooting rule
extended), calls out of the compiled set (v1+, boundary convention §6.3),
compiling method bodies containing surviving block literals (probably
never — that's what the interpreter is for).

## 4. Architecture

Four pieces: candidate selection (compiler), code generation (new
`src/codegen/` module), registration (a `template_id`-keyed code registry),
and dispatch (a new `Callable` variant).

### 4.1 Candidate selection and timing

Selection is a compiler pass, not a runtime event, because (a) return types
exist only at compile time, and (b) the compiler already computes per-node
static types (`static_type`, compiler/mod.rs:1230) and per-class sealedness.
The pass runs after inlining/fusion on the compiled `StaticBlock`, walks the
*bytecode* (not the AST) — the bytecode is the post-inlining truth — and
checks that every instruction is in the v0 set with proven scalar operands.
Walking bytecode also means fusion products (`IntBinLL`, `IntBinLC`) are the
input, which map 1:1 onto Cranelift IR.

Codegen timing: **at unit compile, before execution**, in-process via
`JITModule` (this is JIT *infrastructure* with AOT *semantics* — nothing is
compiled in response to runtime behavior). The runner path already
distinguishes once-per-unit compiles from per-evaluation compiles
(`with_template_ids`, the 1b flag) — AOT compilation piggybacks on exactly
that flag: eval/REPL/interpolation compiles never AOT-compile.

A subtlety inherited from the devirt tier: `.sealed!` executes at runtime,
at the end of the class body. The compiler trusts the same source-level
`is_sealed_marker` the inliner already trusts (a direct, unconditional
`sealed!` self-send in the class body). If the marker lies (it can't — the
marker *is* the send that will run), the method table still seals before any
external caller can dispatch, because the class body runs to completion
before the class global is usable. Same soundness argument as 5·1–5·5
inlining.

### 4.2 Code generation

New module `src/codegen/` (start in-crate; split to `crates/quoin-codegen`
if it grows). One Cranelift function per candidate method:

- **Signature:** `fn(vm: *mut VmState, mc: *const Mutation, a0: i64, a1: f64, …) -> AotResult`
  where scalar params are machine values (i64/f64/i8) and `AotResult` is a
  `#[repr(C)]` `(tag: u8, value: i64/f64)` pair — tag distinguishes
  ok/error/suspend-request (§5). `vm`/`mc` are threaded for helper calls and
  checkpoints; pure-arithmetic paths never touch them.
- **Body:** locals in Cranelift SSA variables; `IntAdd`… → native `iadd`
  etc. with `IntDiv`/`IntMod` emitting the zero-divisor check and bailing to
  the error return (constructing the same `ArithmeticError("Division by
  zero")` via a helper); `Double*` → plain f64 ops (no checks, matching
  `double_bin`); comparisons → `icmp`/`fcmp`; `Jump`/`IfJump`/`ElseJump` →
  blocks and branches; self-calls to compiled siblings → direct `call`.
- **Verification:** every emitted op shape is differentially tested against
  the `devirt_ops` verb it mirrors (same harness style the devirt tier used;
  proptest over operand pairs including `i64::MIN/-1`, zero divisors, NaN).

### 4.3 Registration and dispatch

The compiled artifact registry maps **`template_id` → compiled fn pointer**
(the 1b ids are compiler-minted, process-unique, never reused — exactly the
stable identity this needs). The registry is process-global and append-only,
like the symbol interner; fn pointers are `'static`, `Copy`, no-trace.

Dispatch hookup: `lookup_method`'s tail (dispatch.rs:316-341) already
converts a resolved method into `Callable::Block`/`Native`/`ExtMethod`. When
the resolved block's `template.template_id` has a registry entry, it mints
**`Callable::AotFn { f, sig }`** instead — a new `Copy` variant. Because
this happens in `lookup_method`, the existing inline cache and dispatch
cache memoize the compiled callable exactly like any other: zero new cache
machinery, and the IC's epoch invalidation applies unchanged (not that a
sealed class's entries can ever be invalidated).

`Callable::call`'s new arm mirrors the `Native` arm's contract exactly
(dispatch.rs:136-169): unbox the `Value` args to machine scalars (they are
type-guaranteed by dispatch — a debug assert, not a runtime branch), call
the fn, box the result, `vm.push(ret)`, `Ok(())`. Errors return the
`QuoinError` like any native. `active_native_args` rooting is unnecessary
for scalar args but kept in the shim for uniformity until measured.

### 4.4 What happens to the bytecode

Nothing. The `StaticBlock` keeps its bytecode; the interpreter can always
run the method (REPL `eval:` of a send to it from an id-less compile,
debugger stepping, `QN_AOT=0`). Compiled code is a pure overlay — delete the
registry and the program still runs. This is also the kill switch: an env
var (`QN_AOT=0`) or a debugger session simply skips registry lookups, and
`--break-on-throw`/breakpoint-in-method can evict a method from the registry
wholesale (§9).

## 5. Scheduling, cancellation, and GC

The three "open questions" FUTURE_ARCH deferred turn out to have
established answers in the current runtime.

### 5.1 Preemption: fuel checkpoints via the existing suspend mechanism

Natives already suspend mid-execution: `await_io` calls
`yielder.suspend(YieldReason::AwaitIo{…})` from arbitrarily deep in native
code (vm_scheduler.rs:413), freezing the native Rust frames on the coroutine
stack; `call_method_inner`'s nested step loop suspends with
`CooperativeYield` the same way (vm.rs:1495). **A compiled method's fuel
checkpoint is the same pattern**: at loop back-edges and call entries,
decrement a fuel counter (a `VmState` field initialized from the same
`step_batch()` budget); on exhaustion call a `vm.aot_checkpoint()` helper
that (a) checks `sched.cancel_current` — mapping to the same cancellation
error `run_dispatch` raises at vm.rs:3410 — and (b) suspends with
`CooperativeYield`, resuming in place afterward.

This preserves the observable scheduling contract: compute-bound compiled
code yields with the same granularity as interpreted code (~one budget's
worth of work), `QN_SCHED_STRESS` (batch=1) forces a checkpoint per
back-edge, and the debugger's DebugBreak cadence is unchanged for
interpreted code (compiled methods are opaque to stepping — §9).

### 5.2 GC safety: the resume-segment rule

The rule that makes v0 trivially safe, stated precisely: **gc-arena collects
only between coroutine resumes** (the driver's `collect_debt` at
runner_driver.rs:540 runs after `mutate_root` returns, i.e., after the
coroutine suspends). Within one resume segment, no collection can occur, so
machine registers may hold anything. Across a suspend, the flush-before-
yielding rule applies (FIBER_REDESIGN.md:145, enforced by the
`no_gc_across_yield` lint): no `Gc`/`Value` may live on the native stack.

v0 compiled frames hold only i64/f64/bool — **no `Gc`, ever** — so they may
suspend at any checkpoint with zero rooting work. This is the whole reason
v0 is scalar-only: the dreaded stack-map/rooting problem doesn't exist for
frames that hold no pointers. When v1 admits `Value`-typed locals, the rule
is: spill them to a GC-visible home (the `VmState` stack, as interpreted
frames do) before any checkpoint that can suspend — a compile-time-known
spill set, not a runtime stack map. The lint discipline extends: the shim
and helpers are audited the same way `exec_send` is (vm.rs:3164's rooting
proof comment is the model).

### 5.3 Native stack depth

Compiled recursion consumes the real coroutine stack (1 MiB), like native
re-entry does — but without `MAX_NATIVE_REENTRY`'s protection (that guard
caps *dispatch* re-entry at 12; compiled→compiled calls don't pass through
it). Deep Quoin recursion that the interpreter handles with catchable heap
frames would overflow the machine stack uncatchably. v0 therefore carries a
**depth counter in the compiled prologue** (increment/decrement around
calls, cap tuned to the stack size, on breach return the same catchable
error shape `enter_native_reentry` produces). Cost: one add/compare per
compiled call — measured, and if it shows up, replaced by a stack-limit
probe (compare `rsp` against a bound in `VmState`).

## 6. Errors, boundaries, and what "no deopt" means concretely

### 6.1 Errors

Compiled code produces errors only at: zero divisor (`ArithmeticError`,
identical string), depth breach (§5.3), cancellation (§5.1). Each returns
through the `AotResult` error tag; the shim converts to `QuoinError` and the
normal propagation (catch:, stack traces) takes over. The method's frame
never existed, so traces show the caller — same as a native method error
today (the `last_send_args` snapshot path, dispatch.rs:157-165).

### 6.2 Why there is genuinely no deopt

Every fact compiled against is immutable once true: sealed method tables
can't change (`ensure_not_sealed` has no unseal), template ids are never
reused, dispatch re-checks argument types on every send anyway (that's what
multimethod selection *is* — the typed variant is only chosen for matching
args). There is no assumption a runtime event can falsify, hence no
transition to invalidate. The one soft spot — the REPL redefining a class
wholesale — creates a *new* class object with new method values and bumps
the dispatch epoch; stale compiled fns simply stop being reachable through
lookup.

### 6.3 The boundary, present and future

v0: compiled code calls only compiled code (same sealed class). v1 widens
this with an **outcall shim**: box the args, run the ordinary
`exec_send`-equivalent (which may push interpreter frames and suspend), and
unbox the result — legal at any checkpoint by §5.2's spill rule. Outcalls
make compiled methods composable with the whole language at the cost of
boxing at the edge; profile-guided inlining of the typed subset already
minimizes how often hot paths cross it.

## 7. Slice plan (each shippable, each measured)

- **v0.0 — skeleton:** `codegen/` module behind `QN_AOT=1` (default off),
  Cranelift dep, registry, `Callable::AotFn`, shim, differential harness.
  Compile the trivial candidate set (straight-line Int arithmetic, no
  calls). Bench: a micro `.qn` added to `bench/qn/`.
- **v0.1 — control flow + self-calls + fuel:** inlined branches/loops,
  direct calls within the compiled set, fuel checkpoints, depth guard,
  cancellation. **fib_typed compiles end-to-end.** Target: fib_typed
  approaching the native-Rust fib within small constant factors; corpus +
  both stress modes green with `QN_AOT=1`.
- **v0.2 — List/Map helper calls:** `ListGet`/`ListSet`/`MapGet`/`MapSet`
  as calls into the existing verbs (receiver checked by the helper exactly
  as the interpreter arm does, vm.rs:3844; on guard failure the helper
  raises — within compiled code the *static* types were proven, so a
  non-list receiver is impossible by construction, but the check stays until
  differentially demonstrated redundant). **sieve's inner loops compile.**
- **v0.3 — flip the default:** `QN_AOT=1` by default after a full-suite
  soak (corpus, stress modes, web framework tests, bench checksums);
  `QN_AOT=0` remains the kill switch. Re-baseline the cross-language
  comparison with the PGO+AOT binary.
- **v1 (sketch, separate design pass):** sealed-instance field access
  (slot indices are compile-time-known per §1c's layout facts; the open
  question is the borrow discipline in native code), `Value` locals with
  spill-at-checkpoint, outcalls, `String` ops.

## 8. Testing

- **Differential ops harness:** every emitted op shape vs its `devirt_ops`
  verb, property-tested over edge operands (`i64::MIN`, `-1`, `0`, NaN,
  ±inf).
- **Differential execution:** every compiled method also runs interpreted
  (`QN_AOT=0`) in CI; the corpus and `bench/run.py` checksums must be
  bit-identical. The bench runner's exit-nonzero-on-checksum-failure already
  gates this (it caught the SendField bug; it will catch codegen bugs).
- **Scheduling:** `QN_SCHED_STRESS=1` with batch=1 forces per-back-edge
  checkpoints; a wedge test (compiled infinite loop + a concurrent task +
  cancellation) proves preemption and cancellation inside compiled code.
- **Stack:** a deep-recursion test asserting the catchable depth error, not
  a process abort.
- **Perf:** `bench/run.py` A/B per slice (plain-LTO + same-source control,
  per the Tier 1 methodology; PGO corroboration only).

## 9. Open questions (tracked, not blocking v0)

1. **Debugger interplay.** Compiled methods are invisible to `qn debug`
   stepping. Simplest v0 policy: a debug session (or `--break-on-throw`)
   disables the registry for the run. Later: per-method eviction, or
   compile-with-checkpoints-per-line under debug.
2. **Coverage.** Same shape: `--coverage` runs disable AOT (coverage is
   already a whole-run mode); compiled methods would otherwise vanish from
   the numerator.
3. **Fuel cost.** One decrement/branch per back-edge; if measurable on tight
   loops, coarsen (check every N iterations by loop-trip estimation) without
   changing the observable contract materially.
4. **Cranelift version/platform pinning.** aarch64-apple first (the dev
   machine); x86_64 should Just Work via Cranelift but is untested until CI
   exists. Wasm/JIT-hostile platforms keep `QN_AOT=0`.
5. **Code lifetime.** Registry is append-only, process-lifetime (like the
   interner). REPL sessions that repeatedly `use` fresh units could
   accumulate code; acceptable at the interner's precedent, revisit if REPL
   AOT ever matters (it's off there anyway — id-less compiles).
6. **The collector revisit.** Deferred exactly as FUTURE_ARCH says: only
   when native execution makes allocation dominate. v1's spill rule is
   designed so the collector question stays orthogonal.
7. **VM statistics surface (wanted eventually, wider than AOT).** Expose
   the engine's own behavior as *fast* always-on counters — plain `u64`s
   bumped on the relevant paths, near-zero cost, no sampling: GC
   (collections, debt cycles, bytes/objects allocated, pacing), dispatch
   (IC hits/misses, global-cache hits, megamorphic sites), compile-time
   decisions (methods inlined, devirt ops emitted), and AOT (candidates
   found/compiled/refused + why, fuel yields, depth-guard trips,
   registry size). The AOT slice of this SHIPPED as `VM.stats` (a Map of
   sections; `'aot'` = compiled/refused/skipped + per-kind counts) and
   `VM.aotRefusals` (the per-member drill-down) — src/runtime/vm_stats.rs,
   backed by `RefusalKind`-tagged refusals and recorded candidacy skips in
   src/codegen/mod.rs. GC/dispatch/compile-time sections remain future
   work and slot into the same section Map. A regression test can assert
   "this method compiled" (tests/vm_stats.rs), and `bench/run.py` could
   collect the counters per run.
   AOT landed its counters early rather than retrofitting;
   the full surface is its own small design pass.
8. **Block templates.** Blocks passed to combinators are never candidates:
   `value:` enforces nothing, so their typed params are beliefs, not the
   dispatch-backed guarantees `AotParam` builds on. Two future mechanisms,
   analyzed in GENERICS_ARCH §12: runtime `value:`-time checks for
   explicitly-typed blocks (general, covers escaped callbacks), and
   tag-flow through AOT-inlined iteration protocols (free, covers the
   hot combinator-loop case). They compose; neither is scheduled.

## 10. Expected wins, bounded honestly

fib_typed's remaining cost is frame push/pop + dispatch per call and
interpreter decode per op — all of which vanish inside the compiled set;
several-fold on fib-shaped code is realistic (the devirt tier's "the
dramatic fib win is Tier-2" note, cashed in). sieve loses interpreter decode
on its inner loops but keeps helper-call costs — a large-but-smaller win.
btrees/richards/strings/json are **not** v0 targets (allocation-, dispatch-,
and string-bound respectively) and should be expected to move ~0%; that's
what Tiers 1/3 are for. The strategic value is the beachhead: every later
slice (fields, Values, outcalls) widens the compiled subset with the
soundness story already settled.
