# Speculative AOT: type-feedback compilation for untyped code

*Status: S0-S3 + S5 (cold-arm `^^` + nested-literal materialization) SHIPPED; 2026-07-10: speculation EXTENDED TO BLOCK TEMPLATES and the F1 purity blind spot fixed (see §7). Remaining: S4 quickening (deferred), S5c template-role `^^` (recorded), btrees profitability heuristic (btrees turned net-positive at S3; deprioritized).*

## 1. Why: the measured shape of the untyped gap

The cross-language matrix (`bench/CROSS.md`, post-block-arc) puts untyped
dispatch first among Quoin's frontiers: fib_untyped runs 6.7× behind
CPython 3.13, richards 5.2×. The fresh profile
(`profiling/untyped-dispatch/`) says the *interpreter* has no fat left to
cut: on fib_untyped, `dispatch_one` self-time is 11.7%, `exec_send` 3.9%,
`EnvFrame::get` 2.9%, allocator ~6% — no hot leaf, and the whole send
path is only ~45% inclusive, so a *perfect* interpreted send caps the
bench at ~2×. Richards is decode-loop-bound (20.4% self in
`dispatch_one`). Shaving the interpreter is in diminishing returns; the
cheap-dispatch arc's verdict stands.

What closes the gap is a tier change, and the ceiling is already
measured **on this VM**: fib_typed 0.028s vs fib_untyped 0.551s — a
**20× spread separated only by annotations**. CPython closes its version
of this gap at runtime (the 3.11+ adaptive interpreter observes operand
types and specializes sites). Quoin has the *static* pipeline
(annotations → devirt/AOT); this arc adds the *dynamic* front-end:
observe kinds at runtime, compile unannotated methods speculatively,
guard at entry, Bail to the interpreter when wrong.

Not this arc: btrees/maps/strings are allocation-bound (the other
CROSS.md frontiers) — speculation does not help them.

## 2. Ground truth (what the design stands on)

Everything speculation needs already exists; nothing observes-and-connects:

- **The one-line cliff.** `maybe_collect_aot_candidate`
  (compiler/mod.rs:728): `let Some(hint) = &arg.type_hint else { return }`
  — an unannotated param silently ends candidacy. Speculation replaces
  this `return` with "collect as speculative, kinds to be observed".
- **Entry preconditions + Bail exist.** `AotEntry.needs_list_self`
  (codegen/mod.rs:179) is already a speculative guard: compile assuming
  X, check before any state changes, Bail to `start_block_as_method`.
  Speculation generalizes it to per-param kind checks.
- **Warmth-counted lazy tiering exists.** B3a's `vm.aot_pending_blocks`
  (count per template, compile at `QN_AOT_WARM`, tombstone refusals in
  `aot_refused_blocks`) is the exact lifecycle speculative methods need;
  the map generalizes from blocks to methods.
- **The observation site is one function.** `start_block_as_method`
  (vm.rs:2147) sees every method entry with `template_id`, `args`, and
  `is_method_call` in hand — the natural place to merge arg kinds into a
  per-template profile, and it is only paid for templates still in the
  pending map (hash miss = the common case after warmup).
- **The translator already consumes kinds, not annotations.**
  `AotParam::{Int,Double,Bool,Obj}` is the interface; annotations are
  merely today's only producer. Observed kinds slot in unchanged.
- **Mispredict recovery exists.** Demote-and-retry (scalar purity
  violations recompile with demoted slots) and `AotOutcome::Bail` →
  interpreted fallback are shipped and parity-tested.
- **Redefinition safety is free.** A reopen mints a new template id and
  dispatch never reaches the stale entry (B2); multimethod/guarded
  selectors are already excluded from candidacy and from the dispatch
  cache — speculation inherits both exclusions.
- **Field caches exist for the richards slice.** `field_probe`/
  `field_fill` (vm.rs:3207/3224) memoize field slots per `(ic cell, ip,
  class)`; compiled field access reuses them through the same
  shared-`(template_id, ip)` protocol that fixed B3a's outcall regression.

## 3. The slices

### S0 — observe (no behavior change) — SHIPPED

Collect unannotated single-dispatch methods of AOT-eligible units as
*speculative pending*: template id → (warmth count, per-param kind
lattice, return kind lattice, candidate). Lattices are `Unknown →
Int|Double|Bool|Obj(Poly)`, merged at `start_block_as_method` (args)
and the three method-return sites, only while the template observes.
`QN_AOT_STATS=1` dumps the profiles (fib_untyped: `value: x64: (Int)
-> Int`).

What shipping taught (all in the commit):
- **Hot-path cost is a three-stage gate**: a process-wide observation
  budget on `VmState` (`OBSERVE_BUDGET` = 8192 events; checked first,
  one load from a hot struct — once spent, observation is one predicted
  branch per call forever) → a `spec_state` Cell ON `StaticBlock`
  (same cache line entry binding already touches; a tid-indexed side
  table cost a dependent pointer chase per call) → the cold merge.
  Partial profiles are FINE: S1's guards never trust a profile, so an
  incomplete one is merely conservative.
- **Return observation must obey no_gc_across_yield**: the popped
  frame's Gc pointers are dead after `finalize_instantiation` parks (an
  init that sleeps) — the tid is stashed as a plain u32 in the Frame at
  PUSH time (`Frame.spec_tid`, 0 = none, riding padding). The borrow
  regression suite caught the violation as a segfault.
- **Measurement at this delta scale needs an interleaved A/B *and* a
  same-binary control**: sequential runs drifted ±3-7% thermally; the
  same-binary control bounded true noise at ±1.5-2%, inside which S0's
  residual deltas sit. Acceptance: no resolvable overhead.

### S1 — speculate on parameters (entry guards) — SHIPPED

At warmth (`warm_threshold`, shared with block tiering), a pending
speculative method compiles with its OBSERVED kinds: scalar
observations become the compiled params AND entry preconditions
(checked in the dispatch arm before `invoke`; mismatch Bails);
`BAIL_TOMBSTONE` consecutive mispredictions remove the entry. The
method-cache epoch bumps at promotion so warm inline caches re-fill
with the compiled callable. Returns stay Obj (fib's `value:` refuses
on an arm-shape merge until S2's return speculation).

Interleaved A/B vs main (7 runs): **fib_untyped −17.4%, strings
−16.5%, combinators −8.5%** (new post-arc best), maps −2.6%; btrees
+2.8% — promoted bodies that are PURE OUTCALLS (makeTree-shaped: no
arithmetic to unbox) pay entry/outcall overhead without a win; a
promotion-profitability heuristic is the known refinement.

Promotion swept every unannotated qnlib method into compilation and
found SIX latent seam bugs, each now pinned in `40-aot-parity.qn`
(AotParitySpeculation + AotParitySpecSeams):
- `Block#value`/`value:` existed only as `exec_send`'s fast path —
  every `call_method`-family caller (compiled outcalls included) got
  nil. They are real methods again.
- `invoke`'s exit `truncate(base)` CHOPPED a non-local return's
  delivered value when the `^^` unwind had truncated below the window
  and re-pushed at exactly `base`. NLR outcomes skip the teardown.
- `call_method_cached` charged the 12-deep native-reentry budget per
  outcall — 12 promoted frames = spurious "recursion too deep".
  Replaced by `outcall_nesting` + a cap (`MAX_OUTCALL_NESTING`) past
  which dispatch runs the INTERPRETED body (deep untyped recursion
  degrades, never errors), plus 16 MiB coroutine stacks (the 1 MiB
  default overflowed under promoted alternation depth — SIGBUS).
- `var x = nil` slot-typing was deferred to "first store", which a
  write-captured closure performs OUT-OF-BAND through its snapshot —
  the var was never slotted, snapshotted, or written back
  (recordResult read nil forever). Materialization now forces deferred
  vars into initialized Obj slots.
- Sibling closures consumed by ONE send got INDEPENDENT snapshot envs
  where interpreted siblings share cells: an unfused `whileDo:`'s body
  advanced ITS `i` while the condition's stayed frozen (one extra
  iteration, `at:` out of range). A send consuming 2+ materialized
  closures where any WRITES a capture now refuses — those methods run
  interpreted.
- The write-back flush skipped RECEIVER-position closures.

Debug surface that made the hunt tractable (kept): `QN_AOT_SPEC_MAX` /
`QN_AOT_SPEC_ONLY` (promotion bisection), `QN_AOT_DUMP=<selector>`
(CLIF dump), promotion lines under `QN_AOT_VERBOSE`.

### S2 — speculate on returns (the fib unlock) — SHIPPED

**fib_untyped 0.582 → 0.017s (−97%)** — compiled untyped fib matches
typed per-call; vs the arc's 0.700 baseline that is ~40× and the ≥10×
acceptance is met with room. Everything else at noise once the
trampoline bug below was fixed. Four coordinated pieces:

- **Observed-scalar returns compile as scalars**, STATICALLY verified:
  a return path the translator can't prove scalar raises `RET_DEMOTION`
  and the member retries with an Obj ret (no runtime narrowing — a
  speculated ret must never raise an error the interpreter wouldn't;
  annotated rets keep their deliberate checked-narrow divergence).
- **Translation-time scalar-op devirt**: unannotated bytecode carries
  GENERIC sends (`SendLocalConst(n, 1, '<=:')`) — the compiler's devirt
  needed annotations — so the translator now emits machine ops when
  both operands PROVE scalar (sealed Integer/Double semantics, the same
  guarantee typed devirt uses). The purity whitelist admits the sealed
  operator selectors optimistically; a member that still needs an
  outcall trips the translation purity check and demotes.
- **Epoch-guarded direct self-recursion for open owners**: a
  redefinition epoch (bumped by `DefineMethod` — replace OR fresh, an
  override anywhere can change any dispatch) is stamped into entries
  that emit direct self-calls; `invoke` Bails stale entries to the
  interpreted body, which re-dispatches per send. Pinned by parity
  tests: mid-run redefinition takes effect immediately.
- **Promotion waits for a RETURN observation when the ret is
  speculated** (capped at OBSERVE_CAP): a recursive method reaches
  warmth by ENTRIES alone — fib descends past the threshold before its
  first base case returns — and would otherwise promote with an
  unknowable ret, compiled Obj forever.

The shipping bug: the demote-retry compiled the INNER fn with the
demoted Obj ret but the TRAMPOLINE still converted with the candidate's
scalar ret — Cranelift verifier error, method refused, and combinators
paid +9% for the missing compiled `>:` until `build_trampoline` learned
the effective ret. `count:`/`sum:` still refuse (mixed C/Dyn merges /
uninitialized-local reads) — a box-at-merge unification is the known
next refinement if their weight ever matters.

### S3 — compiled field access (the richards unlock) — SHIPPED

`LoadField`/`StoreField`/`StoreFieldKeep` translate via helpers that
probe/fill the interpreter's own field-slot cache keyed
`(template_id, ip)` — the B3a lesson applied to fields; write barriers
stay host-side. Two follow-ons the first richards run demanded:

- **`SendField`** (the fuser's load-field-then-send) blocked richards'
  hottest methods (`schedule`, `holdCurrent`, `release:`). It now
  pushes an UNCACHED field read (interpreter parity: the ip's cache
  slot belongs to the send IC) and shares the send tail.
- **Merge-shape unification**: mixed scalar/Dyn stacks at a join
  refused whole methods (`xorD008:`, `count:`). Both directions handled:
  box-toward-Dyn inline when the merge was planned Dyn; a
  `MERGE_DEMOTION` retry (the demote-loop pattern) re-plans a
  scalar-first merge as all-Dyn when a later predecessor arrives Dyn.

Interleaved A/B vs the S2 tip: **richards −24.6% (0.700→0.528),
combinators −15.1% (0.224→0.190 — box-at-merge un-refused `count:`),
btrees −5.5%**, rest noise. Cumulative combinators vs the block-arc
baseline: 0.700 → 0.190 ≈ **3.7×**.

HONEST ACCEPTANCE NOTE: the ≥1.8× richards target (0.49→0.27-class,
set against the PGO baseline) is NOT met — this is ~1.3× on the plain
release basis. What remains: the four task `run:` bodies refuse on
`^^`-in-materialized-cold-arms (the documented B3b boundary), and the
`@task.run:packet` megamorphic dispatch stays an outcall by design.
Cold-arm-`^^` support (e.g. cold paths that Bail the frame instead of
materializing) is the recorded follow-up if richards' weight matters.

### S5 — cold-arm `^^` + nested-literal materialization — SHIPPED

The B3b boundary is gone: a materialized cold arm may carry `^^`, and
may contain nested literals. richards compiles with **zero refusals**
(all four task `run:` bodies, `release:`/`queue:`/`addTo:`, and
HandlerTask's two-deep arms).

Mechanism (S5a): a compiled METHOD invocation that materializes a
`^^`-carrying nest mints a real frame id (same `next_frame_id`
counter) and pushes an `AotFrameMark {id, frames_len, stack_base}` —
per-task, like the frames/stack it indexes. `make_closure` stamps the
id as the closure's `enclosing_method_id` (`want_home`); the
`MethodReturn` unwind, finding the home among the marks, pops outcall
frames to the mark, delivers the value at the window base, and sets
`aot_nlr_target`; the AOT error channel unwinds the native frames and
the owning `invoke` consumes the value as its ordinary return. An
in-flight compiled-target `^^` is never absorbed: nested run loops at
their baselines propagate it (a compiled frame has no interpreter
frame of its own to pop, so "all callee frames gone" is delivery, not
completion), and `do_catch`/`do_catch_finally` treat it like
cancellation. Parity for the syntactic `{ ^^ … }.catch:` shape (a
catch-all CATCHES a `^^` interpreted) is kept by refusing
catch-family consumers of `^^` nests at translation.

S5b: the materialization gate scan is TRANSITIVE
(`scan_materialized_nest`) — nested literals execute naturally in the
interpreted closure; only whole-nest free writes (→ writebacks), `^^`
presence, guarded blocks, and the trampoline signature matter.

Three lessons the A/B taught, the hard way (first measure: sieve
+482%, combinators +60%):

- **The old refusal was accidentally load-bearing.** Un-refusing `^^`
  promoted qnlib's own glue: `Block#whileDo:` (the `^^s.whileDo:block`
  TRAMPOLINE — a recursive compiled call and a full-frame arm snapshot
  per iteration) and `any?:` (the blessed `each:{ .if:{ ^^ } }` search
  idiom — an arm snapshot per element). `^^`-arm materialization is
  now gated to sites that run at most once per invocation: refused
  inside a fused-loop span and when the arm re-sends the candidate's
  own selector. Compiling those shapes WELL (tail-calls, hoisted arm
  closures) is the recorded S5c-adjacent follow-up.
- **Per-task state, again**: `aot_enclosing_env` was a bare `VmState`
  field — two tasks parked inside compiled bodies contaminated each
  other's `make_closure` lexical chains (pre-existing; the regression
  test parks at a direct `Async.sleep:` outcall so no nested invoke's
  exit-path restore masks the read). All compiled-frame `^^` context
  now rides the task context like `aot_fuel`.
- **The honest headline: a COVERAGE slice, not a perf slice.** With
  richards fully compiled, the 15-run interleaved A/B says richards
  −1.7%/−2.0% (min/med), sieve −1%, combinators +1.4% residual
  (bounded bookkeeping + layout jitter; interpreted-only A/B ±0.2%),
  rest noise. richards' remaining weight is the megamorphic
  `@task.run:` outcall and allocation — the CROSS.md frontier order
  (alloc/GC first) stands. Artifacts: `profiling/cold-arm-nlr/`.

### S4 (deferred) — interpreter quickening

Patching hot generic Sends into the existing typed instructions from IC
feedback would help `QN_AOT=0` and cold code, but bytecode is shared
per-template (`Rc`) and copy-on-quicken is real complexity for a tier
speculative AOT largely obsoletes. Recorded, not scheduled.

## 4. Doctrine and risks

- **Refusal-not-guard stays.** Speculation adds *entry* guards only —
  cheap, checked before state changes, Bail-exact. Mid-body surprises
  remain refusals/demotions, never runtime type errors the interpreter
  wouldn't raise.
- **Semantics are unchanged by construction**: every guard failure lands
  in `start_block_as_method` with untouched args; the corpus runs the
  whole suite under `QN_AOT_WARM=1` to force maximal speculation.
- **Poly code costs one profile and compiles as Obj** — no cliff, no
  repeated recompilation churn (tombstones bound the retry loop).
- **Startup**: pending-map growth is bounded by eligible method count;
  compilation stays lazy behind warmth, so the B3a +34ms mistake cannot
  recur structurally.
- **Honest unknown**: richards' megamorphic `@task.run:packet` site
  stays a dispatch even compiled (outcall through the shared IC) — S3's
  win comes from the branchy bodies and accessors around it, hence the
  conservative 1.8× target.

## 5. Acceptance

- fib_untyped ≤ 0.055s (≥10×; ceiling 0.028 = fib_typed).
- richards ≤ 0.27s (≥1.8×).
- No regression >1.5% on any other bench in any mode; corpus green in
  default / `QN_AOT_WARM=1` / `QN_AOT=0` / GC-stress / sched-stress.
- `bench/CROSS.md` re-measured: fib_untyped flips from 0.15 to >2 vs
  CPython.

## 7. 2026-07-10 addenda: block-template speculation, and the F1 purity blind spot

**Blocks speculate now** (`perf/block-scalar-spec`). Block templates always
compiled their param as a slot-resident Obj, so scalar sends inside the
hottest blocks in the language — `(x * 3) + 1` in a `collect:` — paid classic
outcalls per element (~38ns of a 60ns/element total, `bench/micro/`). The
B3a warmth window at `codegen::block_entry_for` now doubles as argument
observation: each pending invocation merges the `valueWithSelfOrArg:` item's
kind into a lattice riding `aot_pending_blocks`. At the threshold, a
one-param block with a saturated scalar profile — and at least one
`IntBinKind`-recognized send in its body, so the lane has something to
devirt (an identity block speculated for nothing measured +12%) — compiles
its param into a register lane with an entry precondition. `invoke_block`
checks the live argument before any stack effect; a mismatch Bails to the
interpreted body and tombstones after `BAIL_TOMBSTONE` consecutive misses,
exactly like a speculated method. The trampoline/inner-signature/scalar-op
machinery was already role-agnostic — the translator needed no changes.
block-arith 60 → 22ns/element; combinators −21% whole-process. Semantics
pinned in 50-aot-parity.qn (`AotParityBlockSpec`).

**Interpreted fused `each:` sites route to speculated blocks.** B1's
interpreter splice never *calls* the block, so a compiled speculated
template sat unused at direct `each:` sites (identical under `QN_AOT=0`).
`BranchIfNotList` now carries the argument block's template id; the
interpreted arm prefers the cold send — which reaches the compiled block per
element — exactly when the template compiled WITH a speculated param (an
Obj-param compiled block stays spliced: routing those measured +23% on
maps). While pending, the guard feeds warmth by element count and samples
the first element's kind. each-arith 102 → 54ns/element.

**The F1 purity blind spot (fixed).** PR 77's strict-Boolean guard puts a
`BranchIfNotBool` + materialized arm blocks + an `if:else:` send into every
not-statically-Bool conditional. The syntactic `scalar_pure_set` scan is
reachability-blind, so that dynamically-dead cold span evicted untyped fib
from the pure set → no direct self-recursion → recursive results Dyn → the
speculated scalar ret failed `emit_return`'s static proof → `RetDemote` →
Obj ret → **8× unnoticed** (0.016 → 0.077s whole-process). The scan is now
reachability-aware (`reachable_ips`): it checks only instructions a pure
translation would visit, with `BranchIfNotBool` following only its
fall-through edge — in a member with no slot sources the operand is a
translation constant, so the guard either folds (hot edge only) or pins the
cold edge, whose slot ops then trip the translation-time purity check and
demote; the same verdict either way, decided by the authority. Unknown
instructions default to fall-through: every scan inaccuracy only ADMITS too
much, and the translation purity check backstops every admission. Pinned by
`guarded_conditional_cold_span_keeps_untyped_fib_scalar_pure`.
