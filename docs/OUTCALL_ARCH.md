# The outcall seam: three fixes on one branch

*Status: F1 + F2 + D1 + D2 SHIPPED on `perf/ic-direct-calls` (base
`449dc84`). Cumulative: **btrees −13.8%, richards −13.9%, combinators
−7.8%**; maps +3.0% = static code layout, proven by a same-binary
fast-path-off shim measuring maps flat and btrees −15.3% (notes.md).
F1: List/Map/Set/Bytes class `new` constructs real natives, `new:`
errors clearly. F2: deferred-nil locals are entry-nil (in-loop decls
re-nil at the site); `sum`/`reduce:` promote — the suite's refusals
are down to the two correct `whileDo:` trampolines. D2 disciplines
learned: receiver-phase peek before lane decode; site id in the ip
lane's high bits (a 13th ABI arg taxes every call); fills once per
epoch (polymorphic thrash); no-site sentinel for devirt native
fallbacks. D3 remains the recorded future arc. Corpus 1638/0 ×5.*

## Why: the measured shape

Post the alloc-churn + materialization arcs, btrees and richards share
one profile: **~29-35% in the compiled-to-compiled outcall seam**
(`Callable::call` 13.5/20.4%, `call_method_cached` ~6%, `outcall`
~5%, `ic_probe` ~4.5%, plus registry/hash lines). btrees does ~4M
outcalls in 0.41s — roughly 30ns of seam per call against the ~2-5ns
a direct call costs (the S2 same-group direct call already proves the
floor: it is why fib is 40×).

Anatomy of ONE warm compiled→compiled outcall today
(`helpers::outcall` → `call_method_cached` → `Callable::call` AotCall
arm → `codegen::invoke`):

1. decode receiver + args from lanes into a fresh `Vec` (heap alloc);
2. `outcall_nesting` ±1; `ic_cell_by_id` — an FxHashMap lookup in
   `ic_registry` EVERY call; `ic_probe` (borrow + epoch/shape guards);
3. push `[receiver, args…]` onto `vm.stack` (the A2c rooting window);
4. the AotCall arm: nesting-cap check, spec-precondition scan,
   `active_native_args` push/pop (rooting the window that is already
   rooted by being the stack), outcome match;
5. `invoke`: entry gates, arity, `needs_list_self`, push receiver +
   obj args onto the stack AGAIN (the callee's slot window), scratch
   nils, a fresh `raw: Vec<i64>` (second heap alloc), `HomeCtx`,
   `run_in_frame_ctx` (2-3 `mem::replace` swaps + frame-mark
   bookkeeping), the raw call, `outcome_from_tag`, `finish_frame`.

## Slices

### F1 — `List.new` / `Map.new` / `Set.new` broken shells (correctness rider) — SHIPPED

`Callable::NewNoBlock`/`Callable::New` are the `lookup_method`
FALLBACKS when no user `new`/`new:` exists — so the generic path mints
a payload-less Object shell of class List, and the first native method
call dies with the internal "Not a native state" error
(QUOIN_TODO.md, found during M1 testing). Fix: native CLASS methods on
the three collection classes, which win the hierarchy lookup before
the fallback (exactly how `List.of:` already registers):

- `new` → the real empty native collection (`#()` / `#{}` / `#< >`).
- `new:` → a clear, catchable error ("List has no instance fields —
  use `#()` / `List.of:`"): a config block on a native collection is
  meaningless, and silently minting a poison object is the one wrong
  behavior. (List/Map/Set are sealed, so no subclass complications.)
- Rider sweep: audit other NativeState-payload builtins reachable via
  `.new` for the same trap; fix or error them the same way.

Tests: `List.new.add:1` roundtrip + Map/Set equivalents; `new:` error
pinned; the M1-era `gen.qn` guardrail shape (closures into
`List.new`-built lists) now passes as written.

### F2 — deferred-nil locals: entry-nil semantics (un-refuses `sum:`/`reduce:`) — SHIPPED

Today `var x = nil` DEFERS (no slot; "type decided at first store",
`nil_deferred`). Two problems: a READ before any store has no slot and
refuses the method ("read of unknown/uninitialized local" — qnlib's
`reduce:`, the suite's last coverage refusal); and the force that
materialization performs emits `slot_set(nil)` AT the site, which
inside a loop would re-nil a live accumulator (the reason M3's
cold-span exemption carries a `nil_deferred.is_empty()` condition).
There is also a LATENT divergence: an in-loop `var x = nil` whose
DefineLocal deferred emits nothing, so iteration 2 sees iteration 1's
value where the interpreter re-nils.

Design, using the fact that `invoke` already pushes `n_scratch` slots
initialized to NIL — scratch slots are entry-nil by construction:

- Deferral prescan: a `Push(Nil); DefineLocal(x)` whose ip sits inside
  a LOOP SPAN (the existing backward-jump machinery) does NOT defer —
  it takes a slot up front and emits `slot_set(nil)` at the decl site
  (per-iteration re-nil is the correct semantics; fixes the latent
  divergence).
- A non-loop deferred var, when forced — by a READ (the new case: the
  `local_av` None arm allocates the slot and just returns it) or by a
  materialization — takes a scratch slot with NO site init: the slot
  is already nil at entry, and the decl executes at most once before
  any read (a bytecode-order read before the decl compiles as
  LoadGlobal, never `local_av`).
- The `slot_set(nil)` in the materialize-force is then redundant and
  dropped, and M3's `nil_deferred.is_empty()` condition on the
  cold-span exemption is dropped with it — uniformly safe now.

Acceptance: `sum:`/`reduce:` compile (the suite's refusal count goes
to two: the two correct `whileDo:` trampolines); combinators measured
(reduce feeds `sum`); a semantics pin for the in-loop re-nil shape,
verified against the interpreter.

### D1 — lean seam (mechanical, measure before D2) — SHIPPED

No dispatch change; remove the itemized waste on the warm path:

- decode args into a fixed `[Value; MAX_OUTCALL_ARGS]` window pushed
  straight onto `vm.stack` — no intermediate `Vec`;
- `raw` lane buffer as a fixed array — no second `Vec`;
- ONE window: `call_method_cached_inner` builds the window in
  `invoke`'s layout (receiver, obj-args) so `invoke` reuses it instead
  of pushing a second copy;
- skip the `active_native_args` push for windowed AotCall — the window
  IS the rooted stack; keep the error-snapshot path working off the
  window directly.

### D2 — per-site AOT entry cache (the "AOT IC") — SHIPPED

A side-table cell per compiled outcall site `(tid, ip)` — plain
`'static` storage like the leaked selector/name tables, NOT Gc —
holding `(dispatch_epoch, recv_guard(kind, ptr), entry: &'static
AotEntry)`. The outcall helper checks it before anything else: on hit
(epoch live + receiver guard matches) it enters a slim invoke —
skipping `ic_cell_by_id`'s registry hash, the `ic_probe` borrow, the
`Callable` dispatch match, and the per-call gate re-derivation (the
epoch+guard subsume `entry_gates`' redefinition half; spec
preconditions still check via the entry, they are per-call by design).
Miss → today's full path fills the cell. Invalidation = the epoch, as
everywhere.

Acceptance for D1+D2 together: btrees and richards ≥8% each (the seam
is 29-35%; the target is roughly halving it), everything else within
noise; corpus ×5 per slice; canary healthy.

### D3 (recorded, likely its own arc) — native direct inner calls

Detailed implementation plan: **docs/DIRECT_CALLS_ARCH.md** (D2.5
interior specialization + the D3 direct-call tier: warmth-triggered
retranslation, baked guards, windowless-first tiering).

## Doctrine

As every perf arc: whole-process wall time; interleaved 15-run
`--compare` per slice (quiet re-run authoritative); artifacts +
binaries in `profiling/ic-direct-calls/`; corpus green ×5 modes per
slice; `qn fmt` on any .qn change; `qn check qnlib/warnings.qn`
canary; benches never rewritten to dodge costs.
