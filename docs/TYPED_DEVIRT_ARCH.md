# Typed devirtualization tier (Tier 2) — design

Concrete design for the integrated slice that `docs/FUTURE_ARCH.md` calls Tier 2, following the
Tier-1 ceiling screen (`profiling/unboxed-ceiling/notes.md`), which returned a decisive **GO**: an
unboxed + devirtualized interpreter beats Ruby 2.6 and Python 3.9 on all three tracked benchmarks
(fib ~74×, sieve ~91×, trees ~56× vs Quoin today). This doc is the plan for realizing that in the
*real* VM — reviewed before any VM code is written.

**Targets (the bar we're tracking), from `profiling/unboxed-ceiling/notes.md`:**

| benchmark | today | Python (primary) | Ruby (stretch) |
|---|---|---|---|
| fib(20) | ~18 ms | ≤ 1.0 ms | 0.35 ms |
| sieve(10000) | ~44 ms | ≤ 1.1 ms | 0.72 ms |
| binary trees(10) | ~661 ms | ≤ 83 ms | 32.5 ms |

Tier 2 keeps the interpreter (no JIT) and keeps Quoin's dynamic feel as the **default**; speed is
**opt-in** via type annotations + sealing. Untyped/unsealed code runs unchanged on today's VM.

---

## 1. The reframing the code map forced

`Value::Int(i64)` is **already immediate** — `src/value.rs:164` (the unboxed-integers work removed the
heap box). So for the numeric path there is *no heap allocation to remove*. The cost of `n - 1` today
is:

`Send("-:", 1)` → `exec_send` (`src/vm.rs:2513`) → `lookup_method` + `type_distance` scoring
(`src/dispatch.rs:206`, `:825`) → `Callable::Native` → the native `-:` fn.

That is **~4–5 method dispatches per fib call** (`<=:`, `-:`×2, `+:`, plus the recursive `value:`
sends). The lever is **devirtualization**: when the compiler knows both operands are `Integer` and
`Integer`'s methods can't change, compile `n - 1` to a direct `i64` subtract — no lookup, no
`Callable`, no native-fn call, no tag check.

Storage-unboxing (killing the `EnvFrame` `Vec<(Symbol, Value)>` linear scan, `src/value.rs:576`) is a
*separate, secondary* win for the numeric path (Int already lives in the slot). It matters most for
the **struct/tree path** (Phase 3), where per-node `Gc` allocation is the real cost.

---

## 2. Foundations already in place (verified against the code)

Tier 2 is less green-field than `FUTURE_ARCH.md` implies. Already built:

| foundation | where | what it gives us |
|---|---|---|
| Immediate scalars | `Value::Int/Double/Bool/Nil`, `src/value.rs:164` | i64 already inline in slots/stack |
| **Typed params, parsed + threaded** | `|n: Integer|` → `BlockArgNode.type_hint` (`crates/quoin-syntax/src/ast.rs:71`) → `Block::param_types: Vec<String>` (`src/value.rs:561`), default `"Object"` (`src/compiler.rs:967`) | the type surface + AST + compiler threading for params |
| **Typed params ARE the boundary guard** | `match_score`/`type_distance` (`src/dispatch.rs:531`, `:825`) select `|n: Integer|` *only* when the arg is Integer-assignable | inside a typed method, params are provably their declared type — no extra entry guard needed |
| **`sealed!` implemented** | `Class.is_sealed` (`src/value.rs:683`), `Class#sealed!` (`src/runtime/class.rs:51`), `ensure_not_sealed` blocks all table mutation/subclass/mix (`src/vm.rs:1825`) | the soundness mechanism: a sealed class's method table is permanently frozen; the code comment names it "the intended future trigger for devirtualization" |
| Sealed leaf never invalidates the cache | `invalidate_method_cache` only fires on table mutation (`src/dispatch.rs:347`) | a sealed class is a permanently-stable dispatch anchor |
| Typed **local** syntax — block-header only | `{ | args - x: Integer | ... }` → `BlockDeclNode.type_hint` (`crates/quoin-syntax/src/ast.rs:77`); compiler *discards* the type (`src/compiler.rs:976`) | **ergonomically insufficient** — this only exists in a block *header*, and a block is a value that runs only when `.value`/`.value:` is sent. A method body's own header works, but a *mid-body* typed local means wrapping in a sub-block you must invoke. **Statement-level typed-local syntax is new work** (§4.1, §10). |
| Slot-based fields | `Object.fields: Box<[Value]>` + `Class.field_slots` (`src/value.rs:689`, `:669`) | the shape foundation for typed/unboxed ivars (Phase 3) |
| Method cache | `MethodCacheKey` + `DispatchCache` (`src/dispatch.rs:25`, `src/vm.rs:177`) | the fallback path when devirt doesn't apply |

**Gaps to build:** (a) compiler doesn't propagate types through expressions or use local types;
(b) no devirtualized arithmetic bytecode; (c) no notion of "this builtin is sealed" at compile time;
(d) no typed calling convention; (e) no unboxed struct storage (Phase 3); (f) no ergonomic
statement-level local-declaration syntax, typed or untyped (§10).

---

## 3. The enabling decision: sealed numeric builtins

Devirtualizing `+:` requires proving `Integer#+:` can't be redefined. `sealed!` provides the freeze,
**but** it's a runtime message send while the whole program is compiled AOT (`compile_program`,
`src/compiler.rs:314`) before any user code runs — so the compiler can't query "is Integer sealed?"

Proposed resolution (**decision needed — §10**):

- **Seal `Integer`/`Double`/`Boolean`/`Nil` at VM startup, by default**, right after their native
  methods are registered and before user code compiles/runs. This makes their method tables a
  *language guarantee* the AOT compiler can rely on without a runtime query. It also matches the
  existing "value types are final" stance (the unboxed-integers work already made `@x` on these a
  compile error).
  - **Trade-off:** you can no longer monkeypatch a numeric builtin (`Integer <-- { +: -> … }` becomes
    a `sealed`-class error). This is a real behavior change; arguably desirable, but it's the user's
    call.
  - **Prerequisite (from review):** `qnlib/core/00-bootstrap.qn` currently adds many methods to these
    classes at runtime via `<--`. Sealing at startup forbids that, so those methods must first be
    **moved into the native class registrations** (Rust). Accepted as a one-time migration ("not a big
    loss") that also makes numeric builtins a clean closed native set. Audit `qnlib/` for every
    `Integer`/`Double`/`Boolean`/`Nil <-- { … }` site as part of this; seal only after they're native.
- **Alternative (keeps builtins open):** speculative devirt + a per-class version counter — emit the
  fast op guarded by "Integer's table version unchanged," deopt to a normal send if extended. No
  language change, but it's the first step toward a JIT and adds a guard per op. Recommended only if
  keeping numeric builtins open is a hard requirement.

The rest of this doc assumes the **seal-by-default** path (simplest, sound, no per-op guard).

---

## 4. Architecture

### 4.1 Compile-time type propagation

A bottom-up pass over the AST (in `compiler.rs`) assigns each expression node a `StaticType`, one of:
`Int`, `Double`, `Bool`, `Unknown` (today's dynamic default). Rules:

- Integer/Double/Bool literal → its type.
- A **param** whose `type_hint` is `Integer`/`Double`/`Boolean` → that type (already threaded as
  `param_types`; works today — this is why the fib slice needs no new syntax).
- A **local** with a declared type → that type. This needs **new statement-level declaration syntax**
  (the existing block-header decl section is a value that must be `.value`-invoked — §2), and is the
  natural moment to also **make local declaration explicit and retire the "first-assignment-declares"
  rule** (`src/compiler.rs:707`). See §10 — the user has signalled openness to both. Sieve depends on
  this (its `is_prime`/`i`/`p` are locals); fib does not.
- Arithmetic op on two `Int` subtrees → `Int`; comparison on two `Int` → `Bool`; likewise `Double`.
  Mixed/`Unknown` operand → `Unknown` (falls back to a normal `Send`).
- Anything else (a call result, an untyped local, a field read) → `Unknown`.

This is deliberately a *local, conservative, no-inference-needed* propagation — only annotated locals
and literals seed it. No whole-program type inference.

### 4.2 Devirtualized scalar bytecode

New `Instruction` variants (`src/instruction.rs:98`), emitted **only** when §4.1 proves the operand
types and the operator resolves to a sealed builtin method:

- Integer: `IAdd ISub IMul IDiv IMod ILt ILe IGt IGe IEq INe` — pop two operands, operate on the
  `Value::Int` payloads directly, push the result. **They reuse the existing `Value` operand stack**
  (Int is immediate, so no second stack and no allocation); the win is skipping dispatch + the tag
  check, not storage. Double: a parallel `D*` set.
- These trust their operand types (the compiler guarantees them) — no tag check on the hot path.
- **Semantics must exactly match the sealed native method** (§6): overflow, div/mod-by-zero, and any
  Int→BigInt promotion behavior. This is correctness-critical and is an open prerequisite (§10).

`fuse_bytecode` interaction (`src/compiler.rs:102`): these are non-jump, non-fused ops, so they pass
through untouched by default (the fall-through at `:210`). A later optimization could fuse
`LoadLocal; LoadLocal; IAdd` the way sends are fused today, but that's not needed for the first slice.

### 4.3 The typed/untyped boundary contract

The soundness rule, mirroring Typed Racket's contracts — **guard at the boundary, trust inside**:

1. **Entry (param types): already guaranteed by dispatch.** A `|n: Integer|` method is only reached
   when `type_distance` accepted the arg (`src/dispatch.rs:825`). So `n` is provably an Integer inside
   — *no prologue guard needed*. (This is the single biggest simplifier the code map revealed.)
2. **Call results flowing into typed context:** `.value:(n-1) + 1` — the recursive call returns an
   `Unknown` `Value`. To feed `IAdd`, insert a **guard-unbox**: check `Value::Int`, else raise
   `TypeError`. One tag check per boundary crossing — vastly cheaper than a dispatch. (Slice 2b
   removes even this for sealed-self calls via a typed calling convention.)
3. **Assignment into a typed local:** `x: Integer = <Unknown expr>` → guard-unbox on store. Assigning
   a statically-`Int` expr → no guard.
4. **Exit / boxing:** a devirt result is already a `Value::Int`, so "boxing on the way out" to untyped
   code is free (it never left `Value`). This is why the Integer path is so tractable.

### 4.4 Typed calling convention (slice 2b — closes fib)

To devirtualize the *recursive self-call* in fib (not just its arithmetic), a **sealed** method with
typed params gets a second, specialized entry that passes/returns unboxed values and skips
`lookup_method` entirely:

- Requires a **return type**. fib's `value:` returns `Integer` (both branches do) — inferable by §4.1
  over the method body, or via an explicit return-type annotation (no syntax today — §10).
- The compiler, seeing a self-send `.value:(…)` to a sealed class's typed method, emits a direct
  `CallSpecialized(method_id, argc)` instead of `Send`. The callee's frame reads unboxed params
  directly; the return skips the `Value` round-trip.
- This is the deepest piece and where fib's number approaches the Tier-1 ceiling. **Slice it
  separately** — slice 2a (arithmetic devirt + guard-unbox on call results) already delivers a large,
  measurable, low-risk win on its own.

### 4.5 Unboxed local slots (slice 2c — separable)

Independently of devirt, replace the per-frame `EnvFrame` `Vec<(Symbol, Value)>` linear scan
(`src/value.rs:576`) with **slot-indexed locals** for methods where the compiler can compute a fixed
slot layout (it tracks scopes as name sets already — `src/compiler.rs:230` — so a slot count is a
small addition). New `LoadLocalSlot(u16)`/`StoreLocalSlot(u16)` beside today's Symbol-keyed ops
(kept for closures/dynamic cases). This is the "Step B" deferred in
`profiling/local-var-symbols/notes.md`; it compounds with devirt but is orthogonal and can land in
any order.

---

## 5. Phase 3 (later): unboxed struct types — the tree lever

Binary Trees needs a *different* lever (`profiling/unboxed-ceiling/notes.md`): per-node `Gc`
allocation, not scalar dispatch. Design sketch, built on the same typed+sealed machinery, **deferred
until the numeric path lands**:

- A `sealed!` class with **all-typed ivars** (`| @item: Integer @left: TreeNode @right: TreeNode |`)
  becomes an *unboxed struct*: fixed shape (already have `field_slots`, `src/value.rs:669`), no
  eigenclass, flat typed storage.
- Storage: today `Object.fields: Box<[Value]>` is uniform + GC-traced (`src/value.rs:689`). Unboxed
  typed fields need a representation where scalar fields aren't `Value` and pointer fields are still
  traced — either a per-class packed layout with a trace bitmap, or region/arena allocation for
  sealed-struct instances with bulk free.
- This needs **runtime specialization** (user types are sealed at runtime, not startup), so it can't
  reuse the compile-time seal-by-default trick — a specialization step keyed on the sealed-class
  definition is required. Bigger; scoped as its own phase.

---

## 6. Correctness & GC safety

- **Semantics parity (critical):** each devirt op must be behaviorally identical to the sealed native
  method it replaces — overflow, `/0`, `%0`, and any promotion. Plan: a differential test harness that
  runs a large randomized corpus of `a op b` through both the native send and the devirt op and
  asserts identical results/errors, plus the existing `.qn` suites under both paths.
- **GC:** devirt ops only touch immediate `Value::Int`/`Double` (no `Gc`) — no new rooting concerns.
  New instructions are `#[collect(require_static)]` like all `Instruction`s (`src/instruction.rs:98`).
  Slot-indexed locals (4.5) that hold `Value` must still be traced; unboxed scalar slots (Phase 3)
  must be *excluded* from tracing (the current `Box<[Value]>` is uniformly traced — the hard part of
  Phase 3).
- **Boundary soundness:** the guard-unbox (§4.3) raises `TypeError` (a structured `QuoinError`, per
  the structured-errors work) rather than mis-executing — a typed region can never operate on a
  non-conforming value.
- **Opt-in / backward compat:** with no annotations and unsealed types, §4.1 yields `Unknown`
  everywhere → the compiler emits exactly today's bytecode → zero behavior change. Every existing test
  must stay green with the tier present-but-dormant.

---

## 7. Slices & measurables (measure after each, like the ceiling work)

Each slice is independently shippable and profiled before/after (`profiling/<slice>/notes.md`).

- **Slice 0 — explicit local declaration. ✅ DONE** (branch `experiment/unboxed-devirt`, unmerged).
  `var`/`let` soft keywords + `DeclarationNode` (grammar/AST/parser), strict compiler semantics
  (declare-before-init so recursive closures work; `let` immutability; reserved-ident stores still
  reach the runtime check), formatter support, and a scope-aware codemod (`crates/decl-migrate`,
  throwaway) that added 642 `var`s across 88 `.qn` files + every inline Rust test fixture. Green:
  `cargo test` 21/21, `.qn` 2314/0 incl. `QN_GC_STRESS=1`, `qn fmt` idempotent, `cargo fmt` clean, 0
  warnings. `docs/language/01-foundations.md` §4 updated.
  - **Followup — `new:{}` object-init ergonomics.** Under strict mode a field init whose name isn't an
    enclosing local needs `var` (`new:{ var key='a' }`), while `new:{ item=item }` (name matches a
    param) stays bare — inconsistent. Fix: treat a `new:{}` init block as a special context where
    `field=value` is always field-binding (no `var`, never the undeclared-local error). Small, separate.
  - **Followup — highlighter.** Add `var`/`let` to the syntax highlighter, colored the same as `use`
    (`crates/quoin-syntax/src/highlight.rs`).
- **Slice 2a — arithmetic devirt. ✅ DONE** (`df65763` seal + `e8d726b`). Type propagation +
  Integer devirt ops on statically-Int operands; differential `Devirt` suite. ~30% faster typed fib(30).
- **Slice 2b-A — return types (typed method results). ✅ DONE** (`d7fe17b`). `selector -> Integer { … }`
  syntax; a self-send to a same-class method with a declared Integer return is statically Int, so the
  result `+` devirtualizes. ~4% more on fib(30).
  - **Followup — verify return types.** The declared return type is currently **trusted**: the compiler
    does *not* check that every return point (`^expr` and the fall-through) actually yields the declared
    type. A wrong annotation is *safe* — a devirt op on a non-Int result raises via `pop_two_ints`
    (never UB) — but it fails at runtime rather than compile time. Add a compile-time check that a
    method's return points match its declared return type (this is also the analysis needed to *infer*
    or widen return types). Touches `compiler.rs` (`collect_method_returns` / `self_send_return_type`).
- **Slice 2b-B — devirtualize the calls (Phase 1 DONE; Phase 2 SPIKED & PARKED — dead end).** Turn a
  self-recursive `.value:(…)` in a **sealed** class into a direct call (no `lookup_method`), targeting
  the ~15% dispatch cost.
  - **Decisions:** (1) **A1** — a class is compile-sealed if `sealed!` appears as a *direct*
    (unconditional) statement in its body (reuses the runtime marker; the direct-statement rule keeps
    it sound). (2) **B1** — a guard-free monomorphic call-site cache in the `Block` (sealed ⇒ never
    invalidated ⇒ no guard/epoch — the part that made the general inline cache not worth it).
  - **Scope (first cut):** self-sends to a **same-class method** within the sealed class's own def
    (the compiler knows sealedness + the method set there). Cross-unit sends and class-side/meta
    methods (metaclass sealing) are follow-ons.
  - **Phase 1 — DONE** (`fb67760`): compile-time seal detection + tracking + a behavior-neutral
    `CallSelfDirect` op (delegates to the normal send) for sealed same-class self-sends.
  - **Phase 2 — SPIKED & PARKED (measured net loss).** A per-`(Block, ip)` call-site `Callable` cache
    was **~9% slower** on a sealed-instance-method fib(30) and **never hit** (176/176 misses on
    fib(10)): the `CallSelfDirect` sites live inside `if:else:` blocks, which Quoin **re-materializes
    into a fresh `Gc<Block>` on every call** (each closes over a different `parent_env`). No per-block
    cache can persist for exactly the hot sites, and the only stable key (the shared bytecode `Rc`)
    needs a hashmap — i.e. the FxHash cost we were trying to undercut. See
    `profiling/2bB-csd-cache/notes.md`. This also explains the earlier `experiment/inline-cache` park.
  - **Consequence — CSD dispatch-caching is shelved.** Dispatch is already near the FxHash floor, and
    the tracked Fib/Sieve/Tree benchmarks are class-side (`.meta`) methods on *unsealed* classes that
    never hit `CallSelfDirect` at all. The transient-block finding redirected the real work to
    **Slice 2d** (control-flow inlining), which attacks the fatter, on-benchmark costs. `CallSelfDirect`
    (Phase 1) stays in as harmless plumbing; the old Phase 3 (inline the invocation) is not worth
    pursuing on its own.
  - **Followup — per-method sealing.** Whole-class `sealed!` is coarse; sealing an *individual method*
    (so only that method's dispatch is frozen/devirtualizable while the rest of the class stays open)
    would be finer-grained and preferable. Syntax TBD. Track for after 2b-B lands.
- **Slice 2d — control-flow inlining (v1 `if:`/`if:else:` + v2 `whileDo:` + options B & C DONE; v3 next).**
  Measured **2.0× typed fib(30)** (1.42s→0.71s), **1.5× untyped fib** (B), **~2.5× Sieve** (~40→~16ms),
  **~1.2× Binary Trees** (~545→~454ms, C). All three tracked benchmarks improved. All green: `.qn`
  1233/0 (incl. `QN_GC_STRESS=1`), `cargo test` 209/0. Notes: `profiling/2d-controlflow-inline/notes.md`.
  `if:`/`if:else:`/
  `whileDo:` are ordinary Quoin method sends (`qnlib/core/00-bootstrap.qn`: `True/False` `if:else:`;
  `whileDo:` at :176), so each branch/iteration costs **2 block allocations + 2 dispatches + 2 frames**
  (the `if:else:` method frame, then `.value` on the chosen block) around a single compare/add. This is
  the dominant cost on **all three** tracked benchmarks and hits the exact profiled hotspots (block
  materialization, frame-alloc, dispatch). Lower these forms — **when the argument(s) are literal
  blocks** — to the native `IfJump`/`ElseJump`/`Jump` bytecode the VM already has (the classic
  Smalltalk `ifTrue:ifFalse:`/`whileTrue:` inlining). Per branch/iteration: 2 allocs + 2 dispatches +
  2 frames → 0.
  - **When to inline:**
    - `if:`/`if:else:` — only when the **receiver is statically `Bool`** (2a's `static_type`) **and**
      the block args are literal, 0-arg, **declaration-free** blocks. **Option B (DONE):** comparison
      operators (`<` `<=` `>` `>=` `==` `!=`) are statically `Bool` for *any* operands, so untyped
      `(n<=1)`/`(depth>0)`/`(x==y)` conditions inline too. **Option C (DONE):** an `Unknown`-typed
      receiver (e.g. a predicate send `x.defined?`) inlines behind a runtime `BranchIfNotBool` guard —
      a non-Bool receiver at runtime jumps to a cold path that reissues the real send (MNU / a
      user-defined `if:else:`), so it's fully sound. A known-non-Bool (`Int`) receiver skips inlining.
      Decl-carrying blocks → keep the send (v3, alpha-rename). No regression for dynamic code.
    - `whileDo:` — when the receiver (cond) **and** the arg (body) are literal 0-arg blocks; the cond
      block's truthiness drives the loop.
    - Any non-literal-block arg → fall back to the normal send.
  - **Caret handling (the crux — the block frames disappear):**
    - `^^` (`MethodReturn`) — **unchanged**: still a non-local return to the enclosing method (it
      targets `enclosing_method_id`, which the inlined code is already inside).
    - `^` (`BlockReturn`) — becomes a **`Jump` to the end of the inlined block's region**, leaving its
      value on the stack as the construct's value (for a `while` body, the region-end is the loop-back).
      It is **not** a `MethodReturn`. Only the inlined block's *own* top-level `^` is redirected; a `^`
      inside a deeper, non-inlined block (e.g. a `.each:{}`) keeps normal `BlockReturn` semantics.
  - **Block-local `var`s (the flat-frame hazard):** the VM has one `EnvFrame` per method frame (a flat
    `Symbol → Value` list); a real block gets its own `EnvFrame`, so its `var`s are isolated. Inlining
    removes that frame, so the block's `var`s must bind into the **method** frame. The compiler opens a
    nested compile-time `Scope` (keeps lexical shadowing + 2a types correct), and — to stop the block's
    names from clobbering the method's, or a sibling branch's, same-named locals in that shared flat
    frame — gives inlined block locals **fresh unique symbols** (alpha-rename). Their lifetime stretches
    to the method frame's (a harmless retained slot).
    - **⚠ Debugger caveat (recorded per request):** this var-merging will be **confusing in the
      debugger**. Inlined block locals surface in the *method* frame's variable view (not a separate
      block scope), may carry mangled/alpha-renamed names, and stay visible past their original lexical
      block; and there is no block frame to step into for inlined `if`/`while` bodies (stepping no
      longer pushes a frame). The debugger's variable/scope display and step model will need scope
      metadata to reconstruct the source view over inlined regions — track against
      `docs/DEBUGGER_ARCH.md`.
  - **Slicing:** v1 inlines **declaration-free** control-flow blocks (covers **fib** + **sieve**
    immediately — their bodies only assign to method-level locals). v2 adds alpha-renaming for
    **declaration-carrying** blocks (**tree**'s `makeTree` `if:` declares `var left`/`var right`).
  - **Correctness gate:** the full `.qn` suite stays green (behavior-identical), plus a differential
    test (inlined `bool.if:else:{}` vs. the same via a non-literal block variable) and `^`/`^^` edge
    cases (early `^` in a branch, `^^` through an inlined branch, `^` inside a nested non-inlined block).
  - **Measure:** fib + tree after v1's `if:`/`if:else:`; sieve after `whileDo:`.
- **Slice 2c — unboxed local slots.** §4.5. Kills the `EnvFrame` `Vec<(Symbol,Value)>` scan; compounds
  everywhere (typed or not). Broad + tractable + low-speculation — a strong candidate to do *before* 2b-B.
- **Phase 3 — unboxed structs.** §5. The Binary-Trees lever. Separate investigation.

Honest expectation: Tier 2 keeps real frames + the boundary guards Tier 1 dropped, so the integrated
numbers land *above* the Tier-1 ceilings but — per the 1.4–7× ceiling headroom — plausibly still at or
under the Python/Ruby targets for fib/sieve after 2a+2b.

---

## 8. Risks & de-risking

- **Semantics drift on the fast op** → the differential harness in §6 is a hard gate before merge.
- **Slice 2a underdelivers** if the recursive-call guard-unbox + the still-boxed self-send dominate →
  measure 2a alone; if the number is call-bound, 2b is the fix (already planned).
- **Seal-by-default breaks programs** that extend numerics → surface as a clear migration note; the
  speculative-guard alternative (§3) is the escape hatch if needed.
- **Scope creep into a JIT** → explicitly *not* doing speculation/deopt in the seal path; devirt is
  static, gated on sealing.

---

## 9. Why this is sound *and* stays dynamic

The default stays fully dynamic: unannotated, unsealed code compiles to exactly today's bytecode.
Speed is purchased locally by (a) annotating types and (b) sealing — i.e. explicitly giving up runtime
mutability *where you don't need it*. Dispatch already enforces param types at the boundary, and
sealing already freezes method tables; Tier 2 just teaches the compiler to *exploit* those two
existing guarantees. That is exactly the "gradual performance" sweet spot `FUTURE_ARCH.md` argues for.

---

## 10. Open decisions for you (before implementation)

1. **Seal numeric builtins by default?** (§3) — **DECIDED: yes.** Prerequisite: first migrate the
   methods added to numerics in `qnlib/core/00-bootstrap.qn` (and any other `qnlib` `<--` sites) into
   the native class registrations, *then* seal at startup. Accepted trade-off: numerics become closed
   to `<--`.
2. **Integer overflow & promotion semantics** — **agreed as a hard prerequisite:** the devirt op must
   behave identically to native `Integer#+:` (wrap / error / bignum-promote). Confirm the current
   behavior by reading the native impl before Slice 2a; the differential harness (§6) enforces it.
3. **Return-type annotations** — no syntax exists (`type_hint` is params/locals only,
   `crates/quoin-syntax/src/ast.rs`). For 2b, infer return types from the body, or add
   `-> { |n: Integer| -> Integer … }`-style syntax? (Inference is enough for fib.)
4. **Local-declaration syntax + the "first-assignment-declares" rule** — **DECIDED: explicit
   `var`/`let`, drop the implicit rule** (the clean long-term shape; migration accepted). Spec —
   *confirm the details*:
   - **Keyword-prefixed assignment only.** `var <pattern> = <expr>` (mutable) / `let <pattern> =
     <expr>` (immutable). Declaration and initialization are **one statement** — there is no bare
     uninitialized `var x` (a typed local with no value can't satisfy its type, and it sidesteps
     definite-assignment analysis). Plain `<pattern> = <expr>` (no keyword) becomes assignment-only and
     errors on an undeclared name — this retires the first-assignment rule at `src/compiler.rs:707`.
   - **Types on single-target declarations only:** `var x: Integer = 5`; untyped `var x = 5` →
     `Unknown`. This reuses the exact `name: Type` shape that already parses for params
     (`|x: Integer|`, `block_arg_typed`) — known-good, no new ambiguity.
   - **Destructuring declarations are untyped.** `var`/`let` prefixes the whole assignment and
     composes with Quoin's existing `lvalue+` patterns (space-separated, `*splat`, `_` ignore,
     `(nested)` — `qnlib/presentation/19-assignment.qn`; grammar `Quoin.pest:75-88`): `var a b c = #(1
     2 3)`, `var a *b c = list`, `let first *_ = list`, `var (a b) = pair`. It declares **every**
     `ident_lvalue`/`splat_lvalue` in the pattern as a new (`Unknown`) local; `_`/`*_` ignore as today.
     Error if any declared name already exists in this scope (**no mixed declare-and-assign in one
     statement** — split it). **No `:` type annotations inside patterns** — decided against: `:` is
     already Quoin's keyword-message marker, and typed space-separated targets are hard to scan; the
     perf path never destructures, so nothing is lost. Rebind if a destructured value needs a type
     (`var i: Integer = tuple.at:0`). Revisit later with a distinct delimiter only if a real need
     appears.
   - **`let` immutability** is a bonus that enables registerization / no-reload later
     (`FUTURE_ARCH.md`'s "let vs var" door), and lets the compiler treat a `let`-bound typed local as
     never-reassigned.
   - **Migration = Slice 0:** every existing implicit declaration in `qnlib` + tests gains `var`/`let`
     (mechanical, automatable, `qn fmt`-assisted).
5. **Slice 1 scope** — Integer-only first (fib/sieve), or Integer+Double together? (Integer alone
   covers both target benchmarks.)
6. **Where devirt is allowed** — only inside methods with typed params on sealed classes, or also in
   top-level/typed-local code regardless of the enclosing class? (Affects how broadly 2a applies.)
