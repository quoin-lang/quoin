# Checked generic collections

*Status: G0 + G1 SHIPPED (G0: syntax + lattice, checker-only; G1: runtime
tags + enforcement + tag-aware dispatch — `ElemTag` on the three native
states, all write sites gated including the devirtualized arms and the AOT
ListPush/ListSet helpers, `List.of:`/`Map.of:`/`Set.of:`, `ensure:`,
`elementType`, tagged-literal lowering via `TagCollection`, `sliceFrom:`
propagation). Untagged-path cost verified at zero: full bench suite within
same-source noise. G2 SHIPPED: type variables end to end — class-header
params on `ClassSig` (AST-recorded, merged across the shared cross-unit
table; the builtin collections seeded `List(T)`/`Set(T)`/`Map(V)` so
natives declare `.returns("T?")`-style signatures), receiver-element
binding + argument unification + substitution in
`typed_receiver_return_type`, construction inference (`List.of:X`,
`ensure:X`), `emptyLike` + the `collector` species protocol (List
combinators and Set algebra preserve tags; `collect:` honestly untagged),
typed Iterate signatures (`^List(T)`, `^T?` — the generality proof),
element reads honestly `T?` through nil-narrowing, insertion warnings
mirroring the runtime check, and tagged receivers keeping collection
devirt. G2 findings: a Map's ITERATION element is a pair, not its value
type, so MapOf receivers bind only Map's own methods; a type mentioning an
unbound variable is gradual in `compatible_with` (adjudicates nothing);
`collect:`'s result typing needs `Block(args ^Ret)` annotations — deferred
with that grammar, `ensure:` covers the runtime need. G3 SHIPPED: the translator
tracks tag-backed proofs (`CollectionOf`/`ElemOrNil` — never checker
beliefs), a proven `Boolean?`-if compiles to a direct branch with a
nil→MNU stub (the cold path is never translated, deleting the
capturing-block refusal), `TagCollection` compiles, and Nil/SelfRef box
across block boundaries. **Sieve, with its two annotations, compiles end
to end: 0.97s → 0.12s (~7.9×), checksums identical in both modes — the
acceptance test passes.** G4 (Block types, §11) designed and in
progress: G4a = grammar + lattice, G4b = block-literal inference + qnlib
signatures. Runtime block enforcement analyzed and recorded as a future
arc (§12), not scheduled.
Design revisions from review: real type variables replace the earlier
implicit-`Element` idea; `emptyLike` chosen over extending `default`;
`collect:as:` dropped as redundant with inference + `ensure:`. The settled generics syntax
(`docs/TYPE_SYSTEM_ARCH.md` §"Settled surface syntax": `Class(args)`,
space-separated, nesting allowed) is design-locked but entirely unbuilt:
`List(Integer)` is a hard parse error today. This doc designs the first
implementation slice — **runtime-checked element types for List/Map/Set** —
chosen over checker-only generics because of the soundness doctrine below,
and with a concrete optimizer payoff waiting (AOT_ARCH.md's sieve refusal).*

## 1. Why checked (and not just checked-at-compile-time)

The type system's standing doctrine (TYPE_SYSTEM_ARCH.md): the *checker* is
best-effort and gradual — warnings, never gates — but the *optimizer* may
consume only guarantees. Today Quoin has exactly two guarantee sources:

1. **Typed params** — guaranteed by multimethod dispatch itself.
2. **`sealed!`** — a frozen method table, forever.

Checker-only generics (`List(Integer)` as advisory annotation) would help
ergonomics but give the optimizer nothing: an annotation nobody enforces
can't justify devirtualizing an element read. **Runtime-enforced element
tags are the third guarantee source**: a list tagged `Integer` checks every
insertion, so *whatever comes out is proven `Integer`-or-nil* — the same
"guard at the boundary, trust inside" shape as typed params.

The concrete payoff is already documented in AOT_ARCH.md: sieve does not
compile because `(isPrime.at:p).if:{…}` needs a dynamic-type branch whose
cold path re-materializes a capturing closure. With `isPrime: List(Boolean)`
the element read is proven `Boolean?`, the dynamic branch disappears at the
compiler level, and the remaining nil case has a *compile-time-provable*
answer (§7). Sieve then compiles with no new AOT machinery.

## 2. Ground truth this design stands on

- **Nothing parses.** `type_ref = { namespace? ~ ident }` (Quoin.pest:235)
  — its own comment marks it as "the seam where generics land." All four
  annotation positions (param, block-local, `^`return, `var x: T`) share
  it. The AST carries a flat `Arc<IdentifierNode>`; there is no parameter
  structure anywhere. Nullable `T?` works only because `?` is an ident
  char — a lexer trick, not a grammar feature. Generics are the first real
  structural change.
- **The runtime write surface is tiny.** `NativeListState` is
  `{ vec: Vec<Value> }` with exactly three native insertion points
  (`add:`, `push:`, `at:put:`); `sort`/`sort:` only swap; `sliceFrom:`
  copies already-checked elements. Every qnlib combinator (`collect:`,
  `select:`, `flatten`, `zip:`, `partition:`, `reverse`, `groupBy:`, set
  algebra, …) builds through those natives — **instrumenting three
  selectors covers the whole derived surface for free**. Map has one write
  (`at:put:`, String keys only); Set has `add:`/`remove:` (which already
  dispatch `==:` per element, so a tag check is cheap by comparison).
- **`Array` is precedent, not substrate.** `Array` (ofInts:/ofFloats:) is
  a packed numeric column that already does insertion-time `TypeError`s
  naming the offending index — the enforcement style to copy — but its
  buffer is bytes, not `Value`s. It coexists, unchanged.
- **Dispatch would silently break without a decision.** Param types live
  as raw strings (`StaticBlock.param_types: Vec<String>`); `type_distance`
  resolves a hint against class names. A raw `"List(Integer)"` hint
  matches nothing → the method variant is *unreachable*. §5 makes the
  dispatch semantics explicit instead of accidental.

## 3. Semantics

### 3.1 What a tag means

A collection value optionally carries an **element tag**. `#()` and every
existing construction path produce *untagged* collections — behavior
today, unchanged, zero cost (one `Option` test that predicts perfectly).
A tagged `List(Integer)`:

- **checks every insertion** (`add:`, `push:`, `at:put:`) against the tag
  and raises a house-style `TypeError` on mismatch, naming expected/got
  (and the index for `at:put:`) — the `Array.ofInts:` precedent;
- therefore **guarantees every read**: `at:` yields `Integer` or `nil`
  (out of bounds) — honestly `Integer?` in the lattice;
- **prints and compares structurally as before** (`.s`/`.pp` unchanged;
  `==:` ignores tags — least surprise; two equal-element lists are equal
  regardless of tagging);
- is introspectable: `list.elementType` → `#Integer` symbol, or `nil` when
  untagged.

`nil` elements: **allowed in every tagged collection** (the lattice type
of a read is `T?` regardless — OOB already yields nil, and Quoin
collections are nil-friendly by design, per Iterate's docs). A tag
constrains what a *present* element is, not presence.

### 3.2 Variance

**Invariant, with untagged as the top.** `List(Integer)` is assignable
where `List` is expected (it *is* a List — width subtyping); a bare/
differently-tagged list is **not** assignable where `List(Integer)` is
expected (no tag, no guarantee — the Java-array-covariance lesson, made
moot here anyway because both reads and writes are tag-checked at
runtime). The checker mirrors this; dispatch enforces it (§5).

### 3.3 Which types can be tags (v1)

Flat, non-generic type names: the scalar builtins (`Integer`, `Double`,
`Boolean`), `String`, the bare collections (`List`, `Map`, `Set`), and
user classes (matched with the same parent/mixin walk dispatch uses, so a
`List(Shape)` accepts `Circle`s). **Nested generics (`List(List(Integer))`)
parse and exist in the checker's lattice, but are not runtime-enforceable
in v1** — the resolver warns and the runtime tag degrades to the base
(`List`); no false guarantee is ever recorded (§8, "guarantee honesty").

`Map(K V)`: the settled syntax takes two parameters; keys are String-only
at the representation level (`IndexMap<String, _>`), so v1 accepts
`Map(String V)` and rejects any other key type at resolve time with a
clear diagnostic. `Set(T)` works like `List(T)` (its `==:`-based
membership walk is untouched).

## 4. Syntax and construction

### 4.1 Annotations (the settled syntax, now actually parsed)

```
type_ref  = { namespace? ~ ident ~ type_args? }
type_args = { "(" ~ type_ref ~ (" " ~ type_ref)* ~ ")" }
```

Valid in all four existing annotation positions:

```
var isPrime: List(Boolean) = #();
sum: -> { |l: List(Integer) ^Integer| … };
lookup: -> { |m: Map(String Integer) ^Integer?| … };
```

The AST grows a real type shape — `TypeRefNode { base: IdentifierNode,
args: Vec<TypeRefNode> }` — replacing the flat `Arc<IdentifierNode>` in
the four `type_hint`/`return_type` slots. `annotation_name` renders it
back (`"List(Integer)"`); the `Type` lattice gains `ListOf(Box<Type>)`,
`MapOf(Box<Type>)` (value type; key pinned String), `SetOf(Box<Type>)`,
recursing through `compatible_with`/`join`/`name` exactly as `Nullable`
does today. Bare `List` remains the untagged/any-element type.
(`Block(args ^Ret)` shares the grammar seam — built as G4, §11.)

The IntelliJ plugin mirrors `type_ref` (Quoin.bnf:285) and needs the same
grammar addition — a separate plugin PR, as with past syntax changes.

### 4.2 Construction

Three ways to get a tagged collection, all explicit or annotation-driven —
no inference magic:

1. **Constructor selectors** (class-side natives, taking the element
   class as an ordinary Class value):
   ```
   var flags = List.of:Boolean;      "empty, tagged"
   var index = Map.of:Integer;       "String keys implied"
   var seen  = Set.of:String;
   ```
2. **Annotation-driven literals**: a collection *literal* initializing a
   declaration (or default-init) whose declared type is generic compiles
   to tagged construction — `var isPrime: List(Boolean) = #()` produces a
   tagged empty list; `var xs: List(Integer) = #(1 2 3)` tags and checks
   the elements at construction. (Lowering: `NewList` grows an optional
   tag operand.) This is what makes the sieve edit a pure annotation add.
3. **Checked conversion**: `aList.ensure:Integer` — verifies every
   current element, returns a **new** tagged collection (copy, not
   in-place tagging: retagging an aliased list under someone else's feet
   is the kind of spooky action this design avoids). One generic
   selector across List/Map/Set — on a Map it ensures the *values*
   (keys are pinned String). The name is verification-first ("this must
   be this type") rather than List-specialized; no relation to Ruby's
   `ensure` (Quoin's try/finally is already `finally:`).

   **The argument is a value, and that is a deliberate alignment.**
   `ensure:` and `of:` take an ordinary Class value (`Integer`, `List`),
   which can only express *flat* types — exactly what v1 tags can
   *enforce*. `#().ensure:List(Integer)` is neither expressible nor
   enforceable in v1; `xs.ensure:List` (elements must be Lists, their
   elements unconstrained) is the flat form both the channel and the
   runtime can honestly deliver. Generic syntax lives in *type
   positions* (the four annotation slots) only. The v2 seam, when
   nested enforcement lands: make `Class(args)` legal in expression
   position too, evaluating to a first-class reified **type value**
   accepted by `ensure:`/`of:` — one spelling everywhere, in the
   classes-are-values tradition. The same "bare `ident(…)` is unused"
   rationale that won the annotation syntax should apply to expressions
   (sends are keyword/dot-based; adjacency isn't application), but that
   grammatical freedom must be verified before v2 commits; the fallback
   is an annotation-driven form (`var xs: List(List(Integer)) =
   raw.ensured`), weaker because it works only at declarations.

`List(Integer)` in *expression* position (e.g. `List(Integer).new`) is
deliberately **not** supported: `Value::Class` has no parameter slot, and
overloading call-parens in expressions collides with the method-call
grammar. The selector forms above cover construction without inventing
parameterized class values. (Revisit only if generic *classes* — not just
collections — ever land.)

### 4.3 What stays untagged

Every native decoder that builds collections directly — `JSON.parse:`,
MessagePack/YAML/TOML (`data_to_value`), CSV, `splitString:`, Map
`keys`/`values`, `Array.toList` — keeps producing untagged collections:
decoded data is inherently dynamic, and a guess would be a false
guarantee. Users opt in explicitly (`(JSON.parse:s).ensure:Integer`).
One propagation exception: `sliceFrom:` (and future copying operations on
the receiver itself) carries the receiver's tag — the elements are
already checked.

### 4.4 Type variables (checker-only, declared on class headers)

Type variables are **checker machinery, period** — there is nothing at
runtime a variable could ever be (`Block(args ^Ret)` annotations sit on
blocks that aren't even arity-checked). Tags remain the only runtime
guarantee; variables are how the checker reasons about tag flow. This
keeps guarantee-honesty crisp and makes the design one general feature
rather than an Iterate special case:

- **Declaration** — a class or mixin definition header may declare type
  parameters, same shape as the annotation syntax:
  ```
  Mixin <- Iterate(T U) <- {
      each:    -> { |b: Block(T ^Any)| … };
      select:  -> { |b: Block(T ^Boolean)| ^List(T) };
      detect:  -> { |b: Block(T ^Boolean)| ^T? };
      reduce:  -> { |b: Block(T T ^T)| ^T? };
      collect: -> { |b: Block(T ^U)| ^List(U) };
      groupBy: -> { |b: Block(T ^String)| ^Map(String List(T)) };
  }
  ```
  Every variable the class's methods use is declared once in the header;
  each *call site* instantiates them fresh, so a variable used by only
  one method (`U`) being class-scoped costs nothing semantically.
  Method-level declarations are a compatible later refinement if header
  clutter ever bites. User classes use the same syntax (`Stack(T) <- {…}`).
- **Binding, per call site, by unification**: (1) the receiver — a
  method defined on a parameterized class/mixin binds the class's
  parameter(s) from the receiver's static element type (`ListOf(Integer)`
  → `T := Integer`; Map binds `String` + value type); (2) arguments — a
  `T` in an argument position unifies with the arg's static type, which
  makes `max: -> { |a: T b: T ^T| }` work in plain user code with no
  tags anywhere; (3) blocks — binding `U` in `Block(T ^U)` requires the
  checker to infer a block literal's return type with its params bound —
  the one genuinely new checker capability. Anything unbound is `Any`.
- **Enforcement strength is a property of the class, not the syntax**:
  on the tagged native collections the class parameter is runtime-backed
  (the value's tag); on user classes it is checker-only until user-level
  tags exist. Same feature, two documented strengths.
- **Dispatch**: a variable-typed parameter scores as unconstrained
  (`Object`) in multimethod selection. Variables never dispatch; only
  concrete annotations (`List(Integer)`) are tag-exact.
- **Lying signatures** (`^List(T)` over a body that returns untagged)
  fail loudly at the next tag-demanding position (dispatch mismatch or
  `ensure:`), never silently — the same trusted-return gradualism
  scalar annotations already have.

### 4.5 `emptyLike` — the runtime bridge for the combinators

The checker claiming `select: ^List(T)` is only honest if the runtime
delivers a tagged list. The bridge is a species-style protocol on
Iterate (the instance-side sibling of the existing class-side `default`
protocol — which stays a *value* method, used by the value types too):

- `emptyLike` — a fresh empty collection *like the receiver*: Iterate's
  default implementation is `self.class.default`; the native collections
  override it to carry the receiver's element tag.
- The tag-preserving combinators (`select:`, `reject:`, `take:`,
  `drop:`, `uniq`, `reverse`, partitions, set algebra, …) build their
  results with `.emptyLike` instead of `#()` — a handful of qnlib edits,
  zero per-element cost beyond checks that provably pass (and the
  tag-to-tag fast path skips those).
- `collect:` builds a plain `#()` and returns **untagged** — its
  elements really are whatever the block produced. The static type still
  flows (inference binds `U`); when the *runtime tag* is needed (a
  tag-dispatched param, a checked return), the one general bridge is
  `ensure:`. A fused single-pass form of `.collect:{}.ensure:X` is a
  possible later optimization, not new surface syntax.

## 5. Dispatch

`|l: List(Integer)|` participates in multimethod dispatch **by tag
equality**, extending `type_distance`:

- hint `List(Integer)` matches a value iff it is a List **and** its tag is
  exactly `Integer` (distance = the usual class distance; the tag adds no
  depth). Untagged or differently-tagged lists do **not** match — they
  fall through to a `List`/`Object` variant if one exists, or MNU.
- hint `List` (bare) matches any list, tagged or not — width subtyping.

This makes the tag a **dispatch-guaranteed param fact**, identical in kind
to `|n: Integer|`: inside the method, `l`'s elements are proven without a
prologue check. That is precisely the boundary contract AOT already relies
on.

Implementation notes (as shipped in G1):
- Descriptors are precomputed: `StaticBlock.param_elem_tags` rides beside
  the base-erased `param_types` strings; all-`None` normalizes to empty so
  every pre-generics block shares one shape and legacy scoring does zero
  tag work.
- **Tag requirements are part of a variant's identity**: `|l:
  List(Integer)|` and `|l: List(String)|` share the erased base signature
  `["List"]` but are distinct multimethod variants, not a redefinition
  (same base AND same tags = the true-redefinition semantic, unchanged).
- **Specificity**: base distances are doubled and a *satisfied* tag
  requirement discounts by one — `List(Integer)` beats bare `List` for a
  tagged argument, the base class still dominates, and legacy scoring is a
  pure monotonic scaling (identical orderings, no probes).
- **Caching**: the `(kind, class-ptr)` guards of the inline and method
  caches cannot see tags, so any resolution whose chain contains a
  tag-requiring variant is marked uncacheable — the same mechanism guarded
  variants use. Legacy chains never pay this; tag-multimethod call sites
  take the full scoring path every time (a G3-era optimization candidate
  if profiles ever care).

Multimethod power this buys immediately:

```
render: -> { |xs: List(Integer)| … };
render: -> { |xs: List(String)|  … };
render: -> { |xs: List|          … };   "untagged / anything else"
```

## 6. Runtime representation and cost

`NativeListState` (and Set/Map states) gain one field:

```rust
pub elem: Option<ElemTag>   // None = today's untagged list, zero checks
enum ElemTag { Int, Double, Bool, Str, List, Map, Set, Class(Symbol) }
```

The insertion check is: `None` → nothing (one perfectly-predicted branch —
the entire existing world pays only this); `Some(scalar tag)` → a `Value`
variant test (no allocation, no hashing); `Some(Class(sym))` → the same
`value_matches_type` walk dispatch uses (fast-path string compare, then
class-chain walk). `nil` always passes. Errors follow the `arg!`/Array
house style: `expected` = tag name, `got` = `value.type_name()`, msg like
`"List(Integer): element at 3 must be Integer, got String"`.

The tag is `Copy`/static (a `Symbol` for user classes — interned, no GC
content), so the `'static`-transmuted native states need no new tracing.

## 7. The compiler and the sieve payoff

With tags as guarantees, the checker/optimizer chain extends naturally:

1. **`static_type` learns element types**: a receiver statically
   `ListOf(T)` gives `at:`/`first`/… the type `T?` (honest: OOB is nil).
   Sources: declared annotations (params, `var x: List(T)`), `List.of:`
   construction, `^List(T)` returns via ClassSig.
2. **The `Boolean?` condition lowering** — the piece that unlocks sieve:
   `cond.if:{…}` where `cond: Boolean?` no longer needs
   `BranchIfNotBool`'s open-world cold path. The only non-Boolean
   possibility is `nil`, and `Nil` is startup-sealed with no `if:` — so
   the cold path is a *compile-time-provable* MessageNotUnderstood. The
   compiler emits: nil-test → inline branch on the Boolean / raise the
   exact MNU error. No capturing block is materialized, in the
   interpreter lowering or in AOT (which gets a two-arm branch + an
   error-stub — machinery it already has).
3. **AOT consumes the tags**: `AotParam::from_annotation("List(Integer)")`
   → Obj with a known element type; `ListGet` results narrow to scalars
   through the existing checked-narrow emitter with a provable nil arm
   instead of a dynamic one. Sieve, with two added annotations
   (`var isPrime: List(Boolean) = #()`, `var primes: List(Integer) = #()`
   — the same typed opt-in spirit as fib's annotations and `sealed!`),
   compiles end to end. That is the acceptance test for the whole feature.

Checker-only conveniences ride along gradually (warnings on inserting a
`String` into a statically-`List(Integer)` local, on assigning untagged
where tagged is declared) — best-effort, non-fatal, "corpus 0 false
positives" tripwire as always.

## 8. Guarantee honesty (the rules that keep this sound)

- A runtime tag is recorded **only** when the runtime will actually
  enforce it. Nested generics, `Block(…)` types, and any future
  not-yet-enforceable annotation degrade to the enforceable base *with a
  resolver warning* — the checker may still reason best-effort, but
  nothing downstream (devirt, AOT, dispatch) may treat an unenforced
  annotation as a guarantee.
- The optimizer consumes element types **only** from: dispatch-guaranteed
  tagged params, tagged construction it can see (`List.of:`, tagged
  literals), and `^List(T)` returns *of compiled/sealed methods it can
  trust* (same trusted-return caveats as scalars — a checked narrow at
  the consumption point when the source is untrusted).
- Untagged collections never change behavior, cost, or meaning.

## 9. Slices (each shippable, each corpus-gated)

- **G0 — syntax + lattice (checker-only):** grammar (`type_args` in
  annotation positions AND class/mixin definition headers), the
  `TypeRefNode` AST shape, rendering, `ListOf`/`MapOf`/`SetOf` plus a
  `Var(name)` form in the lattice with `compatible_with`/`join`/`name`
  recursion, resolver rules (`Map(String V)` key pinning, declared-vs-
  unknown variable names, nested-generic warnings). No runtime change;
  `warnings.qn` gallery grows. Plugin grammar PR filed alongside.
- **G1 — runtime tags + enforcement:** `ElemTag` on the three native
  states; checks at the six write sites (3 List, 1 Map, 2 Set);
  `List.of:`/`Map.of:`/`Set.of:` and `ensure:`; tagged-literal
  lowering (`NewList` tag operand); `elementType` introspection;
  `sliceFrom:` propagation; dispatch tag-matching with precomputed
  descriptors; TypeError messages; parity + corpus tests (including the
  qnlib-combinator composition property: `collect:` over a tagged source
  into a tagged destination checks correctly with zero combinator edits).
- **G2 — checker integration (the type-variables slice):** call-site
  unification (receiver-tag, argument, and block-return binding — block
  literal return inference is the chunky new capability), `static_type`
  element propagation, insertion/assignment warnings, narrowing
  interplay with `T?` reads. Deliverables: `emptyLike` (native overrides
  + Iterate default + qnlib combinator edits) and **typed signatures on
  the Iterate mixin** — the proof the design generalizes.
- **G3 — optimizer/AOT integration:** the `Boolean?.if:` nil-stub
  lowering (interpreter + AOT), AOT tag consumption, **sieve annotated
  and compiled end to end** (the acceptance test), bench re-measured.
- **G4 — Block types** (`Block(args ^Ret)`, §11): the type-position
  grammar, `Type::BlockOf`, block-literal return inference (binds
  `collect:`'s `U`), inferred iteration-block params, typed Block
  signatures on Iterate. Checker-only by §4.4's line.
- **Later, explicitly out of scope:** nested generic enforcement,
  non-String Map keys, generic user classes, unions, runtime block
  enforcement (§12). Each gets its own pass when motivated.

## 10. Open questions (settled ones recorded)

1. **Settled:** species protocol = `emptyLike` (instance-side; `default`
   stays the class-side *value* protocol, used by value types too).
   `collect:as:` dropped — the checked conversion is the sole
   checker→runtime bridge, spelled **`ensure:`** (one generic selector
   for List/Map/Set; verification-first, not List-specialized); a fused
   checked pass is a later optimization. Type variables:
   class-header declaration, unification binding, checker-only.
2. **Constructor spelling.** `List.of:Boolean` proposed for brevity and
   symmetry (`Map.of:`, `Set.of:`) — still open to bikeshed.
3. **`each:` block param typing** (G2): with variables this falls out —
   `each: -> { |b: Block(T ^Any)| }` types the block param `T`, not
   `T?` (elements present during iteration are never the OOB nil).
4. **`VM.stats` counters**: tag checks performed / failed, tagged
   collections live — land with G1 per the AOT_ARCH §9.7 note.
5. **`+`-style bulk ops** (`addAll:`, list concat if it ever goes
   native): tag-to-tag fast path — an optimization, not a semantic.
6. **Method-level variable declarations**: deferred refinement if
   class-header declaration ever feels cluttered.
7. **Expression-position type values** (v2, with nested enforcement):
   verify `ident(…)` is grammatically free in expression position; if
   so, reified type values are the path — see §4.2's alignment note.

## 11. G4 — Block types (`Block(args ^Ret)`)

The settled syntax (TYPE_SYSTEM_ARCH §"Settled surface syntax") makes a
block's type its header with the names stripped:

```
{ |a:Integer b:Integer ^Integer| … }   ⟺   Block(Integer Integer ^Integer)
```

`Block()` is zero-args/`Any`-return; bare `Block` (no parens) stays the
fully-unconstrained type every existing annotation already means. The
header half (`block_ret = "^" ~ type_ref` inside `block_decls`) has
parsed since the return-in-header migration; G4 builds the
*type-position* half and the checker capability that makes it pay.

### 11.1 The guarantee line: beliefs, never guarantees

`value`/`value:` binds block args by plain zip — no arity check, no
type check (`start_block`). Param types are enforced only in the
*method role*, by dispatch's match scoring. So `Block(args ^Ret)` is
checker machinery, period, exactly as §4.4 drew the line. Concretely:

- **Inferred** param types (from unifying a literal against a declared
  `Block(T ^Any)` param) enter the block body as *narrowing-grade*
  entries — read by `static_type`, warnings, and nil-narrowing — via
  the same one-shot channel `if:else:` guard arms use. They never go
  through `record_declared_type`, so they seed no devirt and produce
  no reassignment errors: a wrong belief costs at most a soft warning.
- **Explicitly annotated** literal params (`{ |x: Integer| … }`) keep
  their existing behavior: `compile_block` seeds them as declared types
  for in-body devirt. The ops behind that seeding are value-guarded
  (probed: `{ |x: Integer| x + 1 }.value:'nope'` falls back and returns
  `'nope1'`), so the seeding is operationally safe even though `value:`
  never checks — but the "dispatch only selects a typed method when the
  arg matches" justification in the code covers only the method role;
  the comment says so now. Inferred types never take this path.
- Nothing new reaches the runtime: `StaticBlock` is untouched,
  `collect:` stays runtime-untagged (`ensure:` remains the bridge), and
  `ElemTag::from_type(BlockOf) = None` — a `List(Block(Integer ^Integer))`
  tag degrades to `List(Block)`… i.e. to base `List` with a warning,
  like any nested generic (§8).

### 11.2 Grammar, AST, lattice (G4a)

- **Grammar**: `type_args` generalizes to
  `"(" ~ type_ref* ~ ("^" ~ type_ref)? ~ ")"` — one rule for all bases,
  G0-style structural resolve with targeted diagnostics: a `^`-tail or
  empty parens on a non-`Block` base warns and degrades to the base;
  `Block(...)` stops warning "does not take generic arguments".
- **AST**: `TypeRefNode` gains `ret: Option<Arc<TypeRefNode>>` plus a
  parens marker so `Block()` ≠ `Block`.
- **Lattice**: `Type::BlockOf { params: Vec<Type>, ret: Box<Type> }`.
  Width subtyping `BlockOf ⊆ Block` (a shaped block is a block).
  Between shapes: arity-exact, params contravariant, return covariant —
  the sound direction costs nothing extra to write, and the existing
  `contains_var` gate keeps any var-involved comparison gradual. Joins
  of differing shapes widen to bare `Block`, as differing tags widen to
  the bare collection. Recurses through `substitute`/`unify_into`/
  `parse_annotation_str` so natives can declare block signatures too.
- **Dispatch: full erasure.** `|b: Block(T ^Boolean)|` dispatches
  exactly as `|b: Block|` (which *is* an enforceable hint —
  `type_name() == "Block"`). Unlike element tags, block shapes are
  **not** part of variant identity: the runtime cannot tell
  `Block(Integer ^Boolean)` from `Block(String ^String)`, so two
  same-base signatures differing only in block shape are a
  *redefinition*, not coexisting multimethod variants. Guarantee
  honesty applied to dispatch: never let two variants differ by a
  distinction the runtime can't check.
- The formatter is untouched (block headers are sliced verbatim; a
  round-trip test pins it); the ANSI/IntelliJ highlighters learn the
  `^` span; the plugin grammar mirrors `type_args` in a companion PR.

### 11.3 Block-literal inference (G4b — the new checker capability)

At `xs.collect:{ |x| x * 2 }` with `xs: List(Integer)`:

1. The callee's declared param `Block(T ^U)` (via the G2 `method_params`
   machinery) substitutes receiver-bound variables → `Block(Integer ^U)`.
2. The expected param types flow into `compile_block` and seed the
   literal's params as beliefs — `T`, not `T?`, per §10.3 (elements
   present during iteration are never the OOB nil). Explicit header
   annotations win over inference.
3. The body's actual return type is harvested: the join of the tail
   expression and every `^` block-return; `^^` (non-local return)
   contributes nothing — the block never returns normally through it.
   Harvested types are memoized span-keyed so `static_type` answers
   consistently before and after the body compiles.
4. Unification binds `U := Integer`; `^List(U)` → `List(Integer)` —
   checker-typed, runtime-untagged, honest.

A bare literal with a declared header (`{ |x: Integer ^Boolean| … }`)
sharpens its own static type from `Block` to
`Block(Integer ^Boolean)` the same way.

### 11.4 qnlib signatures (the generality proof)

`Iterate(T)` becomes `Iterate(T U)`; the §4.4 signatures land as
written (`collect:` = `|b: Block(T ^U) ^List(U)|`, the predicate family
`Block(T ^Boolean)`, `reduce:` = `Block(T T ^T)`, `sort:`'s comparator
`Block(T T ^Boolean)`), and the concrete `each:` definitions take
`Block(T ^Any)`. Map keeps the G2 nuance: its iteration element is a
pair, not its value type, so Map's Iterate methods bind no `T`.
Corpus-zero-warnings remains the gate.

## 12. Future arc: runtime block enforcement (recorded, not scheduled)

Would checking block args at `value:` unlock anything? Analyzed
2026-07; the answer shapes when — and whether — to build it.

**What type enforcement would mint: a fourth guarantee source** (after
dispatch-checked params, `sealed!`, element tags), and its consumers,
ranked:

1. **AOT block templates — the real prize.** `AotParam` trusts a
   method's typed params *because dispatch enforces them*; that is what
   makes scalar/unboxed parameter slots sound. Blocks have no such
   enforcement, so a block template can never be an AOT candidate with
   trusted params — stuck at `Dyn` + in-body checked narrowing.
   Enforcement at `value:` extends the method-role justification to
   blocks verbatim, and real programs spend their time inside
   combinator blocks, not `while` loops (sieve compiled because it is
   loop-shaped; most code isn't).
2. **Error locality.** Today `{ |x: Integer| x + 1 }.value:'nope'`
   silently returns `'nope1'` — the annotation is decorative at the
   boundary. Enforcement raises the house-style TypeError at the call
   site, makes typed params mean the same thing in both roles, and
   gives §4.4's "lying signatures" a fail-loud point consistent with
   how tags fail.
3. **Marginal.** Enforced *return* types would let `collect:` mint an
   honestly-tagged result when the block declares `^Integer` (the fused
   checked pass §4.5 defers) — ergonomics only, `ensure:` already
   covers it in one pass. Structural dispatch on block shapes becomes
   sound (§11.2's erasure rule could be revisited) — niche. Interpreter
   devirt in block bodies could drop value guards — near-zero, those
   guards are predicted branches.

**Arity checking alone: almost nothing.** The ecosystem *adapts* to
arity rather than assuming it (`valueWithSelfOrArg:` inspects and
calls accordingly), and the zip-bind tolerance (missing → nil, extras
dropped) may be load-bearing. AOT already knows a template's arity.
Better error messages for one class of bugs, at breaking-change cost.

**The counterweights:**

- **For iteration, tags already carry the guarantee.** A
  `List(Integer)` iterated by `each:` hands the block values that are
  tag-proven `Integer`-or-nil — exactly a G3 `ElemOrNil` proof. If AOT
  inlines the iteration protocol (the classic Smalltalk move: the
  combinator becomes a compiled loop invoking the block body directly),
  the proof flows from the receiver's tag into the body with zero
  per-element checks and zero annotations. Enforcement is redundant
  there; where it is the *only* possible source is escaped blocks —
  callbacks, comparators, stored handlers — invoked with values from
  arbitrary places.
- **The cost lands on the hottest primitive in the language.** Every
  typed block call pays a dispatch-grade check; untyped blocks need the
  all-`"Object"`-normalizes-to-empty trick (the `param_elem_tags`
  precedent) to stay at one branch. Inlined control-flow blocks never
  reach `value:` and are exempt.
- **Enforcement can never strengthen inference.** It applies only to
  *explicit* annotations — enforcing an inferred type would change
  program behavior on the strength of checker beliefs, the one line the
  doctrine never crosses. G4 is unaffected whether or not this arc
  ever lands.

**Shape of the arc, if built:** opt-in per annotation; `value:`-time
param checks for explicitly-typed blocks (the general/escaped-callback
story) composing with tag-flow-through-inlined-combinators (the
iteration story, free); enforced returns only if the fused-`collect:`
payoff ever matters. AOT_ARCH §9 tracks the block-template question
from its side.
