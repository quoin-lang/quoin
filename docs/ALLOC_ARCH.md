# Allocation churn: the post-dispatch frontier

*Status: A1 + A2a-c SHIPPED on `perf/alloc-churn` (4 commits). Cumulative
15-run A/B vs main @ `b744e53`: **strings −49% (0.192→0.098s, 1.96×)**,
**btrees −11.5%**, maps −1.7%, richards +1.2-1.4% (at the edge of its A/A
band — watch), rest noise. Remaining: A2d (outcall-path arg windows —
combinators' natives originate from `call_method_cached`, which still
clones), A2e (single-alloc String payload / collection triple-hop
collapse), then the A3 reassessment.*

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

### A3 (reassess after A2, own design)

Escape-analysis stack environments (block-free bodies skip the
per-call EnvFrame — PERF_ROADMAP Tier 3a) vs. btrees compilation
coverage (the sibling-closure-writeback refusal). Sized against fresh
post-A2 profiles; the collector revisit stays deferred.

## 4. Doctrine

Same as every perf arc: benches are never rewritten to dodge costs;
whole-process wall time; interleaved `--compare` + same-binary control
for ≤2% deltas; profiling artifacts per task under `profiling/`;
parity corpus green under all five modes each slice; `qn check
qnlib/warnings.qn` canary. Semantic invisibility is a precondition for
literal interning, not a hope.
