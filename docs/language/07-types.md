# Part VII — The gradual type system

Quoin is dynamic by default: nothing in the language requires a type annotation.
But every annotation position you've seen so far — typed block params (§7), typed
multimethod params (§13) — belongs to one coherent, gradual type system: optional
annotations, a best-effort compile-time checker (`qn check`), runtime-checked
collections, and an optimizer that consumes the parts it can trust.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · **Types** · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · [Packages](10-packages.md) · [Appendices](11-appendices.md)

---

## 28. Types are optional

> **Rules**
> - **Annotations are optional everywhere.** Un-annotated code is not "untyped code
>   that happens to work" — it is the default, fully supported style. An
>   un-annotated parameter or variable is *gradual*: the checker assumes nothing
>   about it and never warns on it.
> - Annotations serve three consumers at once:
>   1. **The checker** — `qn check` reports likely type bugs as **non-fatal
>      warnings** (§30). It never blocks compilation or execution.
>   2. **Dispatch** — a typed method parameter selects a multimethod variant by
>      the argument's *runtime* type (§13, §32). This half is enforced.
>   3. **The optimizer** — typed locals, sealed classes (§34), and checked
>      collections (§33) let the compiler devirtualize and compile code it would
>      otherwise run generically.
> - Only three things are ever **enforced at runtime**: dispatch-checked method
>   parameters, element tags on checked collections, and `sealed!`. Every other
>   annotation is advisory — a wrong one costs a warning, never a crash.

Dynamic Quoin is complete on its own. This method has no annotations and happily
serves two types:

```quoin
Doubler <- {
    double: -> { |n| n + n }
}

Doubler.new.double:21        "* -> 42
Doubler.new.double:'ho'      "* -> 'hoho'
```

Add a type to the parameter and the method's contract becomes real: dispatch
only selects the variant when the argument actually is an `Integer`, so a wrong
argument is a `MessageNotUnderstood` at the call — not a strange result later:

```quoin
Halver <- {
    half: -> { |n: Integer ^Integer| n / 2 }
}

Halver.new.half:42                                    "* -> 21
{ Halver.new.half:'ho' }.catch:{ |e| e.class.name }   "* -> 'MessageNotUnderstood'
```

The stance throughout: **nothing requires types; everything benefits from them.**
Annotate where a contract matters (public methods, collection contents,
nullable data) and leave exploratory code bare.

---

## 29. Annotation syntax

> **Rules**
> - **Where a type may be written** (all positions share one grammar):
>
>   | Position | Syntax |
>   |---|---|
>   | Block/method parameter | `{ \|n: Integer\| … }` |
>   | Return type (in the header, after the params) | `{ \|n: Integer ^Integer\| … }` |
>   | Local declaration | `var x: Integer = 5`, `let y: String = 'hi'` |
>   | Block-local (after the `-`) | `{ \|a - tmp: Integer\| … }` |
>   | Class-header type variable | `Stack(T) <- { … }` |
> - **Type names**: a bare `Name` is a class in the root namespace; namespaced
>   classes work in every position (`var f: [IO]File = …`, `|e: [Web]Halt|`).
> - **Nullable**: `Integer?` means "`Integer` or `nil`" (§31). No space — `?` is
>   part of the identifier, so `Integer ?` is not a type.
> - **Generics**: `Class(args)`, space-separated — `List(Integer)`,
>   `Map(String Integer)`, `Set(String)`, nesting allowed. Only `List`/`Map`/`Set`
>   take type arguments today; Map keys are pinned to `String` (§33).
> - **Block types**: `Block(args ^Ret)` — a block's type is its header with the
>   names stripped. `Block()` is zero-arg, bare `Block` is fully unconstrained.
> - **Type variables** are declared on the class header — `Stack(T) <- { … }` —
>   and usable as types in that class's methods (`push: -> { |x: T| … }`,
>   `top -> { |^T?| … }`). They are checker machinery only: a variable-typed
>   parameter never constrains dispatch and carries no runtime check.
> - A method's return type lives in its header too — there is no `-> Ret` form:
>   `area -> { |^Double| … }`.

```quoin
var count: Integer = 0                            "* typed local
var title: String? = nil                          "* nullable: String or nil
var xs: List(Integer) = #(1 2 3)                  "* checked collection (§33)
var index: Map(String Integer) = #{ 'ada': 1 }    "* value type Integer; keys are String
var pred: Block(Integer ^Boolean) = { |n| n > 0 } "* a block type
pred.value:3                                      "* -> true
```

Return types sit in the header after the parameters, marked with `^` — the same
character as the return statement, in a type position:

```quoin
Circle <- { |@r|
    area -> { |^Double| 3.14159 * @r * @r };
    grow: -> { |k: Double ^Double| @r = @r * k; .area }
}

(Circle.new:{ r = 2.0 }).area          "* -> 12.56636
```

Because a block's type is exactly its header minus the names, the two sides of
this correspondence read the same way:

```quoin norun
{ |a: Integer b: Integer ^Integer| a + b }     "* a value of type…
Block(Integer Integer ^Integer)                "* …this type
```

> **⚠ Gotcha — in the block role, annotations are not runtime checks.** `value:`
> binds arguments with no arity or type checking (§7), so a typed block param is
> documentation plus checker/optimizer input — not a guard. Only the *method*
> role enforces parameter types, because there dispatch itself does the checking
> (§32):

```quoin
{ |x: Integer| x + 1 }.value:'nope'    "* -> 'nope1'
```

---

## 30. The checker: `qn check`

> **Rules**
> - `qn check <path>…` type-checks files **without running them**. Clean → silent,
>   exit 0. Otherwise each diagnostic prints as `file:line:col: warning: message`
>   with the source line and a caret, and the exit code is 1.
> - The same warnings print (to stderr) when you simply run a program — `qn check`
>   is the "check only" entry point, not a separate analysis.
> - **Warnings are never fatal.** A program with type warnings still compiles and
>   runs; the checker exists to point, not to gate.
> - **Gradual means silent on dynamic code.** The checker only speaks where types
>   are *known*: annotated values, literals, and stdlib contracts. Un-annotated
>   code never warns.
> - What it catches: type mismatches at declarations, reassignments, arguments and
>   returns; sends a class provably can't answer (compile-time
>   `MessageNotUnderstood`); return-type covariance violations on overrides;
>   nil-misuse on nullable values (§31); bad insertions into checked collections
>   (§33); malformed generic shapes.
> - `qnlib/warnings.qn` is the maintained gallery: one method per diagnostic.
>   `qn check qnlib/warnings.qn` renders every warning the checker can produce.
> - A deliberate warning can be silenced in place: a **trailing** `"* allow: <kind>`
>   comment suppresses that kind on its own line only. The pragma polices itself —
>   an unknown kind name, or a pragma on its own line, is a warning.

Given a file with three bugs:

```quoin norun
"* stats.qn
Stats <- {
    label: -> { |score: Integer ^String| score };

    firstOver: -> { |xs: List(Integer) ^Integer|
        xs.detect:{ |n| n > 100 }
    };

    span -> { var r: NumberRange = 1..10; r.middle }
}
```

`qn check stats.qn` reports:

```
stats.qn:2:42: warning: type mismatch: expected `String`, found `Integer`
    |
  2 |     label: -> { |score: Integer ^String| score };
    |                                          ^^^^^
  stats.qn:2:18: note: `score` is `Integer` (parameter)
    |
  2 |     label: -> { |score: Integer ^String| score };
    |                  ^^^^^
stats.qn:5:9: warning: type mismatch: expected `Integer`, found `Integer?`
    |
  5 |         xs.detect:{ |n| n > 100 }
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^
stats.qn:8:43: warning: `NumberRange` does not respond to `middle`
    |
  8 |     span -> { var r: NumberRange = 1..10; r.middle }
    |                                           ^
```

Each diagnostic carries the location, the message in Quoin's own type names, the
offending line with a caret, and — where an inference is involved — a `note:`
tracing *why* the checker believes what it believes (`score` is `Integer`
because the parameter says so).

The three warnings show the checker's range. The first is a straight
declaration-vs-value mismatch. The second is nullable honesty: `detect:` can
come up empty, so its result is `Integer?`, which doesn't satisfy a declared
`^Integer` (§31). The third is a **compile-time MNU** — but note what made it
possible: `NumberRange` is a *sealed*, already-loaded class, so no future code
can add `middle` to it, and the checker can *prove* the send fails. On an open
class the same send stays silent — some later `<--` extension could legitimately
add the method, and a false warning is worse than a missed one.

The checker also enforces the override contract that keeps inherited types
meaningful — an override must return a subtype of what the base method declares
(this is what lets the checker trust `x.defined?` to be `Boolean` for *any*
receiver, §31):

```quoin norun
Sneaky <- {
    defined? -> { |^String| 'always' }
}
```

```
override.qn:2:21: warning: override of `defined?` returns `String`, incompatible with `Boolean` from `Object`
    |
  2 |     defined? -> { |^String| 'always' }
    |                     ^^^^^^
```

One convenience the checker allows rather than flags: an `Integer` *literal* in
a `Double` position is promoted to the double it obviously means, at the value
level:

```quoin
var d: Double = 1        "* no warning: the literal 1 becomes 1.0
d.class.name             "* -> 'Double'
```

Finally, the escape hatch. Sometimes the flagged behavior is the point — a test
that *wants* to send `if:` to a maybe-nil value to pin the runtime error, say.
A trailing `allow:` comment silences exactly that warning kind, exactly there:

```quoin norun
(flags.at:n).if:{ ^^1 };  "* allow: nil-receiver (nil -> MNU is the point)
```

The kind name is the warning's family — `nil-receiver`, `caret-discard`, `mnu`,
`no-variant`, `element-type`, `type-mismatch`, `return-type`, `unknown-type`,
`annotation`, `portability` (a block shipped to a worker whose shape can never
cross — see chapter 5's portable-block rules) — and several can be listed,
separated by commas. A parenthesized
rationale is encouraged and ignored by the parser. Three rules keep suppressions
honest: the pragma must **trail** the warned line (on a line of its own it would
be captured as a doc comment, so that placement warns instead of silently doing
nothing); an unknown kind name warns rather than no-ops, so a typo can't leave a
phantom suppression; and a pragma only reaches its own line, so it can't blanket
a file.

---

## 31. Nullable types & nil narrowing

> **Rules**
> - `T?` is "`T` or `nil`". It is the honest type of anything that can be absent:
>   a Map lookup, `detect:`, an element read from a checked collection (§33).
> - **Nil-misuse warnings are opt-in by annotation**: a non-nil-safe message sent
>   to a value the checker knows is `T?` warns (``receiver of `abs` may be nil``,
>   ``left operand of `+` may be nil``). Nil-safe messages (`defined?`, `s`, `pp`,
>   `class`, `==`, `!=`, `hash`) stay silent. Unannotated values never warn.
> - **Narrowing is flow-sensitive.** A nil guard refines the type inside the
>   guarded arm (and after it, when the nil arm diverges). Recognized shapes,
>   for a local or an `@field`:
>
>   | Guard | Narrows |
>   |---|---|
>   | `x.defined?.if:{A} else:{B}` | `A`: `x` is `T` · `B`: `x` is nil |
>   | `x.defined?.else:{ ^^ … }` | rest of the method: `x` is `T` |
>   | `(x != nil).if:{A}` / `(x == nil)` | same, polarity flipped |
>   | `x.defined? && expr` | `expr` sees `x` as `T` |
> - Reassignment updates the flow: after `x.defined?.if:{} else:{ x = 0 }`, both
>   arms leave `x` non-nil, so the join does too.
> - `Object#defined?` is declared `^Boolean` (and covariance protects it, §30),
>   which is why `x.defined?.if:` is sound narrowing for *any* receiver.

Dereferencing a nullable without a guard draws the warning:

```quoin norun
Badge <- {
    width: -> { |name: String? ^Integer| name.length + 4 }
}
```

```
badge.qn:2:42: warning: receiver of `length` may be nil
    |
  2 |     width: -> { |name: String? ^Integer| name.length + 4 }
    |                                          ^^^^
```

Guard it and the same send is silently fine — the checker follows the flow. The
early-return form reads especially well: handle absence once, then the rest of
the method sees the narrowed type:

```quoin
Grades <- {
    letterFor:in: -> { |name: String scores: Map(String Integer) ^String|
        var n: Integer? = scores.at:name;
        n.defined?.else:{ ^^ 'absent' };      "* from here on, n is Integer
        (n >= 90).if:{ 'A' } else:{ 'B or below' }
    }
}

var g = Grades.new
var scores: Map(String Integer) = #{ 'ada': 97 }
g.letterFor:'ada' in:scores        "* -> 'A'
g.letterFor:'grace' in:scores      "* -> 'absent'
```

This is the static complement of the strict-conditional rule from §8: `nil.if:`
is a runtime error, and `x.defined?.if:{ … }` is both the idiomatic guard *and*
the thing the checker understands.

```quoin
var scores: Map(String Integer) = #{ 'ada': 97 }
var n: Integer? = scores.at:'grace'
n.defined?.if:{ 'present' } else:{ 'absent' }      "* -> 'absent'
```

A `T?` parameter participates in dispatch as base-type-or-nil: a
`|name: String?|` variant matches a `String` argument at the base type's
specificity, and matches `nil` exactly — so a nullable variant can sit beside
concretely-typed siblings:

```quoin
W <- {
    width: -> { |name: String?| name.defined?.if:{ 'str' } else:{ 'none' } };
    width: --> { |n: Integer| 'int' }
};
var w = W.new;
w.width:'x'    "* -> 'str'
w.width:nil    "* -> 'none'
w.width:7      "* -> 'int'
```

---

## 32. Types at dispatch time

> **Rules**
> - Typed parameters are the **enforced** half of the type system: a method
>   variant is selected only when every argument matches its declared parameter
>   types, by the multimethod rules of §13 (specificity ranking, guards refine,
>   equal ties are ambiguous).
> - No matching variant → `MessageNotUnderstood` at the call. If the selector
>   exists with non-matching types, the error **lists the filtered-out
>   candidates** as a hint.
> - Because dispatch guarantees the parameter's type, the body needs no defensive
>   checks — and the optimizer treats the annotation as proven fact.
> - **Element tags dispatch too** (§33): `|xs: List(Integer)|` matches only a
>   list *tagged* `Integer`; bare `|xs: List|` matches any list. Two variants
>   may differ only by tag.
> - **Block shapes do not dispatch**: `|b: Block(Integer ^Boolean)|` dispatches
>   exactly like `|b: Block|` (the runtime can't check a block's shape, so two
>   variants differing only in block shape are a *redefinition*, later wins).
> - A guard composes with the type: `|n: Integer { n > 100 }|` matches an
>   `Integer` for which the guard is truthy.

```quoin
Triage <- {
    rank: -> { |n: Integer { n > 100 }| 'big' };
    rank: --> { |n: Integer| 'small' };
    rank: --> { |x| 'not a number' }        "* untyped catch-all
}

var t = Triage.new
t.rank:400          "* -> 'big'
t.rank:7            "* -> 'small'
t.rank:'seven'      "* -> 'not a number'
```

Without a catch-all, a wrong-typed argument fails loudly — and helpfully:

```quoin
Describer <- {
    describe: -> { |n: Integer| 'int ' + n.s };
    describe: --> { |s: String| 'str ' + s }
}

Describer.new.describe:7        "* -> 'int 7'
Describer.new.describe:'hi'     "* -> 'str hi'
{ Describer.new.describe:3.14 }.catch:{ |e: MessageNotUnderstood| e.message }
                                "* -> 'no method \'describe:\' for Describer'
```

(Uncaught, the same error also prints the candidate variants — see §35.)

Generic collection types make dispatch element-aware, because tagged collections
really know their element type at runtime (§33):

```quoin
Render <- {
    show: -> { |xs: List(Integer)| 'all ints' };
    show: --> { |xs: List| 'any old list' }
}

var r = Render.new
r.show:(#(1 2).ensure:Integer)      "* -> 'all ints'
r.show:#(1 2)                       "* -> 'any old list'
r.show:#(1 'two')                   "* -> 'any old list'
```

The untagged `#(1 2)` falls through to the bare `List` variant even though its
elements happen to be integers — dispatch trusts **tags** (guarantees), never
inspection of the current contents.

---

## 33. Checked generic collections

> **Rules**
> - A `List`/`Map`/`Set` may carry a runtime **element tag**. Tagged collections
>   **check every insertion** and raise a `TypeError` on mismatch; therefore every
>   read is *guaranteed* tag-or-nil. Untagged collections (every plain literal,
>   everything decoded from JSON/CSV/etc.) behave exactly as before, cost nothing,
>   and match only bare `List`/`Map`/`Set` types.
> - Three ways to construct a tagged collection:
>   1. `List.of:Integer` / `Map.of:Integer` / `Set.of:String` — empty, tagged
>      (`Map.of:` tags the **values**; keys are always `String`).
>   2. An annotated declaration whose initializer is a literal:
>      `var xs: List(Integer) = #(1 2 3)` — tags and checks the elements.
>   3. `coll.ensure:Integer` — verifies every current element and returns a **new**
>      tagged collection (the original is untouched). This is the bridge from
>      dynamic data to checked data.
> - `coll.elementType` → the tag as a Symbol (`#Integer`), or `nil` when untagged.
> - `nil` elements are always allowed — a tag constrains what a *present* element
>   is, so reads are honestly `T?` (§31).
> - Element-preserving combinators (`select:`, `reject:`, `take:`, `drop:`,
>   `reverse`, `uniq`, `sort`, slices, set algebra) **keep the tag**. `collect:`
>   returns **untagged** — its elements are whatever the block produced; chain
>   `.ensure:` when you need the tag back.
> - User classes work as tags (`List.of:Shape` accepts subclass instances). Nested
>   generics (`List(List(Integer))`) are checker-only: the runtime tag degrades to
>   the base with a warning, never a false guarantee.
> - The checker mirrors the runtime: a statically-visible bad insertion warns at
>   compile time; the same insertion arriving dynamically raises at runtime.

```quoin
var xs = List.of:Integer
xs.add:3
xs.elementType                            "* -> #Integer

var bad = #('one').at:0                   "* arrives dynamically — invisible to the checker
{ xs.add:bad }.catch:{ |e| e.message }    "* -> 'List(Integer): element must be Integer, got String'
xs.count                                  "* -> 1
```

The insertion check is the entire mechanism — because writes are guarded, reads
need no checks, and whatever comes out of a `List(Integer)` is provably an
`Integer` or `nil`. Construction through an annotated literal checks up front:

```quoin
{ var nope: List(Integer) = #(1 'two' 3); nope }.catch:{ |e| e.message }
    "* -> 'List(Integer): element at 1 must be Integer, got String'
```

Decoded data is inherently dynamic, so decoders never guess a tag — `ensure:` is
the explicit opt-in:

```quoin
var raw = JSON.parse:'[1, 2, 3]'
raw.elementType                           "* -> nil
(raw.ensure:Integer).elementType          "* -> #Integer
{ #(1 'two').ensure:Integer }.catch:{ |e| e.class.name }    "* -> 'TypeError'
```

Tags flow through the combinators that preserve elements, and honestly don't
through the one that transforms them:

```quoin
var days: List(String) = #('monday' 'tue' 'wednesday')
(days.select:{ |d| d.length > 4 }).elementType    "* -> #String
days.reverse.elementType                          "* -> #String
(days.collect:{ |d| d.length }).elementType       "* -> nil
```

On the checker side, declared element types catch both directions of mistake —
bad writes and unguarded reads:

```quoin norun
Inventory <- {
    restock: -> { |counts: Map(String Integer)|
        counts.at:'bolts' put:'twelve'
    };

    firstCount: -> { |counts: Map(String Integer) ^Integer|
        counts.at:'bolts'
    }
}
```

```
inventory.qn:3:31: warning: `Map(String Integer)` rejects a `String` element — this raises a TypeError at runtime
    |
  3 |         counts.at:'bolts' put:'twelve'
    |                               ^^^^^^^^
inventory.qn:7:9: warning: type mismatch: expected `Integer`, found `Integer?`
    |
  7 |         counts.at:'bolts'
    |         ^^^^^^^^^^^^^^^^^
```

> **⚠ Gotcha — `ensure:` copies; tagging is never in-place.** Retagging a list
> aliased elsewhere would change its behavior under someone else's feet, so
> `ensure:` always returns a fresh collection and leaves the receiver untagged.
> Note the type-argument forms are for *type positions* only: construction and
> conversion take an ordinary class value (`List.of:Integer`, `.ensure:Integer`) —
> `List(Integer).new` is not an expression.

---

## 34. Sealing

> **Rules**
> - `Class.sealed!` **freezes** a class: no `<--` reopening, no new `->`/`-->`
>   definitions, no `.mix:`, no subclassing. Extension attempts raise a catchable
>   `ClassError`; a subclass attempt raises too. Seal *last* in a class body (§12).
> - `Class.abstract!` forbids instantiating the class itself; concrete subclasses
>   instantiate normally. Independent of sealing.
> - **The value built-ins ship sealed**: `Integer`, `Double`, `Boolean`, `Nil`,
>   `List`, `Map`, `Set`, and `NumberRange`. Open for extension: `String`,
>   `Symbol`, `Bytes`, `Block`, `Object`, the `Error` hierarchy, and every
>   user class that doesn't opt in.
> - **Why**: a sealed method table is a *guarantee* — the optimizer can compile
>   `n + 1` on a known `Integer` down to machine arithmetic only because no code
>   can ever redefine `Integer#+:`. Sealing is what turns type annotations from
>   hints into compiled speed; it's also what makes compile-time MNU provable
>   (§30).

The visible consequence: you cannot monkey-patch or subclass the sealed
built-ins —

```quoin
{ Integer <-- { double -> { self * 2 } } }.catch:{ |e| e.message }
    "* -> 'Cannot extend sealed class [/]Integer'
```

— extend `String` (open) or wrap a sealed type in your own class instead. Your
own classes can buy the same guarantees:

```quoin
Money <- { |@cents|
    cents -> { @cents };
    .sealed!
}

{ Money <-- { inflate -> { 0 } } }.catch:{ |e| e.class.name }    "* -> 'ClassError'
{ Money <- Coupon <- {} }.catch:{ |e| e.class.name }    "* -> 'ClassError'
```

Extension and subclass attempts both raise a typed `ClassError`, so one
`catch:{ |e: ClassError| … }` covers either.

`abstract!` is the other, independent switch — a class that exists only to be
subclassed:

```quoin
Shape <- {
    .abstract!;
    describe -> { 'a ' + .class.name + ' of area ' + .area.s }
}
Shape <- Circle <- { area -> { 314 } }

{ Shape.new }.catch:{ |e| e.message }    "* -> 'Cannot instantiate abstract class [/]Shape'
Circle.new.describe                      "* -> 'a Circle of area 314'
```

---

## 35. Errors at runtime, warnings at compile time

> **Rules**
> - The division of labor:
>
>   | Mistake | When caught | As |
>   |---|---|---|
>   | Reading an unbound name | runtime | `NameError` |
>   | Unknown selector | runtime (compile-time warning when provable, §30) | `MessageNotUnderstood` |
>   | Wrong-typed argument to a typed method | runtime, at dispatch | `MessageNotUnderstood` + candidate list |
>   | Bad insertion into a tagged collection | runtime (warning when statically visible) | `TypeError` |
>   | Everything else the checker sees | compile time | non-fatal warning |
> - Reading a name bound to nothing raises a catchable `NameError` (assigning to
>   an undeclared name was already a compile error — the two halves agree).
> - To ask whether a class exists *without* reading its name, use
>   `Class.exists?:#Name` (a namespaced class needs the quoted symbol form:
>   `Class.exists?:#'[IO]File'`).
> - An uncaught `MessageNotUnderstood` on a multimethod prints the variants that
>   exist but didn't match — read it as "here's what this selector *can* accept."

A misspelling fails at the read, not three calls later as a mysterious `nil`:

```quoin
{ typoedName }.catch:{ |e: NameError| e.message }
    "* -> 'undefined name `typoedName` — nothing with that name is in scope'

Class.exists?:#Integer         "* -> true
Class.exists?:#Wibble          "* -> false
Class.exists?:#'[IO]File'      "* -> true
```

A `NameError` can't be checked at compile time — `use` runs at run time and a
method may name a class defined later in the file, so the read site is the first
place the answer is knowable.

And when typed dispatch rejects every variant, the uncaught error names the
candidates it filtered out. Running the `Describer` of §32 against a `Double`:

```quoin norun
Describer.new.describe:3.14
```

```
VM execution error: Message not understood: receiver=Describer, selector='describe:', args=[Double]
  describe:Integer
  describe:String
  at describe.qn:5:1
  |
  | Describer.new.describe:3.14
  |
```

The summary of the whole chapter is in that table: the **checker** warns early
where types are written, the **runtime** enforces the three real guarantees
(dispatch, tags, seals), and dynamic code sails through both untouched.

---

Next: **[Part VIII — Tooling](08-tooling.md)**.
