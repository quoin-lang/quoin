# Cheap materialization: fusion first, thin closures second

*Status: M1 SHIPPED on `perf/cheap-materialization` (stacked on
`perf/alloc-churn`, PR #59) — measured btrees −31.8% (1.47×), richards
−14.4% (1.17×), json −2.6%, rest noise; corpus 1611/0 ×5 modes. The
alpha-renaming lives in `Compiler::declare_local`/`local_symbol`
(splice scopes) + `devirt.rs` `spliceable_arm`/`splice_hazard_free`;
shape pins in `src/compiler/tests.rs`, semantic pins in
`qnlib/tests/45-controlflow-inline.qn`. M2/M3 not started. Baselines +
experiments in `profiling/cheap-materialization/`.*

## 1. Why: the measured shape

The alloc-churn arc (docs/ALLOC_ARCH.md §A3) ended on a data point: with
the recursion gate lifted, compiling btrees' `makeTree` made the bench
**+7% SLOWER** (re-reproduced this pass: 0.95s → 1.02s interleaved
10-run). Every tree node paid a full-frame snapshot materialization
where the interpreter's closures are a pointer share. The same
economics is why qnlib's `whileDo:`/`any?:` sit behind trampoline gates
(sieve 5.8×, combinators +60% when compiled naively) and why S5c
template-`^^` is blocked.

The design pass root-caused WHERE those materializations come from, and
the answer moves the arc's center of gravity:

**`makeTree`'s `if:else:` arms never fuse.** The compiler's
control-flow inlining is v1: *"Every arg must be a literal, 0-arg,
declaration-free block"* (`src/compiler/devirt.rs:171`,
`inlinable_block` — "declaration-carrying blocks need alpha-renaming, a
follow-up"). `makeTree`'s then-arm declares `var left`/`var right`, so
the whole construct compiles as a real `if:else:` send with
`Push(Block)` arms. The AOT tier then materializes BOTH arm closures
per node (each: `make_closure` = 2 GC allocs, + one `closure_bind`
helper call per frame binding), outcalls `if:else:`, and the winning
arm — carrying the recursive sends and the config literal — runs
INTERPRETED. The compiled function performs exactly one native
comparison. `run:`'s two `whileDo:` sites have the same disease (`var
iterations/i/t` in the bodies), which is also what produces its
sibling-shared-captures refusal.

**Validation, both directions** (dirty-tree experiments, artifacts in
`profiling/cheap-materialization/notes.md`):

- Gate lifted, shape unchanged (arms unfused): btrees 0.95 → 1.02
  (+7%). Profile: `closure_bind` appears, GC +1.1pt, seam churn
  (`start_block_as_method` ×1.7, `dispatch_one` grows).
- Shape fixed by hand (declarations hoisted to method scope in a
  SCRATCH copy — arms become declaration-free, v1 fusion applies,
  `makeTree` promotes under the UNMODIFIED gates): btrees **0.91 →
  0.62 = 1.47×**.

So fusion coverage is worth ~1.5× on btrees today, and cheap
materialization proper is the follow-on that attacks the residual:
in the hoisted profile, `closure_bind` 3.6% (the config-literal
snapshot: self + item + depth + left + right per node) and the
interpreted-config seam (`Callable::call` 9.7%, `call_method_cached`
6.4%, `outcall` 4.6%, `run_nested` 2.4%).

## 2. Ground truth

- Compiled-side materialization (`src/codegen/translate.rs`
  `materialize_closure` ~1883, helpers `make_closure`/`closure_bind` in
  `src/codegen/helpers.rs:368-424`): fresh `EnvFrame` chained to
  `vm.aot.enclosing_env` + `new_block` (2 GC allocs beyond the env),
  then one `closure_bind` per frame binding — EVERY local, obj param,
  and `self`, captured or not. Written frees create write-back
  obligations flushed after the consuming send (`flush_writebacks`);
  obligations are keyed by SSA id and refuse any escape
  (`refuse_tracked_escape`).
- Interpreter closure creation (`src/vm.rs:3134-3161`): Rc bump + 2 GC
  allocs (Object + `Gc<Block>`), `parent_env` = pointer to the LIVE
  frame env. Zero copies, zero binds, shared mutable cells by
  construction.
- The materialization gates (all `translate.rs`): G3 per-iteration `^^`
  in a fused-loop span; G4 own-selector recursion (makeTree); G5
  written param/self (no writable home); G6 sibling closures sharing
  written captures (run:, unfused whileDo:); G8/G9 obligation-carrying
  escape. G5/G6/G8 exist BECAUSE snapshots diverge from live cells —
  they are consequences of the snapshot model, not independent facts.
- Config literals (`is_init_literal`, static (E) semantics): stores
  bind locally (field names), captures are read-only, no `^^` — they
  are obligation-free BUT still pay the full snapshot, still run
  interpreted (barred from B3a templates), and still ride the `new:`
  outcall seam (`start_block_for_instantiation` frame + `run_nested`).
- Control-flow inlining v1 (`src/compiler/devirt.rs`):
  `try_compile_inlined_conditional` / `try_compile_inlined_while`
  require literal, 0-arg, DECLARATION-FREE blocks. `inline_block_body`
  already handles `^` (jump-patched) and leaves `^^` untouched.
  Splicing a declaring block would collide same-named siblings in the
  method scope — hence the v1 restriction.

## 3. Slices

### M1 — alpha-renamed control-flow fusion (compiler; both tiers win) — SHIPPED

Extend `inlinable_block` v1 → v2: a literal 0-arg block WITH local
declarations fuses by renaming each block-local declaration to a fresh
source-unspellable name, applied to the declaration and every
reference within the arm — including free references from nested block
literals that survive as literals (capture-avoiding: a nested
re-declaration of the name stops the substitution).

Hazards, named:

1. **Init-literal store targets are field names.** In `new:{ left=left
   }` the STORE target binds the field; only the RVALUE read renames
   (`left=left·1`). The compiler knows init position
   (`next_block_is_init`), so the exemption is mechanical — but it must
   be tested with shadowing both ways.
2. **Binding-generation capture.** Splicing hoists a per-execution
   block binding into the method frame. Observable only when BOTH: the
   construct re-executes in the same frame (it is a loop body / sits
   under a fused loop span), AND a surviving nested literal captures a
   renamed local (iterations would share one cell where they minted
   generations). Rule: refuse v2 renaming in exactly that conjunction.
   A recursive `will_fuse` AST predicate lets provably-fusible inner
   constructs not count as "surviving literals", so run:'s whole loop
   nest fuses bottom-up (inner body has no literals; outer body's only
   literals are the inner cond/body, which fuse).
3. Renamed locals are visible to the debugger/eval-in-frame under
   internal names. Accepted for v1 of this slice; recorded, not hidden.

Expected coverage: btrees `makeTree` (arms fuse; the config literal's
nest no longer sends the own selector, so G4 never fires — no gate
change needed) and `run:` (loop nest fuses; the G6 refusal becomes
moot for it). Interpreter-tier programs win too: fused loops drop the
qnlib trampoline (2 frames + 2 sends per iteration) even under
`QN_AOT=0`.

Acceptance: btrees ≥1.3× vs branch base (validated projection: 1.47×);
every other bench within noise; corpus green ×5 modes; `qn check
qnlib/warnings.qn` canary healthy.

### Post-M1 reassessment (measured; supersedes the M2/M3 plan below)

The M1 profiles re-rank the levers:

- **btrees residual** (653 samples): the interpreted-config seam is
  ~35-40% — `Callable::call` 8.9 + `dispatch_one` 8.3 +
  `call_method_cached` 6.3 + `ic_probe` 4.4 + `run_nested` 3.1 +
  `collect_instance_vars` 2.3 + `finalize_instantiation` 2.1 +
  `store_set_local` 1.4 + frame starts ~2.4. The env-home's target —
  `closure_bind` — is down to 3.4%, ALL of it the config snapshot.
- **richards residual**: compiled-to-compiled outcall dispatch
  (`Callable::call` 20.4%, IC machinery ~15%), zero materialization.
  Not this arc's lever (future: direct calls for IC-stable sites).

So the arc pivots: **M2 becomes FUSED INSTANTIATION** (the config seam,
btrees' dominant cost), and **env-home is DROPPED** — once configs stop
materializing, btrees' `closure_bind` goes to ~0 and no bench has a
measurable materialization residue left to justify the machinery.

### M2 (revised) — fused instantiation (compiler superinstruction; both tiers win)

`X.new:{ f1=e1; …; fn=en }` on the plain-shape path compiles to a
guarded dual form, exactly the option-C/`each:` pattern:

    <receiver>
    BranchIfNotPlainNew(→cold)     // new IC kind: (class_ptr, epoch) → verdict
    <e1> … <en>                    // field rvalues inline in the METHOD frame
    NewWithFields([f1…fn])         // new_object + bind fields by name + push obj
    Jump(→end)
    cold: Push(Block(config)); Send(new:, 1)
    end:

The check runs BEFORE the rvalues evaluate (the cold path re-evaluates
them inside the real config closure — no double evaluation). The
verdict is cached per site like the field-slot IC: `new:` on this class
resolves to `Callable::New` (a user meta `new:` → cold — the evil-new
case), `ensure_instantiable` passes, and the memoized `InitPlan` (A2a)
has NO init methods (classes with `init:`/`init` → cold, v1). Hot path
kills, per instantiation: the closure materialization (AOT tier), the
config `Frame`+`EnvFrame`, the nested `run_nested` drive, the
interpreted stores, and the `collect_instance_vars` walk — replaced by
inline expr evaluation + one helper doing `new_object` + n field
writes. The AOT tier translates the same bytecode naturally (exprs are
ordinary inline code; the branch + `NewWithFields` become helper calls
with a stack-window root, A2c pattern).

Compile-time eligibility (AST, in `compile_method_call`; anything else
keeps today's form): the arg is a 0-arg, header-decl-free init literal
whose top-level statements are ALL single-target plain assignments to
bare lowercase names; no `Declaration` statements; rvalues contain no
`self`, no `@field`, no bare `.sends` (config `self` IS the new
object), no `^^`/`^>`, no nested block literals (a literal's captures
of config-bound names resolve differently without the config frame),
and no read of a name a PRIOR statement in the same config stored
(read-after-store sees the config-local binding today when the name
shadows an outer local). Exception mid-evaluation is equivalent either
way: today the half-populated frame is discarded unfinalized; fused,
the object was never allocated.

Acceptance: btrees ≥1.25× further (the seam is ~35-40%); interpreted
tier (`QN_AOT=0`) improves too — the fusion is bytecode-level; corpus
×5 incl. an evil-`new:` + init-carrying-class + non-class-receiver
cold-path battery; every other bench within noise.

### M3 — gate-lift audit (unchanged, after M2)

Re-audit G3/G4 and the qnlib trampoline gates against post-M2 profiles;
S5c template-`^^` revisit; cross-language re-measure at arc close.

### ~~M2 — thin materialization: the frame env-home (codegen)~~ (DROPPED post-M1 — kept for the record)

For compiled frames that still contain materialization sites: allocate
ONE `EnvFrame` at frame entry (the env-home) holding exactly the union
of cells that any materialized nest in the method captures (incl.
self/params where captured). Those cells' native accesses go through
the env-home (fixed bind order at translation → known slot index →
direct indexed load/store; symbols kept for interpreter interop).
`make_closure` becomes: thin Block whose `parent_env` = env-home — 2 GC
allocs, ZERO binds, exactly the interpreter's model.

Consequences, in order of importance:

- Write-backs die for env-homed cells: the cell IS shared, semantics
  exact by construction.
- G6 (siblings) becomes CORRECT rather than refused — sharing is the
  point. G5 (param/self writes) gets a writable home. G8 escapes carry
  no obligations to orphan. Each lift is audited + A/B'd separately.
- G3/G4 profitability re-derived with thin cost (the 5.8×/60%/+7%
  numbers were all full-snapshot numbers).

Env-home chains to `vm.aot.enclosing_env`; interacts with `want_home`/
NLR frame marks; hangs off the frame (not `VmState`) so task swaps are
untouched. Cost shift: per-access indirection on env-homed cells vs
per-materialization binds — env-homed cells are by construction the
ones blocks capture, i.e. accessed across a send boundary anyway.

Acceptance: btrees additional gain (`closure_bind` ~3.6% + write-back
sites); btrees `run:` compiles (G6 lift) and measures ≥ neutral;
combinators/richards ≥ neutral; corpus ×5.

### M3 — gate-lift audit + the config seam (stretch)

- Re-audit G3 (per-iteration `^^` arms) and G4 (recursion) under thin
  materialization, one A/B each; revisit the qnlib `whileDo:`/`any?:`
  trampoline gates and S5c template-`^^`.
- Init-literal templates: post-(E) a config block's stores bind
  locally, so the B3a bar is defensive, not semantic. Compiling
  configs (with `start_block_for_instantiation` parity —
  `instantiating_obj` in compiled code) kills the remaining
  interpreted-config seam (~5-8% of hoisted-btrees). Sized as its own
  slice if the M1+M2 profile still shows it.
- Arc close: re-measure the cross-language matrix (bench/CROSS.md).

## 4. Doctrine

Same as every perf arc: benches are never rewritten to dodge costs (the
hoisted-btrees copy was a SCRATCH validation artifact, not a bench
change); whole-process wall time; interleaved 15-run `--compare` for
landings (quiet re-run authoritative); profiling artifacts + matching
binaries per task under `profiling/cheap-materialization/`; parity
corpus green under all five modes each slice; `qn fmt` on any .qn
edits; the `qn check qnlib/warnings.qn` canary each slice.
