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

### Phase 3 — checker pass (best-effort, gradual; separate from codegen)
- Bidirectional: check against annotations where present, infer where absent.
- Compute expression types via **method return types, field types, literals**, and **control-flow
  narrowing** (after a `.nil?` false-branch the value is non-nil).
- Report the **high-value errors**: wrong argument type, wrong return type, **method-not-found on a
  known type** (compile-time MNU), nil-misuse, unknown type name.
- **Never speak on `Any`/dynamic** (gradual-friendly).

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

## Synergy with the perf roadmap

Not a detour from perf: **the real `Type` representation is the same substrate Tier-1 method inlining
needs** (`docs/FUTURE_ARCH.md`) — inlining requires knowing the receiver's exact class, which the
4-value lattice cannot express. Build the `Type` representation once; both the checker's diagnostics and
the next perf tier benefit. Ergonomics and performance converge on the same investment.
