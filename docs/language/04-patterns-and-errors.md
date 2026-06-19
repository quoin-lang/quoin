# Part IV — Patterns & errors

Matching values with `case`/`~`, destructuring with `.bind:`, and raising and
catching errors.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · **Patterns & errors** · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Library & reference](06-library-and-reference.md) · [Appendices](07-appendices.md)

---

## 14. Pattern matching & `case`

> **Rules**
> - `subject.case:{ .when:cond do:result … .default:fallback }` — tests each `cond` against `subject` with the `~` operator; the **first match wins**. With no match and no `default:`, the result is `nil`.
> - `do:` (and `default:`) accept either a **block** (the block receives the subject as its argument) or a **plain value** (used as the result).
> - The **`~` match protocol** (`a ~ b`) is tried in this order: a custom `~:` method on `a` → a block predicate on either side → a class/type test (`Class ~ value` ⇒ is-instance-of) → regex (`regex ~ string`, or `string ~ regex`) → range membership (via the range's `~:`) → `==:` equality fallback.

```buildingblocks
grade = score.case:{
    .when:(90..101) do:'A'                 "* range membership
    .when:(80..90)  do:'B'
    .when:{ |n| n < 0 } do:'invalid'       "* predicate block, gets the subject
    .default:'F'
}

name.case:{
    .when:#/^[A-Z]+$/ do:{ 'shouting'.print }   "* regex match
    .when:'Ada'       do:{ 'hi Ada'.print }     "* equality
    .default:{ 'unknown'.print }
}
```

The same `~` operator works standalone: `5 ~ (1..10)`, `'abc' ~ #/b/`,
`value ~ TypeError`, `x ~ { |n| n > 0 }`.

> **⚠ Gotcha — `case:` matches with `~`, not `==`.** A `when:` clause succeeds
> whenever `cond ~ subject` is truthy, so ranges, regexes, classes, and predicate
> blocks all "match" — not just equal values. Order your clauses most-specific
> first, since the first match wins.

---

## 15. Errors & stack traces

> **Rules**
> - `value.throw` throws **any value**. The `Error` classes add class-side convenience constructors: `Error.throw:'msg'` and `Error.throw:'msg' payload:p` build an instance and throw it.
> - `{ … }.catch:{ |e| … }` runs the receiver block; if it throws, the thrown value is passed to the catch block, whose result becomes the value. `{ … }.catch:{ |e| … } finally:{ … }` additionally runs `finally:` **always** (on success or failure).
> - **Catch by type** with `case`/`~` inside the handler: `e.case:{ .when:TypeError do:… }`.
> - **Built-in hierarchy** (`00-bootstrap.bub`): `Error` with `@message @payload`, accessors `message`/`payload`, and `s` (→ `'ClassName: message'`); subclasses `TypeError`, `ArgumentError`, `MessageNotUnderstood`, `ArithmeticError`, `IndexError`, `FiberError`.
> - **Runtime errors are structured**: the VM maps its internal errors to these BB `Error` objects at the `catch:` boundary, so you can catch and inspect them.

```buildingblocks
result = {
    (amount < 0).if:{ ArgumentError.throw:'amount must be >= 0' }
    process:amount
}.catch:{ |e|
    e.case:{
        .when:ArgumentError do:{ ('bad input: ' + e.message).print; 0 }
        .default:{ e.throw }                  "* re-throw what we don't handle
    }
} finally:{
    'done'.print
}
```

Internal failures surface as the matching BB error type — e.g. an out-of-range
index or a type mismatch becomes a catchable `TypeError`/`IndexError`, and sending
an unknown selector becomes a `MessageNotUnderstood` — each with a `message` you
can read.

> **⚠ Gotcha — `throw` accepts any value, but typed catching expects `Error`s.**
> You *can* `42.throw`, but catch-by-type (`e ~ TypeError`) only works when the
> thrown value is an `Error` instance. Throw `Error` subclasses (or use the
> `Error.throw:` constructors) if handlers will dispatch on type.

> Stack traces: uncaught errors print a highlighted trace (with source snippets).
> The mechanics are an implementation detail; nothing in the language surface
> depends on them.

---

Next: **[Part V — Concurrency & iteration](05-concurrency-and-iteration.md)**.
