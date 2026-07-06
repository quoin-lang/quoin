# Allocation churn: the post-dispatch frontier

*Status: A1 + A2a-d SHIPPED on `perf/alloc-churn`. Cumulative 15-run A/B
vs main @ `b744e53` below. A2e (single-alloc String payload / collection
triple-hop collapse) is ASSESSED AND DEFERRED with rationale (§3-A2e):
inlining the String grows EVERY Object by the enum max-variant (~16B —
the inline-fields cancellation effect, and btrees is the workload it
punishes), and it breaks the copy-a-cheap-Gc-handle-out-of-the-borrow
discipline `recv!`/`arg!` and every string native rely on — either
guard-holding across ops (the borrow-across-yield hazard class) or a
clone per access. Needs its own prototype + A/B, not a rider. Next: the
A3 reassessment (escape-analysis stack envs vs btrees' sibling-closure
compile coverage) against fresh profiles.*

## 1. Why: the measured shape

The cross-language matrix (bench/CROSS.md, post-S5) puts allocation/GC
first among Quoin's frontiers: btrees 4.6× behind CPython, strings 3.7×,
with dispatch substantially closed for hot shapes. Fresh symbol-level
profiles on main @ `b744e53` (`profiling/alloc-vs-strings/`):

**btrees** (1152 samples): ~19% allocator+collector (`do_collection`
8.0%, `mi_malloc`/`mi_free` 8.5%, `RawVec` growth ~2.4%), ~30%
dispatch/interpreter (its two hottest methods REFUSE compilation — the
sibling-closure-writes gate), ~4% per-`new:` bookkeeping
(`finalize_instantiation` + `collect_instance_vars`).

**strings** (214 samples): ~20% Rust `fmt` machinery — `a + b` is
`format!("{}{}", a, b)`, and the profile's odd `QuoinError as Debug`
15% line is linker-folded (ICF) derive-fmt code reached through it —
plus ~18% allocator+collector and ~11% interpreted dispatch. The
actual string algorithms (substring search etc.) are ~1-2%.

The two "options" converge: the strings fix-list is allocation fixes in
string clothing. One arc, strings-first (cheap, high-visibility,
de-noises the profiles), machinery second (cross-suite).

## 2. Ground truth (the allocation map)

- `Value` is a 16-byte Copy union; Int/Double/Bool/Nil are immediates.
  Everything else is `Gc<RefLock<Object>>` + a payload indirection:
  String/Symbol/Bytes/Block carry a second `Gc`; List/Map/Set carry
  `Gc<RefLock<Box<dyn AnyCollect>>>` (three hops to the Vec).
- Every string = 2 GC allocations (`vm.rs new_string`) + the String's
  own buffer. **String literals are NOT interned**: `materialize_constant`
  clones + double-allocates on every push (symbols DO intern).
- Every interpreted call = 1 GC alloc (`EnvFrame`) + a vars Vec + a
  send-args Vec; every NATIVE call clones the args Vec a second time
  into `active_native_args` purely for rooting (`dispatch.rs`).
- Every `new:` re-derives the ivar-name list (cloning a String per
  field name) and the init-class chain — both static per class,
  memoizable exactly like `field_slots` already is.
- gc-arena forbids reusing Gc allocations (no pooling) and fights
  variable-length inline layouts; `Fields::Inline` (≤3 slots) is the
  existing mitigation. Collection runs only between coroutine resumes
  (`collect_debt` every 10 driver steps; pacing `QN_GC_SLEEP`=4.0).

## 3. Slices

### A1 — string locals (this slice)

1. **Concat without `format!`**: `+:` builds
   `String::with_capacity(a+b)` + `push_str` (typed variant AND the
   untyped fallback's eager receiver clone). Expected: most of the
   ~20% fmt share on strings.
2. **Native linear `List#join:`**: qnlib's `Iterate#join:` is a
   QUADRATIC interpreted `+` loop (Python/Ruby: one linear C call —
   the biggest cross-language asymmetry in the bench). A List-class
   native pre-sizes one buffer; non-String elements still go through
   `.s` dispatch for exact semantics; the Iterate mixin version stays
   for non-List receivers.
3. **String-literal caching**: intern literal Values in a VmState map
   (content-keyed, literals only, bounded by distinct program
   literals). Precondition VERIFIED before landing: strings are
   immutable at the Quoin level and no identity-revealing operation
   exists for them (else sharing is observable and this slice drops).
4. **Small potatoes riding along**: `splitString:` Vec capacity hint;
   `index:`'s second O(n) byte→char pass fused.

Acceptance: strings ≥1.5× (0.195s → ≤0.13s whole-process, interleaved
15-run A/B); every other bench within noise; corpus green ×5 modes.

### A2 — allocation machinery (cross-suite)

1. Kill the native-call `args.clone()` (root the SAME Vec via
   `active_native_args`, hand the callee a borrow — or root via the
   value stack).
2. `exec_send` args: stack-slice window or reusable scratch instead of
   a fresh reversed Vec per send.
3. Memoize per-class instantiation data on `Class` (ivar names — no
   per-name String clone — and the init chain), like `field_slots`.
4. Single-allocation payloads: String inline in `ObjectPayload`
   (drops strings to 1 GC alloc); collapse the collection
   `Gc<RefLock<Box<dyn>>>` triple hop for builtins if the Collect
   plumbing allows.

Acceptance: btrees ≥1.15×; strings/maps/json further improvement;
nothing regresses beyond noise.

### A3 (reassessed — coverage direction resolved, envs remain)

Post-A2 btrees profile: ~43% dispatch/interpreter (its hot methods
refuse compilation), ~11% alloc/GC. The COVERAGE direction was run to
ground in A3a: the config-block write-back false positive is fixed
(init-form binding is now STATIC — `StaticBlock::is_init_literal`,
decision (E)), which un-refused `makeTree` — and compiling it measured
btrees +6.8% SLOWER (the whileDo:/any?: lesson: the arms carry the
recursion, so every node pays a full-frame snapshot materialization
where the interpreter shares a pointer). The recursion gate now covers
all materializations, `^^` or not, and makeTree deliberately stays
interpreted.

REMAINING A3 levers, both own-design arcs:
- **Cheap cold-arm materialization** (hoisted/lazy closures): the ONE
  unlock shared by makeTree, qnlib's `whileDo:`/`any?:`, and S5c
  template-`^^`. Until then, recursive/per-iteration materialization
  correctly refuses.
- **Escape-analysis stack environments** (PERF_ROADMAP Tier 3a):
  attacks the per-call EnvFrame + the interpreted-dispatch share
  directly.
- The `run:`-style sibling refusal (cond/body sharing written cells)
  is a REAL semantics boundary, not a false positive — only cheap
  shared-cell materialization would lift it.

## 4. Doctrine

Same as every perf arc: benches are never rewritten to dodge costs;
whole-process wall time; interleaved `--compare` + same-binary control
for ≤2% deltas; profiling artifacts per task under `profiling/`;
parity corpus green under all five modes each slice; `qn check
qnlib/warnings.qn` canary. Semantic invisibility is a precondition for
literal interning, not a hope.
