# Speculative AOT: type-feedback compilation for untyped code

*Status: DESIGN — no slices shipped.*

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

### S0 — observe (no behavior change)

Collect unannotated single-dispatch methods of AOT-eligible units as
*speculative pending*: template id → (warmth count, per-param kind
lattice, return kind lattice, candidate). Lattices are `Unknown →
Int|Double|Bool|Obj → Poly`, merged at `start_block_as_method` (args)
and method return (result), only while the template stays pending.
Ship with `QN_AOT_STATS`-style counters; acceptance = zero measurable
overhead on the bench suite (the counting hash lookup must stay
miss-fast).

### S1 — speculate on parameters (entry guards)

At warmth, compile the candidate with observed `AotParam`s; scalar
observations become **entry kind preconditions** (`needs_list_self`
generalizes to `preconditions: params`) checked by `invoke()` before
any state changes — mismatch Bails to the interpreted body. `Poly` or
`Obj` params ride as Obj (still compilable — B2 proved Obj-param
bodies win). K consecutive entry Bails → tombstone (v1; demote-recompile
is a later refinement). Returns stay Obj in this slice.

### S2 — speculate on returns (the fib unlock)

Obj returns leave self-recursive scalar math boxed; fib_untyped needs
`AotRet::Scalar` to hit the fib_typed path. Speculated returns are
sound exactly where the value is produced:
- compiled→compiled sibling/self calls: the callee's declared scalar ret
  is trusted (same as typed today);
- every deopt edge (outcall result, Bail continuation): checked narrow —
  on mismatch, demote-and-retry recompiles the member with the return
  demoted to Obj (machinery exists; G3's tag-proof narrows are the
  precedent).
Acceptance: fib_untyped within 2× of fib_typed (≥10× vs today's 0.551).

### S3 — compiled field access (the richards unlock)

`LoadField`/`StoreField` translate via helpers that probe/fill the
interpreter's own field-slot cache keyed `(template_id, ip)` — the B3a
lesson applied to fields: both tiers warm one cache. Accessor-heavy
open-owner methods (richards' Tcb/Packet) then compile speculatively
like everything else. Store barriers go through the same helper (the
GC write barrier lives host-side; no barrier code in Cranelift).
Acceptance: richards ≥1.8× vs today's 0.492s.

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
