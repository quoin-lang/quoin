# Direct calls: retiring the outcall shell (D2.5 + D3)

*Status (verified 2026-07-09 at `dbe188d`): **PARTIAL — all slices landed; the tier ships
default-off.** D2.5a (`8489807`, skip the env swap for env-blind callees), D2.5b (`9b96ac1`,
per-entry marshaling plans), D3a (`a2a29a9`, the `dispatch_epoch` ABI) and D3b (`7f966c2`, W0
direct edges) are all on main via PR 75. The machinery is proven, but the direct-edge gate
measured net-negative, so it is **off unless `QN_DIRECT_WARM` is set** (see also
`QN_DIRECT_ONLY` / `QN_DIRECT_MAX` / `QN_DIRECT_NULL` in `docs/internal/ENV_FLAGS.md`); code lives in
`src/codegen/mod.rs` (`lane_plan`, `BakedW0`) with `tests/direct_calls.rs`. Successor arc:
`docs/internal/WINDOW_ARENA_ARCH.md`. Written 2026-07-06 at `2197545` as a plan; read it as one.*

> **Tracked as #76** — Make compiled→compiled direct calls pay (default-off tier + window arena W1).

## 1. Problem statement, with numbers

After D2 (the per-site AOT IC), a warm compiled→compiled call still
pays the SHELL — measured ~15-20ns of the original ~30ns seam, against
the ~2-5ns floor that S2's same-group direct calls prove (fib). At the
current tip (bench/CROSS.md): btrees 1.6× and richards 3.3× behind
CPython, with the shell the largest single remaining slice of both.

Itemized, one warm AOT-IC hit (`helpers::outcall` fast path →
`codegen::invoke`):

| # | cost | where |
|---|---|---|
| 1 | helper-call boundary (Cranelift→extern "C") | the `outcall` call itself |
| 2 | receiver decode (kind match) | helpers.rs `decode` |
| 3 | site peek: index + entry/epoch/recv-guard | `VmState::aot_site_peek` |
| 4 | arg lanes decode into `argv` | helpers.rs |
| 5 | arg-shape guards (`value_type_guard` per arg — borrows objects) | `aot_site_args_match` |
| 6 | S1 precondition scan | helpers.rs |
| 7 | window push (receiver + args onto `vm.stack`) | helpers.rs |
| 8 | `outcall_nesting` ± | helpers.rs |
| 9 | `entry_gates` (fiber + direct_self epoch), arity, `needs_list_self` | `invoke` |
| 10 | raw-lane build (re-encode of what the caller had in registers!) | `invoke` |
| 11 | scratch pushes (`n_scratch` × `Vec::push`) | `invoke` |
| 12 | `run_in_frame_ctx`: `enclosing_env` swap ALWAYS; home/marks per `HomeCtx` | codegen/mod.rs:439 |
| 13 | the raw call | — |
| 14 | `outcome_from_tag` + `finish_frame` truncate + `slot_write` | codegen/mod.rs |

Note the double transformation: the CALLER had scalar args as native
SSA values, encoded them into lanes (kind,bits), the helper decoded
them into `Value`s, and `invoke` re-encoded them into raw lanes. For a
scalar-heavy callee, steps 2, 4, 5, 10 are pure round-trip waste.

Two structural facts the design leans on (verified in-tree):

- **The raw ABI is uniform** (`AotRawFn`, codegen/mod.rs:164): every
  entry is `(vm, mc, fuel*, depth*, slot_base, args*, ret*) -> u8`.
  Per-callee variation lives in lane CONTENTS (which lanes are scalar
  bits vs slot indices), `n_scratch`, and ret interpretation — never
  in the signature. Cross-entry direct calls are therefore ONE
  `call_indirect` shape through `entry.raw`, no per-signature
  emission.
- **Windowless callees exist and matter.** S2's same-group direct
  calls already run scalar-pure callees with NO slot window. Any entry
  with all-scalar params, scalar ret, `n_scratch == 0`, and no `self`
  reference needs steps 7, 11, 14's truncate not at all. The S-arc
  promoted exactly these shapes (untyped fib = 40×).

## 2. D2.5 — interior specialization (one session, low risk)

Keep the helper boundary; delete interior waste. Four independent
riders, each measurable alone:

**D2.5a — skip the env swap for env-blind callees.**
`run_in_frame_ctx` does `mem::replace(&mut vm.aot.enclosing_env, …)`
unconditionally (codegen/mod.rs:458), but `vm.aot.enclosing_env` is
consulted ONLY by `make_closure` (helpers.rs — cold-path
materialization). Add `AotEntry::materializes: bool` (stamped by the
translator: did `materialize_closure` run for this body — superset of
the existing `materializes_nlr`). When false AND `HomeCtx::Untracked`,
skip the env replace/restore entirely (`run_in_frame_ctx` gains an
`env_blind: bool` arm or a split entry point). Soundness: an entry that
never materializes never reads the field; nested calls install their
own. Hazard: the flag must be stamped WHEREVER materialize_closure is
reachable, including cold spans — stamp it in `materialize_closure`
itself (set a translator field, copy into the entry at registration).

**D2.5b — per-entry marshaling plan.** Precompute at registration
(on `AotEntry`): `scalar_mask: u64` (bit per param: scalar vs Obj) +
the expected `AotKind` per scalar lane. The helper fast path then
builds raw lanes STRAIGHT from the caller's (kind,bits) lanes — a
scalar param whose lane kind matches copies `bits` verbatim (no
`Value` decode, no re-encode); an Obj param pushes the decoded value
and records its index. Steps 4/5/10 collapse into one loop; the
arg-shape guard for scalar lanes becomes `lane_kind == expected` (no
`value_type_guard` object borrow — only Obj params still derive a
class pointer).

**D2.5c — fold `entry_gates` into the peek.** The fiber check
(`vm.sched.current_fiber.is_some()`) hoists to the fast path's
entry condition (one load); `direct_self`'s `compile_epoch !=
redef_epoch()` is subsumed for site-cache hits IF the site epoch and
redef epoch bump together — verify: `bump_redef_epoch` and
`dispatch_epoch` are DISTINCT counters; the site cell guards
`dispatch_epoch` only. Either (i) also stamp `compile_epoch` in the
cell and compare, or (ii) keep the one-load gate. Measure; (ii) is
probably fine.

**D2.5d — `outcome_from_tag` slimming.** The Ok path allocates a
closure + matches ret shape per call; specialize: the helper reads
`entry.ret` once and inlines the three scalar cases + slot read.

Acceptance: btrees/richards ≥3% each on the profiling build; corpus
×5; no bench beyond noise (watch maps for layout drift — use the
same-binary shim test from OUTCALL_ARCH notes if it moves). Estimated
effort: one session including the measurement matrix. Every rider is
revertable independently — land as separate commits.

**MEASURED (profiling/direct-calls/notes.md): the interior is ~free.**
a and b landed (sound; b's lane_plan/invoke_prebuilt is D3b's baked-site
marshaling verbatim) but both are wall-flat — btrees +0.5%, richards
+0.3%, noise. c/d skipped: strictly smaller than b's deleted work. The
itemized ns live in the BOUNDARY (extern call + window discipline), not
the interior — which moves the arc's weight onto W0 and predicts the
D3c W1-(A) gate fails (a helper-assisted window keeps the boundary).

## 3. D3 — the direct-call tier

### 3.1 Architecture

Re-translate a WARM caller once its outcall sites are stable, baking
per-site guarded direct calls:

```
site is warm+monomorphic (D2 cell stats)
        │ retranslation queue (driver-side, like B3a lazy compiles)
        ▼
recompile caller tid → new AotEntry, registry overwrite
  per specialized site:
    guard: epoch live? receiver shape == baked? lane kinds == baked?
      ├─ hit:  [window push if callee needs one] → call_indirect entry.raw
      │        → tag check → ret decode                      (native, no helper)
      └─ miss: the generic emit_outcall path                 (exactly today's)
```

- **Callee identity is baked, not looked up**: the specialized site
  embeds `entry.raw` (iconst fn ptr), the marshaling plan, `n_scratch`,
  and ret shape AT RETRANSLATION TIME from the D2 cell's entry.
- **Invalidation = the existing epoch discipline.** Guards compare
  `dispatch_epoch` (via a pointer passed in `FnCtx`, like fuel/depth —
  NOT a raw VmState field offset). A bumped epoch fails every guard →
  generic path → D2 refills → optionally re-retranslate. Entries are
  leaked `'static`; a stale baked `entry.raw` is never CALLED (guard
  fails first) and never dangles. This is the same no-deopt argument
  as B2/S2 (docs/internal/BLOCK_AOT_ARCH.md §6.2).
- **Registry overwrite is already legal**: `register`/`lookup` go
  through the RwLock map (codegen/mod.rs:248); spec promotion already
  inserts at runtime. In-flight invocations of the OLD entry complete
  on their own leaked code — unchanged model.
- **Polymorphic sites never specialize** (the D2 cell's stability
  counter gates the queue): richards' `@task.run:` stays generic by
  design; its win comes only from its monomorphic sites.

### 3.2 The window fork — decide by callee tier, not globally

- **Tier W0 (windowless): all-scalar params, scalar ret,
  `n_scratch == 0`, body never references `self` as a value, not
  `materializes`, not `materializes_nlr`, not `needs_list_self`.**
  Direct call = guards + `call_indirect` + tag branch + use the ret
  register. No stack touch at all. This is S2's shape generalized
  across groups — implement FIRST; it proves the queue + guards +
  registry-swap machinery with minimal new invariants. Expect most of
  the fib-adjacent and small-arith-method traffic.
- **Tier W1 (windowed).** Callee needs its slot window on `vm.stack`.
  Three options, to be A/B'd in a spike before committing:
  - **(A) helper-assisted window**: one tiny
    `window_push(vm, recv_lane…, lanes*, plan) -> base` extern call +
    native `call_indirect` + one `window_pop(vm, base, ret…)` — 3
    boundary crossings vs today's 1 big one. Only worth it if the
    interior savings dominate; the D2.5 numbers will predict this.
  - **(B) raw Vec manipulation in native code**: probe
    `vm.stack`'s (ptr,len,cap) location at startup (write a sentinel,
    scan — or a `#[repr(C)]` shadow struct maintained by push/truncate)
    and emit inline pushes with a grow-fallback helper. Fastest;
    couples native code to Vec internals or taxes every interpreter
    stack op with shadow maintenance. HIGH-RISK — only if (A)+(D2.5)
    measurably disappoint.
  - **(C) reserve-and-write**: keep a VmState-owned fixed-capacity
    window arena (a `Box<[Value; K]>` ring indexed per depth) TRACED
    like the stack, so compiled frames get windows without Vec
    mechanics. Changes the slot-addressing story (`slot_base` points
    into the arena); `abs_slot` consumers and helpers that index
    `vm.stack[idx]` must learn two spaces, or the arena replaces the
    top-of-stack region wholesale. Medium risk; cleanest long-term.
  Recommendation: ship W0; spike (A); take (C) only as its own
  follow-up arc if W1-(A) leaves ≥5% on btrees.

### 3.3 Warmth, stability, and the queue

- `AotSiteCell` gains `hits: u32` and `stable_since_epoch` (bump-free
  hit streak). The fast path increments `hits` (one add — measure; if
  visible, count only in the fill path + sampled).
- Threshold: reuse `QN_AOT_WARM` semantics — a site is HOT at N hits
  (default high, e.g. 512) with no refill since fill.
- Queue: `codegen::retranslate_queue` (Mutex<Vec<u32>> of caller
  tids). The outcall fast path pushes the CALLER tid (available: `tid`
  arg) when a site crosses the threshold — dedup via a `queued` bitset.
  Drained where B3a drains lazy compiles (the driver boundary between
  steps — find `block_entry_for`'s pending-candidate drain and mirror
  its placement), so retranslation never runs inside a VM step.
- Retranslation inputs: the original `AotCandidate` must be RETAINED
  (today candidates for compiled entries may be dropped post-compile —
  keep a `FxHashMap<u32, AotCandidate>` of compiled-method candidates,
  or re-derive from the retained `Rc<StaticBlock>` + entry metadata).
  Slice D3a settles which; retaining the candidate is simpler.
- Per-site baking data: the translator, when retranslating tid T,
  reads T's site cells (they are stable ids — SAME ids must be REUSED
  on retranslation so the D2 cells keep working for the generic
  fallback: thread a `site_map: ip -> site_id` captured from the first
  translation through the retained candidate; `next_outcall_site()`
  only mints for first-time sites).

### 3.4 Guard emission detail (W0)

Per specialized site, in the caller's native code:

1. `epoch_now = load(fx.epoch_ptr)`; `br_if epoch_now != BAKED_EPOCH → generic`.
   (`FnCtx` gains `epoch: CVal` — a pointer to `vm.dispatch_epoch`
   passed by `invoke` alongside fuel/depth. No raw struct offsets.)
2. Receiver guard: the baked cell captured `(recv_kind, recv_ptr)`.
   For an `AV::C` scalar receiver the check FOLDS AT RETRANSLATION
   (kind is static). For `AV::Dyn`: `slot_peek`-style inline is NOT
   available without borrowing — use the mini-helper
   `guard_recv(vm, lane_kind, lane_bits, baked_kind, baked_ptr) -> i8`
   for Obj receivers in v1 (one SMALL call, still skips everything
   else), fold scalar receivers natively. Upgrading Obj receiver
   guards to raw reads is a recorded non-goal until proven necessary.
3. Arg guards: scalar lanes fold statically (caller AV kinds are known
   at retranslation; a kind mismatch means the site simply doesn't
   specialize). Obj args in W0 don't exist (all-scalar tier).
4. Preconditions: for W0 callees these are exactly the scalar-kind
   checks already folded by (3) — skip the runtime scan; stamp
   `spec_bails` semantics as N/A on the direct edge (the generic path
   still owns tombstoning).
5. `call_indirect(uniform_sig, iconst(entry.raw), [vm, mc, fuel, depth,
   iconst(0 /*no window*/), lanes_ptr, ret_ptr])` — lanes written into
   the existing `kinds/bits`-style stack buffer, raw layout per the
   baked plan. NOTE: verify `slot_base = 0` is safe for a windowless
   callee (it never derefs it — assert via the W0 entry criteria) or
   pass a poison value that faults loudly in debug.
6. Tag branch: 0 → use `ret` per baked ret kind; nonzero → route to
   the caller's existing `tag_check` machinery (errors/cancel/depth
   propagate identically).
7. `outcall_nesting`: W0 direct edges do NOT increment (no Rust-stack
   re-entry happens — the native call is flat). VERIFY against the
   MAX_OUTCALL_NESTING rationale (vm.rs:214 comments: the counter
   counts Rust-stack alternations; a flat native call adds none). The
   callee's own interior outcalls still count normally. Depth/fuel:
   the callee's checkpoint handles both (same pointers).

### 3.5 Slices

- **D3a — plumbing, no behavior change** (1 session): candidate
  retention map; site-id reuse on retranslation (`site_map`);
  `epoch` pointer in `FnCtx`/`invoke`; site hit counters +
  `QN_DIRECT_WARM` env knob (ENV_FLAGS.md entry); retranslation queue
  + driver drain that recompiles and OVERWRITES the registry entry —
  but emitting the IDENTICAL generic code (a "null retranslation").
  Acceptance: corpus ×5 with retranslation FORCED on every warm site
  (`QN_DIRECT_WARM=1`), zero perf change, `QN_AOT_STATS` reports
  retranslation counts.
- **D3b — W0 direct edges** (1-2 sessions): baked guards + uniform
  `call_indirect` for all-scalar sites; bisect hooks
  (`QN_DIRECT_ONLY=tid,tid` / `QN_DIRECT_MAX=n` mirroring the S1
  hooks — they found every seam bug of that arc). Acceptance: fib
  family/scalar-heavy corpus shapes unchanged semantically; measured
  win on btrees' scalar sites; corpus ×5 incl. GC/SCHED stress;
  redefinition tests (redefine a directly-called method mid-run →
  guard fails → generic → correct new behavior; epoch-bump storm
  test).
- **D3c — W1-(A) spike + decision** (1 session): helper-assisted
  window for windowed monomorphic sites behind a flag; interleaved
  15-run A/B vs D2.5 interior. DECISION GATE: keep only if btrees
  gains ≥3% beyond D2.5; else record numbers and stop at W0 (option C
  becomes its own future arc).
- **D3d — hardening** (1 session): the seam-bug hunt. Sweep matrix:
  ×5 corpus modes × {QN_DIRECT_WARM=1, default} × SCHED seeds; the
  audit-style standalone repros for: NLR through a direct edge (`^^`
  from a W0 callee — W0 excludes materializes_nlr, so `^^` cannot
  originate there; ASSERT that), park/cancel during a direct callee's
  interior outcall, redefinition mid-recursion, tombstone interplay,
  fiber-context refusal (current_fiber gate on the direct edge — the
  guard must include it or W0 must be proven fiber-safe: DECIDE in
  D3a, default = include the fiber load in the guard).
- **Close-out**: profiling artifacts per doctrine, CROSS re-measure,
  ENV_FLAGS.md, memory/docs updates.

### 3.6 Effort and payoff, honestly

Effort: D2.5 ≈ 1 session; D3a-d ≈ 4-6 sessions of the kind this repo's
arcs take, with the hardening slice as large as any feature slice
(every arc that touched this boundary shipped with 3-7 latent seam
bugs found by the stress/bisect sweeps; D1/D2 going clean means the
debt is still out there).

Payoff model (from the D2 shim experiment + S2 floor): the shell is
~15-20ns/call ≈ 10-20% of btrees and richards. D2.5 should take
roughly a third of that for a session; W0 most of the rest for
btrees-like monomorphic scalar traffic; richards is CAPPED by its
deliberate megamorphism (only its monomorphic sites specialize).
Combinators joins only when `block_call` (the vWSOA seam) gets the
same treatment — recorded as a separate extension, same architecture,
after D3 stabilizes.

## 4. Soundness invariants (the checklist for review)

1. A direct edge is taken ONLY under: live `dispatch_epoch`, baked
   receiver shape, statically-folded scalar arg kinds, (fiber gate —
   see D3d), callee tier criteria proven at retranslation.
2. Stale entries are unreachable, never freed: leaked `'static`,
   guard-fenced. No deopt, no patching of RUNNING code — retranslation
   REPLACES the registry entry; old code finishes in flight.
3. Site ids are stable across retranslations of the same tid (the D2
   cells and generic fallback keep working).
4. W0 callees cannot touch `vm.stack`, `enclosing_env`, or `^^`
   machinery — enforced by the tier criteria AT RETRANSLATION, and
   debug-asserted (poison slot_base).
5. `outcall_nesting` invariants: direct flat calls add no Rust-stack
   alternation; interior outcalls still count. The MAX_OUTCALL_NESTING
   cap continues to bound coroutine-stack growth.
6. The PERF-SACRED arms (interpreter Return / run_dispatch step; R5
   lesson) are untouched by this entire plan.

## 5. Non-goals (recorded so they stay decisions, not drift)

- No raw native reads of object/class internals in v1 (Obj receiver
  guards go through the mini-helper).
- No `block_call`/vWSOA specialization until D3 stabilizes.
- No deopt/on-stack replacement — the no-deopt posture stands.
- Window option (B) (raw Vec internals) only after (A) + D2.5 numbers
  prove ≥5% remains on the table; option (C) is its own arc.

## 6. Doctrine

As every perf arc: whole-process wall time; interleaved 15-run
`--compare` per slice with quiet re-runs authoritative; same-binary
shim tests for any bench that moves ≤3% (the maps lesson —
`/usr/bin/time`'s 10ms resolution cannot adjudicate it); artifacts +
matching binaries under `profiling/direct-calls/`; corpus ×5 per
slice; `qn check qnlib/warnings.qn` canary; bisect env hooks land WITH
the feature, not after.
