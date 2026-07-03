# Type System — round-out plan

Actionable plan for evolving Quoin's static types from a *targeted optimization aid* into a real
*gradual type checker* with good error ergonomics — while keeping the dynamic-by-default feel. Branch:
`experiment/type-system`. Companion to `docs/FUTURE_ARCH.md` (the two converge — see "Synergy").

## Where it is today (grounding)

Types already do real work, but **at runtime, for two non-checking purposes**:

- **Multimethod dispatch.** Param types (`|n: Integer|`) select a method *variant* by the argument's
  runtime type (scored by type-distance). Full class types. A mismatch is a **runtime** MNU, not a
  compile error. (Because dispatch guarantees the param type, the body needs no runtime guard —
  compiler.rs "the param is provably that type… no runtime guard needed".)
- **The optimizer.** A tiny **4-value lattice** — `StaticType { Int, Bool, List, Unknown }` — propagated
  forward at compile time to decide when to emit a devirtualized op.

And critically: **the compiler emits zero type errors.** Annotations are optimizer hints + dispatch
selectors; a wrong annotation just fails to optimize or fails later at runtime. There is no *type
checker*. Un-annotated block args eagerly default to `"Object"` (the dispatch catch-all). Good error
*rendering* (source span + caret) exists for parse/runtime errors, but there is no type-diagnostic
channel.

**The reframe:** the runtime already understands rich types; the compile-time side is a thin,
un-checking shadow. Rounding out = lift compile-time understanding to what the runtime already knows,
and add the checker.

## Goal & stance

Round out into a **gradual, best-effort type checker**: catch type bugs at compile time, with good
ergonomics, opt-in, never nagging on dynamic code.

- **Best-effort first** (TypeScript-style): check where types are written, trust dynamic code, add no
  new runtime machinery. The optimizer already gets soundness from dispatch + per-op fallback, so the
  checker does **not** need to be sound for perf.
- **Sound-gradual deferred** (Typed Racket-style boundary contracts): add later, only where hard
  guarantees are wanted.

## Decisions from the design discussion

1. **Un-annotated params are `Any`, not `Object`.** Stop eager-defaulting to `"Object"`
   (compiler.rs ~2437). Keep the annotation optional (`None`) and **decouple the two readings of
   "absent"**:
   - *Dispatch* treats absence as the **catch-all** variant (unchanged runtime behavior).
   - *The checker* treats absence as **`Any`** — gradual, unchecked → no false errors on dynamic code
     (e.g. `{ |x| x.customMethod }` must not error).
   `Object` (restrictive top class) and `Any` (gradual escape) are **distinct types**. Explicit
   `|x: Object|` → the restrictive top class (revisit if that proves annoying). **General principle:
   eager defaults that serve the runtime become lies to the checker — audit others (return types,
   field types, collection elements) for the same trap.**
2. **Defer "suggest the fix" (did-you-mean).** Ecosystem/method-surface too small to be worth the
   fine-tuning, and a *wrong* suggestion is worse than none. Revisit when the ecosystem is larger.

## Settled surface syntax

Three type-syntax decisions, locked before building the parser/resolver around them:

**Nullable — `Integer?`.** `?` is an identifier character (so `nil?`/`empty?` are single tokens),
which means `Integer?` lexes as *one* identifier. So nullability is a **resolver** rule, not a
grammar change: a type-position identifier ending in `?` → `Nullable(base)`. Unambiguous because
class names are PascalCase while predicates are lowercase. No space (`Integer ?` is not it).

**Generics — `Class(args)`, space-separated.** `List(Integer)`, `Map(String Integer)`. `<…>` is
ruled out (`<`/`>` are operators + `<-`/`<--`/`->`/`-->` arrows, plus the `>>` nesting problem);
`[…]` is namespaces; `{…}` is blocks. A bare `ident(…)` is unused (sends are `.sel:`), so parens
are free, delimited, and nest cleanly: `Map(String List(Integer))`.

**Block signatures — `Block(args… ^Ret)`, and `^Ret` moves into the block header.** A function
type needs both args and a return, and a flat list can't tell `Block(Integer Integer)` (two args,
`Any` return) from a one-arg/one-return reading. The fix: mark the return with `^`, reusing
Quoin's return operator. So `^` means "the return" in three positions — statement (`^ expr`),
header annotation (`|a ^Ret|`), and type slot (`Block(… ^Ret)`). This makes **a block's type its
header with the names stripped**:

```
{ |a:Integer b:Integer ^Integer| … }   ⟺   Block(Integer Integer ^Integer)
```

Consequences: `->` is de-overloaded back to just the method arrow (`sel -> { … }`); the return
type moves out of `-> Ret` into the header as `^Ret`; **a bare (non-method) block can now declare
its return type** (`{ |x ^Integer| … }`), which `-> Ret {}` couldn't reach. No `^` ⇒ `Any` return;
`Block()` = zero args / `Any`; `Block` (no parens) = fully unconstrained. `^` is single (the
block's own return), never `^^` (that's the non-local return, a control-flow marker, not a type).
`^Ret` sits after the args, before the `-` local-decl separator; and last in `Block(… ^Ret)`.

## Work plan (sequenced)

### Phase 0 — migrate the return-type syntax (do first)
Move the return type from `sel -> Ret { |args| }` (Slice 2b-A) to `sel -> { |args ^Ret| }`, per
the settled syntax above. Touches: the pest grammar (drop `ret_type` after `op_meth`; add
`block_ret = "^" ident` to `block_decls`), the AST (`return_type` moves from the method nodes onto
`BlockNode`), the parser + compiler (`collect_class_ctx` reads `m.block.return_type`), the
highlighter, the ~4 qnlib/test sites, and the IntelliJ plugin. Mechanical and small; done before
Phase 1 so the resolver/checker build on the final location.

### Phase 1 — the real `Type` representation (foundation) ✅ DONE
Landed in `src/types.rs` (`Type` enum + `Type::from_annotation_name`); `compiler.rs` swapped
off `StaticType`. Behavior-preserving — the devirt gates still act only on `Int`/`List`/`Bool`
and treat every other type (`Any` included) as "no static knowledge", so codegen is byte-identical.
`Instance` uses the class **name** (`Arc<str>`), not a numeric `ClassId` (no class registry yet).

Replace `StaticType{Int,Bool,List,Unknown}` with a proper `Type`:
- **Builtins**: `Int`, `Double`, `Bool`, `String`, `Nil`, `List`, `Map`, `Set`, `Block`.
- **User class types**: `Instance(ClassId)`.
- **Nullability**: `T?` (union with `Nil`) — Quoin has `nil`, so this matters a lot.
- **`Any`** (gradual escape) — DISTINCT from `Object` (the top class).
- **`Never`** (bottom).
- Later: generics (`List(T)` / `Block(args… ^Ret)` — see Settled surface syntax), general unions.

This is the shared substrate for both the checker and the optimizer.

### Phase 2 — resolver ✅ DONE
Resolve annotations → `Type` against a real known-class set and flag unknown names. Landed as
`Compiler::resolve_annotation` + a `SeenTypes` accumulator (`src/types.rs`); un-annotated → `Any`
(the `"Object"` default is now only the runtime *dispatch* signature, decoupled from the static type).

Decisions (forced by the investigation — classes are compile-time-invisible across units, since the
runner compiles the prelude and each `use` in a *separate* `Compiler`; the VM class table isn't
reachable at compile time):
- **Non-fatal warnings**, not errors — an unknown type prints `warning: unknown type Foo` to stderr
  and still lowers/runs (gradual best-effort; also the diagnostics substrate Phase 3 needs).
- **Shared "seen types" accumulator** — one `SeenTypes` (`Rc<RefCell<HashSet>>`) rides on `VmOptions`,
  threaded into every `Compiler` the run spawns (the VM's `use`-loads *and* the runner's top-level
  program), plus a per-unit top-level pre-scan and a record-on-definition hook (catches nested defs).
  So a unit sees the classes earlier-compiled units (prelude, imports) defined — no false positives on
  stdlib types. Residual gap: a class the program itself `use`s (loaded during its *own* run) is unseen
  at its compile → a non-fatal warning.

### Phase 3 — checker pass (best-effort, gradual; **interleaved** into the compile pass)
Bidirectional (check against annotations where present, infer where absent), gradual (never speak on
`Any` or an unknown class), non-fatal warnings on the `diagnostics` channel. Staged:

**3a — self-contained checks ✅ DONE** (VM `ca76d3e` + `65d8557`). `Type::compatible_with` (strict —
signatures never widen) + `static_type` extended to synthesize all literal types.
- **Return type**: a block/method's tail and `^`/`^^` returns checked against its declared `|args ^T|`.
- **Typed decl**: `var x: T = expr` resolves `T` (also flags unknown types in decls), checks the
  initializer, and records `T`.
- **Numeric promotion is value-level, not type-level**: an `Integer` *literal* where a `Double` is
  expected is emitted as a `Double` (`^Double { 1 }` → `1.0`); a non-constant `Integer` → warning.

**3b — cross-class checks ✅ DONE** (VM `54be965` … `8e0b8ad`). A parallel `ClassTable`
(`src/class_table.rs`: name → {parent, mixins, own selectors, sealed, per-method param types}), threaded
like `SeenTypes`, populated from the current-unit AST **+ `introspect::describe_class`** for VM-resident
classes (reuses the `$inspect` extraction; VM sigs are `from_vm` = authoritative — they include native
methods + applied `Foo <-- {}` extensions). Resolution (`responds_to`) walks the *exact* dispatch order,
so no drift; the corpus (0 false positives on thousands of real sends) + a checker-vs-VM cross-check test
are the anti-drift guards.
- **`Instance` subtyping** — in `check_type` via the parent/mixin chain (only ever *removes* false positives).
- **Compile-time MNU** — a send to a selector the receiver's class can't answer.
- **Argument-type checks + promotion** — args checked/promoted against the method's param types.

MNU and arg-checks are gated on **`from_vm` + `sealed`** (an open class could gain the method/overload, so
staying silent there is sound); missed check = fine, false positive = not. Inline-block-args still deferred.

**3c — flow-sensitive type narrowing (nil-first, generic framework).** The hardest slice; needs real
flow analysis to avoid false positives. Built as a **general refinement layer**, not a nil special case.

*Core decision.* A per-program-point overlay maps a **narrowable path** (`Local(name)` or `Field(@name)`)
→ refined `Type`, laid over the flow-insensitive `types`/`declared_types` scope maps. The **mechanism is
type-generic** (any `Type` refinement); only the initial **rule set** is nil-specific. `static_type` /
`local_type` consult the overlay — a narrowed key's type wins.

*Guard grammar (from a corpus survey — the guard is a syntactic shape, not one operator).* `.defined?` is
a plain Bool-returning method (`true` on any object, overridden `--> false` on Nil), composed with the
`.if:`/`.else:` sends. So the true-arm narrows to **non-nil** (reverse polarity of a `nil?` check):

| idiom | narrows |
|---|---|
| `RECV.defined?.if:{A} else:{B}` | `A`: RECV non-nil · `B`: RECV nil |
| `RECV.defined?.else:{B}` | `B`: RECV nil; if `B` diverges (`^^`/`^`/throw) → RECV non-nil *after* |
| `RECV.defined?.if:{A}` | `A`: RECV non-nil |
| `RECV == nil` / `!= nil` as the condition | polarity-flipped |
| `RECV.defined? && EXPR` | `EXPR`: RECV non-nil (short-circuit) |

`RECV` is a local *or* a `@field` (the corpus narrows fields heavily). The condition matching keys off the
**AST shape**, hooked at the existing `try_compile_inlined_conditional` site — but *independent of the
devirt-inline gate* (that needs a statically-Bool receiver; `x.defined?` types as `Any` today).

*Two surfaces.* (1) **Read side (narrowing)** — the overlay above. (2) **Use side (the payoff):** a
non-nil-safe send to a *confidently* `Nullable(T)` receiver → `warning: receiver may be nil`. Nil-safe
allowlist: `defined?`, `==`, `!=`, `s`, `pp`, `class`, `hash`. **Gated to explicit `T?` or a
narrowed-nullable**, silent on `Any`/unknown — so it speaks only on code that opts in by annotating `T?`.
The corpus annotates nothing `T?` yet, so the misuse check is **silent on today's corpus by construction**;
the corpus's role here is to prove *narrowing* adds no regressions.

*Slices.*
- **3c·0 — representation + locked grammar.** Overlay + `NarrowKey`; wire `static_type`; lock the grammar
  above. No checks; corpus unchanged. Settles the open questions below.
- **3c·1 — arm + divergence narrowing (Tier 1, the 80%).** Recognize the shape; compile arms with
  refinements; post-guard narrowing when the nil-arm diverges (`defined?.else:{ ^^… }`). Reassignment /
  field-write **widens**. No user warning yet; validate via corpus + narrowing unit tests.
- **3c·2 — the nil-misuse check (payoff).** Warn on non-nil-safe sends to a confidently-nullable,
  un-narrowed receiver. Corpus 0 false positives + positive tests.
- **3c·3 — join/merge + loops (Tier 2).** `&&` short-circuit narrowing DONE (`421c049`). **Arm-exit
  join/merge DONE** (`4fc8dd4`): after a guard conditional the guarded key's type is the **join** of the
  arms' exit states (via `Type::join`, the nil-lattice LUB = the union constructor, kept nil-scoped), not a
  revert to the declared type — so `x.defined?.if:{} else:{x=0}; x+1` sees `x` as `Integer`. Arm exits are
  captured by a one-shot `next_block_capture` mirroring `next_block_narrowing`; `apply_guard_join` subsumes the
  old divergence path (diverging arms drop out, surviving/fall-through paths join). **Sound by construction:** a
  guard fires only on a declared `T?`, so the join is always `⊑ T?` and the checks are monotonic → a missed
  (nested) reassignment is a false *negative*, never a new false positive. **Still deferred:** conservative loop
  back-edge widening (its own mechanism, zero corpus impact). **Follow-up gap:** a typed *param* (`|x: Integer?|`)
  uses `record_local_type` (devirt hint), not `record_declared_type`, so a param *reassignment* doesn't
  flow-update narrowing — the join's reassignment case works for `var` locals but not params; making param
  annotations declared-contracts is a small orthogonal change (needs its own corpus 0-FP check).
- **3c·4 — return-type covariance (unlocks sound `defined? → Bool`).** The doc's original 3c·4 bonus —
  `static_type(x.defined?) → Bool` — was *unsound as stated*: `defined?` is a plain overridable Quoin method
  (`Object#defined? -> { true }`, `nil` → `false`; qnlib/core/00-bootstrap.qn), so a user class could reopen it
  to return non-Bool → the inline path uses a *guarded* inline precisely to stay safe. The fix is the **Liskov
  rule for returns**: an override may return a *subtype* of the base return, never a widened/unrelated type.
  Enforce that on `Object#defined? : Bool` and no class can make `x.defined?` non-Bool → typing it `Bool` for
  *any* receiver becomes sound, narrowing/nil-misuse see through it, and every untyped `x.defined?.if:` in the
  corpus upgrades from a guarded to a **direct** inline (a corpus-wide codegen win). Also finally delivers the
  long-deferred "persist return types" idea. Trust basis: the covariance check is a gradual *warning* (not a
  hard error), so `defined? → Bool` is a contract-backed judgment on the *same* basis the VM already uses to
  direct-inline `var b: Bool` — no new soundness class; corpus 0-FP stays the tripwire.
  - **3c·4a DONE** (`c5398a9`) — `method_returns: HashMap<selector, Type>` on `ClassSig`; populate declared
    returns (`block.return_type`) from the AST for **both** `ClassDefinition` (`Foo <- {}`) *and* `ClassExtension`
    reopens of a simple class name (`Object <-- {}` — how the core classes add methods, since they're reopened
    not `<-`-defined); `ClassTable::insert` **merges/preserves** `method_returns` + `add_returns` augments a reopen
    so a later `populate_from_vm` (from_vm sigs carry no returns today) doesn't clobber accumulated ones. No reader
    yet → corpus unchanged. (The pre-fix `qn fmt` return-only-header bug that blocked `|^Bool|` headers was fixed
    first in `6bfbd30`.)
  - **3c·4b DONE** (`e5455bc`) — declared `^Boolean` on `Object#defined?` (+ `nil` override) in bootstrap;
    **covariance check** (`ClassTable::inherited_return` = nearest ancestor's declared return, with `Object` as an
    implicit universal-root fallback): an override's known return must be `compatible_with` the base, else warn
    (`override of \`sel\` returns \`X\`, incompatible with \`Y\` from \`Class\``, with span). *Confident-only* —
    `override_return_violates` speaks on a scalar mismatch or a **proven** non-subtype between two bare classes
    (consulting `is_subtype`), silent on Any/mixed/nullable-of-class → 0-FP. Corpus 0-FP verified.
  - **3c·4c DONE** (`0b6c6f1`) — `object_rooted_return_type`: `static_type(RECV.sel) →` the declared return of an
    **Object-rooted** no-arg method (covariance makes it sound for any receiver), giving `x.defined? → Boolean`;
    feeds narrowing, nil-misuse, and the direct-inline path. A bytecode regression test pins the guarded→direct
    upgrade. Corpus 1255/0/0 + GC stress, behavior-preserving; covariance case added to `qnlib/warnings.qn`.
    *Declares only `defined?` for now — other Object return contracts are follow-ups, each corpus-verified.*
  - **3c·4d — DEFERRED (nullable-guard inline recovery).** The 3c·1 early-return keeps *declared-`T?`* guards on
    the general (non-inlined) send path so their arms narrow; recovering the inline for them needs per-arm
    narrowing spliced into the inline path (`emit_inline_conditional_body`/`inline_block_body` save-restore of
    the `narrowed` overlay). Opt-in, **zero corpus impact today** (nothing annotates `T?`) → deferred. The
    corpus-wide *untyped*-guard win (3c·4c) needs none of this.
  - Provenance seed + doc/memory + corpus/stress/fmt land across 4b/4c.

*Correctness guards.* Corpus 0 false positives is the tripwire (as in 3a/3b); gradual (silent on
`Any`/unknown); a unit test per rule (arm polarity, divergence, reassignment-widen, merge, loop-conservatism,
field-invalidation).

*Open questions for 3c·0.* (1) Field-narrowing conservatism — invalidate `@x`'s narrowing on `@x = …`, and
also on any `self`-send that could reassign it (leaning yes). (2) Bare `.else:` on a Bool — recognized for
narrowing regardless of codegen inlining. (3) The exact nil-safe allowlist.

*Future unlocked by this framework (generic by design):*
- **Type-test narrowing** — a new *condition rule* (`x is-a Dog` → `Dog` in the arm) on the same overlay,
  reusing 3b's subtype relation. The framework is rule-agnostic.
- **General union types** (Phase 1 deferred these) — 3c's **join** operation *is* the union constructor;
  today it joins only `T`/`Nullable(T)`, but generalizing join → `A｜B` is the natural next step, and
  narrowing then becomes "narrow a union to a member." **3c is the substrate for unions.**
- **Exhaustiveness** (a `case` over a union), **reachability / dead-code** and **definite-assignment** (both
  seeded by the divergence tracking), and **devirt** (a narrowed non-nil/exact type removes nil-checks and
  enables monomorphic inlining — Phase 5).

*Cross-cutting follow-up (not a blocker): an AST-matcher.* Structural recognizers are accreting
(`call_selector_*`, `receiver_class`, `is_sealed_marker`, `mixin_target`, plus 3c's guard shapes), each a
nested `if let … && matches!(…)` chain that's easy to get subtly wrong (the 3b variadic-fold bug was one).
Extract a matcher **after ~3 real 3c matchers land** (rule of three), shaped by the real patterns — start
with composable matcher fns / `macro_rules!` combinators, reserve a proc-macro surface-syntax DSL only if it
earns it. Hard constraint: Quoin AST matching is **not purely structural** (a selector is a *reconstruction*
with variadic folding; local-vs-`@field`-vs-`Instance` are semantic predicates), so the matcher must **bottom
out on the existing helpers**, never re-derive them.

### Phase 4 — error ergonomics
Reuse the existing span + caret renderer. Deliver:
- **Precise spans** — caret under the offending sub-expression (AST already carries `source_info`).
- **Actual vs expected** in Quoin's type names.
- **The why-chain (provenance)** — the highest-value feature: "`x`: `String` — inferred at line 3 from
  `x = name`." Track where each inferred type came from.
- **Root cause, not cascade** — report the source mismatch, suppress downstream.
- **Gradual-friendly** — silence on `Any`.
- *(Deferred: fix-suggestions / did-you-mean.)*

### Phase 5 — feed the optimizer
Let devirt/inlining consume the richer `Type` (receiver's exact class → method inlining; `List<Int>` →
unboxed elements).

## Deferred / follow-up tasks (tracked)
- **3c·3 loop back-edge widening** — the other Tier-2 half (arm-exit join/merge is done, `4fc8dd4`). Narrowing
  across a loop must conservatively widen at the back-edge (a value narrowed in one iteration may not hold on
  re-entry). Its own mechanism; zero corpus impact → deferred.
- **Typed-param declared contracts — DONE** (`6a5909c`): a `|x: T|` param now records via
  `record_declared_type`, so its annotation is a contract — reassignment is checked *and* flow-updates
  narrowing, completing the arm-exit join for nullable params. Corpus 1255/0/0. **Surfaced** the `Object`-as-top
  gap below.
- **`Object` annotation → top type `Any` — DONE** (`df54bfe`): `from_annotation_name("Object")` now yields
  `Any` (not `Instance("Object")`), so `var x: Object = …` / `|x: Object|` no longer false-positive `expected
  Object, found …`; `Object?` collapses to `Any` too. All six callers benefit (checker decls/params/returns,
  covariance, VM-sourced arg-checks); the runtime `Object` *string* (dispatch/hierarchy) is a separate path,
  untouched. Corpus 1255/0/0.
- **3c·4d — nullable-guard inline recovery** (see the 3c·4d slice above): per-arm narrowing spliced into the
  inline path so declared-`T?` guards inline *and* narrow. Opt-in, zero corpus impact → deferred.
- **Fork-1b — persist return types into runtime introspection.** 3c·4a records declared returns into the
  *checker's* `ClassTable` from the AST. The runtime `MethodVariant`/`introspect::ClassInfo` still carry no
  return type, so `from_vm` sigs contribute none and `$inspect`/`describe_class` can't show returns. The bigger
  follow-up: have the compiler's accumulated return types flow into runtime introspection (compiler →
  `MethodVariant`), so `$inspect` shows returns *and* cross-unit return contracts survive without relying on the
  AST-recording + merge-preserve path. Cheap-ish once 3c·4a's accumulator exists; also lifts the
  cross-class-return→`Any` limit noted in Phase 3b.
- **Object return contracts beyond `defined?`** — `s : String` and `pp : String` **DONE** (`8c0336d`). These
  two are *native* (Rust) methods with no `^Ret` AST header, so their contracts are **seeded** into the
  ClassTable via `seed_native_object_returns` (called from `populate_from_vm`) rather than declared in bootstrap;
  one source feeds both object-rooted typing and covariance. `hash` is *not* native on `Object` and `class`
  returns a class object (needs a `Class` type) — both skipped. Further native contracts would follow the same
  seed pattern until the TODO below lands.
- **TODO — native-method return-type declarations (replaces the seed hardcoding).** Native methods already
  declare *arg* types at registration — `typed_class_method(sel, &["Integer"], fn)` /
  `sdk_typed_{instance,class}_method` in `src/value.rs`, threaded via `NativeMethodDef` → `introspect::ClassInfo`
  → `ClassSig::from_class_info` (which fills `method_params`). **Extend the same path to a return type**: add an
  optional return annotation to those builders (a `_ret` variant or an extra `&str`), carry it on
  `NativeMethodDef`/the VM method variant, surface it in `introspect`, and populate `ClassSig.method_returns`
  (currently hardcoded `HashMap::new()` for `from_vm` sigs). Then `Object#s`/`#pp` (and any native method)
  declare `→ String` at their own registration site, and the checker picks it up via `from_vm` — deleting the
  `seed_native_object_returns` special-case and generalizing beyond a hand-picked few. Straightforward (mirrors
  the existing arg-type wiring); this is the *native-method half* of **Fork-1b** (compiler-declared returns are
  the other half), so the two are best done together — both make `from_vm` sigs carry returns.

## Synergy with the perf roadmap

Not a detour from perf: **the real `Type` representation is the same substrate Tier-1 method inlining
needs** (`docs/FUTURE_ARCH.md`) — inlining requires knowing the receiver's exact class, which the
4-value lattice cannot express. Build the `Type` representation once; both the checker's diagnostics and
the next perf tier benefit. Ergonomics and performance converge on the same investment.
