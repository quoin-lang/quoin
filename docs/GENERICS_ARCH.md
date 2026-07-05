# Checked generic collections

*Status: DESIGN — no code yet. The settled generics syntax
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
(`Block(args ^Ret)` shares the grammar seam but is out of scope here.)

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
3. **Checked conversion**: `aList.asListOf:Integer` — verifies every
   current element, returns a **new** tagged list (copy, not in-place
   tagging: retagging an aliased list under someone else's feet is the
   kind of spooky action this design avoids). `asSetOf:`/`asMapOf:`
   likewise.

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
guarantee. Users opt in explicitly (`(JSON.parse:s).asListOf:Integer`).
One propagation exception: `sliceFrom:` (and future copying operations on
the receiver itself) carries the receiver's tag — the elements are
already checked.

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
on. Implementation note: param descriptors are precomputed at compile time
(`StaticBlock` grows a parsed form alongside `param_types: Vec<String>`),
so scoring never string-parses; the tag check itself is an enum compare
for scalar tags and the existing class walk for user classes.

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

- **G0 — syntax + lattice (checker-only):** grammar (`type_args`), the
  `TypeRefNode` AST shape through all four annotation positions,
  rendering, `ListOf`/`MapOf`/`SetOf` in the lattice with
  `compatible_with`/`join`/`name` recursion, resolver rules
  (`Map(String V)` key pinning, nested-generic warnings, unknown-base
  warnings). No runtime change; `warnings.qn` gallery grows. Plugin
  grammar PR filed alongside.
- **G1 — runtime tags + enforcement:** `ElemTag` on the three native
  states; checks at the six write sites (3 List, 1 Map, 2 Set);
  `List.of:`/`Map.of:`/`Set.of:`/`asListOf:`-family; tagged-literal
  lowering (`NewList` tag operand); `elementType` introspection;
  `sliceFrom:` propagation; dispatch tag-matching with precomputed
  descriptors; TypeError messages; parity + corpus tests (including the
  qnlib-combinator composition property: `collect:` over a tagged source
  into a tagged destination checks correctly with zero combinator edits).
- **G2 — checker integration:** `static_type` element propagation,
  insertion/assignment warnings, narrowing interplay with `T?` reads.
- **G3 — optimizer/AOT integration:** the `Boolean?.if:` nil-stub
  lowering (interpreter + AOT), AOT tag consumption, **sieve annotated
  and compiled end to end** (the acceptance test), bench re-measured.
- **Later, explicitly out of scope:** nested generic enforcement,
  `Block(args ^Ret)` types, non-String Map keys, generic user classes,
  unions. Each gets its own pass when motivated.

## 10. Open questions

1. **Constructor spelling.** `List.of:Boolean` vs `List.holding:` vs
   `List.typed:` — bikeshed welcome; `of:` proposed for brevity and
   symmetry (`Map.of:`, `Set.of:`).
2. **`each:` block param typing** (G2+): should iterating a
   `List(Integer)` type the block param `Integer?`→`Integer` (elements
   present during iteration are never the OOB nil)? Nice checker win;
   needs the generic-signature machinery — deferred.
3. **`VM.stats` counters**: tag checks performed / failed, tagged
   collections live — land with G1 per the AOT_ARCH §9.7 note.
4. **`+`-style bulk ops** (`addAll:`, list concat if it ever goes native):
   tag-to-tag fast path (skip per-element checks when source tag ⊆
   destination tag) — an optimization, not a semantic.
