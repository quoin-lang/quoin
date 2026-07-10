# Part IV — Patterns & errors

Matching values with `case`/`~`, destructuring with `.bind:`, and raising and
catching errors.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · **Patterns & errors** · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · [Appendices](10-appendices.md)

---

## 14. Pattern matching & `case`

> **Rules**
> - `subject.case:{ .when:cond do:result; … .default:fallback }` — tests each `cond` against `subject` with the `~` operator; the **first match wins**. With no match and no `default:`, the result is `nil`. Each arm is an ordinary statement, so end it with `;` — a following line that starts with `.when:`'s leading dot would otherwise continue the previous arm (§2).
> - `do:` (and `default:`) accept either a **block** (the block receives the subject as its argument) or a **plain value** (used as the result).
> - The **`~` match protocol**: `a ~ b` is `a.~:(b)` — the matcher is the **left** operand, so dispatch is class-first on `a`'s `~:` (define `~:` on your own class to customize). Built-in matchers: a **Class** tests is-instance-of (`Integer ~ 5`), a **Regex** tests a match against the string (`#/…/ ~ str`), a **Block** runs as a predicate over `b`, a **range** tests membership, and the default `Object#~:` is `==:` equality. (Because the matcher is on the left, `case` puts the `cond` first: `cond ~ subject`.)

```quoin
var score = 87
var grade = score.case:{
    .when:(90..101) do:'A';                "* range membership
    .when:(80..90)  do:'B';
    .when:{ |n| n < 0 } do:'invalid';      "* predicate block, gets the subject
    .default:'F'
}
grade;                                     "* -> 'B'
var name = 'Ada'
name.case:{
    .when:#/^[A-Z]+$/ do:{ 'shouting'.print };   "* regex match
    .when:'Ada'       do:{ 'hi Ada'.print };     "* equality
    .default:{ 'unknown'.print }
}
```

The same `~` operator works standalone, with the matcher on the left:
`(1..10) ~ 5`, `#/b/ ~ 'abc'`, `TypeError ~ value`, `{ |n| n > 0 } ~ x`.

> **⚠ Gotcha — `case:` matches with `~`, not `==`.** A `when:` clause succeeds
> whenever `cond ~ subject` is truthy, so ranges, regexes, classes, and predicate
> blocks all "match" — not just equal values. Order your clauses most-specific
> first, since the first match wins.

---

## 15. Errors & stack traces

> **Rules**
> - `value.throw` throws **any value**. The `Error` classes add class-side convenience constructors: `Error.throw:'msg'` and `Error.throw:'msg' payload:p` build an instance and throw it.
> - `{ … }.catch:{ |e| … }` runs the receiver block; if it throws, the thrown value is passed to the catch block, whose result becomes the value. `{ … }.catch:{ |e| … } finally:{ … }` additionally runs `finally:` **always** (on success or failure).
> - **Typed catch.** A typed handler param — `catch:{ |e:IoError| … }` — only catches when the thrown value is (a subtype of) that type; a non-match **re-raises** to an enclosing `catch:`. An untyped `|e|` (≡ `|e:Object|`) is a catch-all.
> - **Multiple handlers by type.** Chain `catch:` keywords: `{ … }.catch:{ |e:IoError| … } catch:{ |e:Error| … } finally:{ … }`. Handlers are tried in **source order, first match wins** — so write them most-specific → least-specific, with any untyped catch-all **last** (a broad handler placed first shadows the narrower ones below it). This first-match ordering is a deliberate exception to Quoin's otherwise order-independent multimethod dispatch: a handler's type lives on a runtime block, not a scored method chain, so there is no specificity order to fall back on. (Inside a single handler you can still branch with `case`/`~`: `e.case:{ .when:TypeError do:… }`.)
> - **Built-in hierarchy** (`core/00-bootstrap.qn`): `Error` with `@message @payload`, accessors `message`/`payload`, and `s` (→ `'ClassName: message'`); subclasses `TypeError`, `ArgumentError`, `MessageNotUnderstood`, `AmbiguousMethodError`, `ArithmeticError`, `IndexError`, `FiberError`.
> - **Runtime errors are structured**: the VM maps its internal errors to these Quoin `Error` objects at the `catch:` boundary, so you can catch and inspect them.

```quoin
var amount = -5
var result = {
    (amount < 0).if:{ ArgumentError.throw:'amount must be >= 0' };
    .process:amount                "* reached only when the check passes
}.catch:{ |e:ArgumentError| ('bad input: ' + e.message).print; 0 }
 catch:{ |e:IoError|        ('io failed: ' + e.message).print; -1 }
 finally:{ 'done'.print }
"* anything that isn't an ArgumentError or IoError re-raises automatically —
"* most-specific handler first, no explicit re-throw needed.
result                             "* -> 0
```

Internal failures surface as the matching Quoin error type — e.g. an out-of-range
index or a type mismatch becomes a catchable `TypeError`/`IndexError`, and sending
an unknown selector becomes a `MessageNotUnderstood` — each with a `message` you
can read.

> **⚠ Gotcha — `throw` accepts any value; it types by its actual class.**
> `42.throw` is caught by `catch:{ |e:Integer| … }` (a thrown value matches by its
> class), but **not** by `catch:{ |e:Error| … }` — `42` isn't an `Error`. Throw
> `Error` subclasses (or use the `Error.throw:` constructors) when handlers should
> dispatch on the error hierarchy.

> Stack traces: uncaught errors print a highlighted trace (with source snippets).
> The mechanics are an implementation detail; nothing in the language surface
> depends on them.

---

Next: **[Part V — Concurrency & iteration](05-concurrency-and-iteration.md)**.
