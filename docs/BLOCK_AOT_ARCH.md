# Block-template AOT: compiling the combinator tier

*Status: B0-B3 ALL SHIPPED (PR #54; the narrative below records each
slice in landing order). B0 — batched nested block-execution loops:
combinators −27.9%, maps −26.0%, strings −18.9%; the driver-stepping
profile category eliminated outright. B1 — guarded fused `each:` loops:
combinators −28.5% more, maps −30.8%, strings −22.4%; cumulative
combinators 0.700→0.357s (~2×), block-invocation machinery erased from
the profile. B1 shipped as a GUARDED COMPILER INLINE rather than
translator splicing (§3 revised below): a new `BranchIfNotList` peek-guard
(the `BranchIfNotBool` pattern) with the literal body spliced via the
Phase-5 `inline_block_body` machinery — so the interpreter wins on every
native-List receiver including bare `.each:` self-sends inside Iterate's
combinators, and the AOT translator compiles the shape by killing the
guard on proven receivers (`List`/`List(T)` params seed
`NativeList`/`CollectionOf` proofs; generic-annotated params became
candidates via dispatch-name erasure; `MethodReturn` now translates as the
method's return). Fusion refuses bodies that reference the rebound `self`
(bare sends/`@fields` — `valueWithSelfOrArg:` binds the ELEMENT as self),
declare top-level locals, take 2+ params, or `^>`. B2 SHIPPED —
combinators −4.5% more (cumulative 0.700→0.343s, 2.04×), modest by
design: only `collect:`-shaped bodies compile. What it built:
`use`-loaded units now mint template ids + AOT candidates (the eval
id-less policy had silently swept ALL of qnlib in — its methods could
never compile before); open-owner candidacy per the §3 amendment
(no direct calls; reopen-invalidation pinned by a parity test — note a
same-dispatch-signature reopen REPLACES, a different signature APPENDS a
multimethod variant that may never win); `Block` params ride as Obj;
returns erase like params (`^List(U)` → Obj); and the `needs_list_self`
entry precondition (a fused self-loop compiles hot-path-only; `invoke`
Bails non-List receivers to the interpreted body — Range/Generator
parity-tested). Startup +~1.5–2ms for the qnlib compile (now ~3.4ms
total vs kill switch — visible on short whole-process benches).
B3a SHIPPED — combinators −19.2% more (cumulative 0.700→0.278s, 2.52×):
block LITERALS compile as registry entries under their template_id
(method ABI + slot 0 = self≡arg, slot 1 = the param's own cell, slot 2 =
the block object; free names through `env_get`/`env_set` against the
closure's real EnvFrame chain — exact shared-cell semantics; `^^`
refuses), invoked from both seams (`valueWithSelfOrArg:` native and the
compiled `block_call` helper). Three hard lessons, each measured: (1)
compiled bodies MUST share the interpreter's inline caches —
`call_method_cached` keys outcalls by `(template_id, ip)`, the same
identity the interpreted send uses, or compiled code loses to the warm
IC it replaced (+4.2% regression before; −19.2% after); (2) eager
template compilation cost +34ms startup — candidates stash per-VM and
compile lazily at the 8th invocation; (3) fiber teardown force-unwinds
CANNOT cross Cranelift frames (process abort) — originally closed by bailing
all compiled entry inside user fibers; SUPERSEDED by the aot-fibers arc:
fibers now carry a per-fiber AotTaskState, compiled entry marks the fiber,
and an abandoned marked fiber leaks its suspended stack (corosensei
force_reset) instead of force-unwinding, so compiled frames run — and
suspend — inside fibers (see Fiber::drop for the invariant argument).
B3b SHIPPED — cold-path closure MATERIALIZATION: a compiled frame builds
a real closure over a snapshot of its whole environment, CHAINED to the
invoking frame's enclosing env (`vm.aot_enclosing_env`) so nested
closures resolve free names through the full lexical chain — the corpus
caught the unchained version turning webapp 405s into 500s (a fused
`SendConst(Block)` slipped past the collector prescan). Gates: no `^^`,
no guard block; captured READS are snapshot-exact; captured WRITES to
frame locals read back after the consuming send (`count:`'s
`{ n = n + 1 }` arm); writes past the frame hit real env cells
unchanged. Known accepted edge: a closure ESCAPING its consuming send
sees the snapshot, not later frame writes. `QN_AOT_WARM=1` = the
maximal-compilation stress mode. The whole `select:`/`reject:`/`uniq`/
`count:`/`sum:` family now compiles.

**ARC ACCEPTANCE MET: combinators 0.700 → 0.234s cumulative = 2.99×**
(maps 1.95×, strings 1.63×; every other bench at baseline).*

## 1. Why: the measured shape of combinator cost

AOT v0–v0.3 compiled the loop-shaped world (fib ~28×, sieve ~7.9×). Real
Quoin programs spend their time in `each:`-derived combinators instead,
and the baseline profile of `bench/qn/combinators.qn`
(`profiling/block-template-aot/notes.md`, main @ `4676243`) shows why
compiling them is a different problem: **there is no hot leaf**. Self-time
is distributed machinery — the interpreter dispatch loop (13.1%), the
driver/scheduler stepping (~16.6%), send + block-invocation plumbing
(~7.7%), the allocator (~6.9%), environment reads (3.5%).

The anatomy of one element of `xs.each:{ |x| sum = sum + x }`:

- `List#each:` itself is fine: its `whileDo:` inlines to native jumps and
  runs in the batched flat loop; `.at:i` is one IC-served send.
- `b.valueWithSelfOrArg:x` is the tax. It is a native method
  (`src/runtime/block.rs:93`) that calls `vm.execute_block` — which runs
  the block body on a **nested, unbatched step loop**
  (`src/vm.rs:1749-1776`): one full coroutine suspend → driver → resume
  round-trip *per instruction* of the body, plus a bytecode-`Rc` clone per
  step (`step_internal`, vm.rs:3536-3540). The flat `run_dispatch` loop
  batches ~256 steps per yield and hoists the clone; the nested loop does
  neither. (`blk.value`/`value:` have a fast path that stays in the flat
  loop — vm.rs:3343 — but no `each:`-family combinator uses it.)
- Per element: a fresh GC'd `EnvFrame` + `bind` per param + a `Frame` push
  into a plain `Vec` (no pool) + an args `Vec` (vm.rs:2137-2178).

So the win condition is *eliminating categories*, not shaving leaves —
which is what loop fusion does, and why the arc starts with an
interpreter-only slice that removes the largest category for free.

## 2. Ground truth (what the design stands on)

- **The soundness boundary is precise.** `List`/`Map`/`Set` (and the
  scalars) are startup-sealed (`qnlib/prelude.qn:11-23`): `List#each:`,
  `at:`, `at:put:`, `add:` are frozen — semantics an optimizer may bake
  in. The **Iterate mixin is deliberately open** (the prelude comment says
  so): `collect:`/`select:`/`detect:`/… are user-extensible and resolve
  live through `mixin_classes` at dispatch. Therefore: **specialize the
  sealed `each:` primitive, never a derived combinator's body.**
- **The registry seam already exists.** The AOT registry is keyed by
  `template_id`; every block literal's `StaticBlock` carries one;
  `Callable::for_block` (dispatch.rs:79-89) mints `AotCall` purely from
  it. Redefinition self-invalidates: a reopened method is a *new* block
  with a *new* template_id — dispatch simply never reaches the stale
  entry. What's missing is (a) a call seam at the block-invocation
  primitives (`value:`/`valueWithSelfOrArg:` don't consult the registry)
  and (b) a captures ABI.
- **Captures are shared mutable cells.** A block holds
  `parent_env: Gc<RefLock<EnvFrame>>`; `LoadLocal`/`StoreLocal` walk the
  chain and mutate the *enclosing* frame's slot in place
  (value.rs:603-647). `xs.each:{ |x| sum = sum + x }` genuinely updates
  the caller's `sum` per element. Compiled code must preserve cell
  semantics for anything that stays shared — or fuse the frames so there
  is nothing to share.
- **`^^` must be modeled, not excluded.** Six combinators (`all?:`,
  `none?:`, `any?`/`any?:`, `detect:`, `nth:`, `contains?:`) exit through
  a non-local return from inside an `each:` block. At runtime it is a
  frame-popping unwind to a lexical `enclosing_method_id`
  (vm.rs:4257-4288) propagated as `Err(NonLocalReturn)` through the
  nested-loop frame-count checks.
- **Today, a block literal disqualifies its whole method.** The
  translator refuses at `Push(Constant::Block)` (translate.rs:1231), so
  `xs.each:{…}` inside an otherwise-compilable method refuses the method
  wholesale — the `each:` send never even becomes an outcall.
- **G3's proof machinery is loop-ready.** A receiver's
  `CollectionOf(tag)` proof persists in a `VarSlot::Obj` local across
  blocks; `ElemOrNil` proofs are re-minted per `list_get`, so a fused
  loop's per-element narrowing works even though proofs drop at joins.

## 3. The slices

### B0 — batch the nested block-execution loops (interpreter; ships alone)

Thread the driver's step budget through `execute_block` and
`call_method_inner` so nested block execution yields at the same
~`step_batch()` granularity as the flat loop, and hoist the per-step
bytecode-`Rc` clone the way `run_dispatch` does.

- **Contract note (approved):** nested execution currently yields *more*
  often (every instruction) than top-level code; after B0 it yields with
  the *same* granularity — scheduling becomes uniform rather than
  finer-grained inside blocks. `QN_SCHED_STRESS`/`QN_GC_STRESS` force
  batch=1, so stress coverage is unchanged.
- **Risk watched:** the cancel-while-running/starvation family from the
  async audit — cancellation must be observed at the same points the
  batched flat loop observes it. The audit's repro corpus is the gate.
- Benefits all block-heavy code, interpreted or compiled, `QN_AOT=0`
  included.

### B1 — fused `each:` loops in compiled user methods

In the translator: `Push(Constant::Block literal)` immediately consumed by
`Send each:` on a receiver **proven** a native List (an `Obj` param
annotated `List`/`List(T)`, or a `CollectionOf` proof), where `each:` can
only resolve to the sealed primitive, compiles to a native index loop:

```
n = list_count(recv)
i = 0
loop:
  fuel back-edge tick
  elem = list_get(recv, i)          // ElemOrNil proof if the list is tagged
  <block body spliced inline>       // same compiled frame
  i += 1; if i < n goto loop
```

- **The closure never exists.** The body is spliced into the defining
  frame, so captures are plain SSA/slot accesses — the shared-cell
  question vanishes because there is only one frame. `sum = sum + x`
  compiles to a local update.
- **`^^` becomes a jump to the method epilogue** — its lexical target *is*
  the method being compiled.
- **`^` is the per-element value** (each: discards it) — a jump to the
  loop latch.
- **Refusal stays the safety valve:** fusion refuses on `^>` in the body,
  `catch:`/`finally:` wrapping, a block that escapes (stored, passed on),
  or an unproven receiver. Unfused = today's behavior (whole-method
  refusal shrinks to exactly the patterns not yet covered).
- Fusion set (approved): sealed `List#each:` and sealed
  `NumberRange#each:` first; the native `Set`/`Map` `each:` are a later
  extension once the pattern is proven.

### B2 — compile the Iterate combinator bodies

`collect:`/`select:`/`count:`/`sum:`/`reduce:into:`/… become AOT
candidates themselves: each body is a fused loop over `self` (an `Obj`
param) invoking the *arbitrary* block argument per element. This is where
user call sites of the combinators win without their own methods
compiling.

- Per-element invocation of the block argument: via the B3 compiled entry
  when the registry has one (compiled→compiled), else a batched
  interpreter invocation (cheap post-B0).
- **Candidacy amendment (approved):** the sealed-owner rule exists to
  protect the *direct sibling call* fast path (a frozen callee set).
  A compiled method that makes **no direct calls** is redefinition-safe by
  construction — `AotCall` is minted per-dispatch from the block's own
  `template_id`, and a reopened Iterate method is a new template that
  simply dispatches to itself. Amended rule: *methods of open classes are
  candidates iff their compiled form contains no direct calls* (every
  send crosses a dispatch-equivalent seam). AOT_ARCH §3's sealed-owner
  bullet gains this clause; §6.2's no-deopt argument extends verbatim.

### B3 — block-template compilation

Compile block literals as registry entries under their existing
`template_id`:

- **Params are `Dyn` + checked narrowing** (approved): `value:` enforces
  nothing, so a block's typed params remain beliefs — the compiled body
  narrows exactly where the interpreter would fail, preserving behavior.
  Call sites that *prove* more (a B2 fused loop over a tagged collection)
  pass proofs and the narrows fold away. §12's `value:`-time enforcement
  stays unscheduled — this arc does not need it.
- **Captures ABI:** new helpers `env_get`/`env_set` read/write the real
  `EnvFrame` cells through the block's `parent_env` — shared-mutation
  semantics preserved by construction. (Locals the body doesn't share
  stay SSA.)
- **`^^` from a compiled block:** returns through the error lane as the
  `NonLocalReturn` shape the nested-loop unwind machinery already
  understands.
- Invocation seams: B2 loops call compiled blocks directly; the
  `value:`/`valueWithSelfOrArg:` primitives get a `for_block`-style
  registry lookup so escaped blocks win too.

## 4. What this arc does NOT do

- No speculation, no deopt — refusal remains the only "bail", at compile
  time (AOT_ARCH §2 doctrine unchanged).
- No assumption about any open-class method body (Iterate stays
  reopenable; only the sealed `each:` primitives are baked in).
- No runtime `value:` enforcement (GENERICS_ARCH §12 stays a separate,
  unscheduled decision).
- No new language surface: this is pure execution-tier work.

## 5. Acceptance and measurement

- Baseline: `profiling/block-template-aot/` (before.json.gz + qn-before,
  notes.md) — combinators bench 0.67s, category breakdown recorded.
- Per slice: corpus + both stress modes + `QN_AOT=0`, bench A/B (release,
  same-source control), checksums identical; profile after each
  perf-visible slice, deltas in notes.md.
- Arc target: **≥3× on the combinators bench**; every other bench at
  baseline. B0 alone predicted ~1.3–1.5× (it removes most of the ~17%
  driver category and part of dispatch).

## 6. Open questions (tracked)

1. B1 escape analysis granularity: v1 = "literal consumed directly by the
   fused send" (syntactic); anything subtler refuses.
2. B2 set: which combinators compile first (bench-driven: `collect:`,
   `select:`, `count:`, `sum:`, then the `^^` family once B1's epilogue
   pattern is proven under B2's outlined-loop shape).
3. B3 and the debugger/coverage story: same policy as methods (compiled
   blocks are opaque; debug/coverage runs disable the registry).
4. Whether `valueWithSelfOrArg:` should get the `value:` flat-loop fast
   path independent of this arc (a B0-adjacent interpreter win; measure
   after B0).

## 7. 2026-07-10: the speculation follow-on (recorded here, designed in SPECULATIVE_AOT_ARCH §7)

Two of this arc's seams moved again on `perf/block-scalar-spec`: block
templates now speculate their argument's scalar kind through the B3a warmth
window (the vWSOA seam observes; `invoke_block` gains an entry
precondition), and the B1 guard (`BranchIfNotList`) carries the literal's
template id so INTERPRETED fused sites route to the cold send once the
block compiles speculated — the splice had been starving the block tier at
direct `each:` sites. §6.2's "which combinators compile first" is settled
differently than anticipated: the combinator bodies were already compiling
(B2/B3b); the wins were inside the *blocks* and at the *interpreted call
sites*. The per-element cost model lives in `bench/micro/`.
