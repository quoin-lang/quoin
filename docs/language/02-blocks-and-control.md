# Part II — Blocks & control flow

Blocks are Quoin's closures, and they're also how all control flow works — there are
no `if`/`while` statements, only messages sent to booleans and blocks.

Nav: [Foundations](01-foundations.md) · **Blocks & control** · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Library & reference](06-library-and-reference.md) · [Appendices](07-appendices.md)

---

## 7. Blocks & closures

> **Rules**
> - A block is `{ … }`. Parameters: `{ |a b| … }`; type hints (optional): `{ |a:Integer b| … }` — a hint may be namespaced (`{ |e:[Web]Halt| … }`); ignore a param with `_`.
> - **Block-locals**: names after a `-` in the header are locals initialized to `nil` — `{ |a b - x y| … }` declares params `a b` and locals `x y`.
> - **Named block**: `{ #name |…| … }` attaches a debug name, readable via `.name`.
> - **Invoke**: `.value` (0 args), `.value:arg` (1 arg), `.valueWithArgs:#(…)` (N args). Also `.arity` (param count) and `.args` (param names).
> - Blocks are **closures** capturing a *live reference* to the enclosing scope — later mutations of an outer local are visible inside the block.
> - Calling with the **wrong number of args is not an error**: extra args are ignored; missing params read as `nil`.

```quoin
double = { |n| n * 2 }
double.value:21                  "* 42

adder = { |a b| a + b }
adder.valueWithArgs:#(3 4)       "* 7

{ |a b| a + b }.arity            "* 2
{ #greet |x| 'hi ' + x }.name    "* 'greet'
```

Closures capture the live environment, so a block can see — and call — names that
change after it was created (this is how recursive named blocks work):

```quoin
count = 0
bump = { count = count + 1 }
bump.value
bump.value
count                            "* 2
```

> **⚠ Gotcha — arity is not checked.** `{ |a b| … }.value:1` runs with `b` bound to
> `nil`; `{ |a| … }.valueWithArgs:#(1 2 3)` runs ignoring `2` and `3`. No error is
> raised either way, so an arity mismatch shows up only as an unexpected `nil` or a
> dropped argument.

Other invocation selectors exist for binding a receiver as well as arguments —
`valueWithSelf:`, `value:withSelf:`, `valueWithSelfOrArg:` — these are mostly used
by the iteration protocol (Part V) to pass each element as both `self` and the
block argument.

---

## 8. Control flow is a library, not syntax

> **Rules**
> - `if:`, `else:`, `if:else:`, and `not` are methods defined **only on `true` and `false`** (see `core/00-bootstrap.qn`). `nil` has none of them.
> - **Conditionals are strict**: `if:`/`else:`/`if:else:` and `whileDo:` require an actual Boolean condition — sending one to a non-Boolean (including `nil`) is a `MessageNotUnderstood`. There is no truthiness coercion for these.
> - **Combinators do coerce truthiness** (falsy = `false` or `nil`, everything else truthy): `&&` and `||` short-circuit and return the *operand value* (`7 || false` → `7`, `nil && x` → `nil`), and `!` (via `Object#'!'`/`Nil#'!'`) maps any value to a Boolean (`!5` → `false`, `!nil` → `true`).
> - The `if:`/`else:` blocks are zero-arg; they run via `.value`.
> - **Loops** are methods on a *block* used as the condition:
>   - `{ cond }.whileDo:{ body }` — re-evaluates `cond` (must yield `true`/`false`) before each iteration.
>   - `{ cond }.whileDefinedDo:{ |v| body }` — loops while `cond`'s value is `defined?` (non-`nil`), passing that value into the body.

```quoin
(score > 90).if:{ 'A'.print } else:{ 'not yet'.print }

(x == nil).if:{ 'missing'.print }     "* compare to produce a boolean first

i = 1
{ i <= 3 }.whileDo:{ i.print; i = i + 1 }
```

Because conditionals are just messages, the receiver of `.if:` must already be a
boolean. Comparison operators (`==`, `<`, …) and predicate methods (`defined?`,
`contains?:`, …) are how you produce one.

> **⚠ Gotcha — `nil.if:` is an error, not "false".** Many languages treat `nil` as
> falsy; Quoin does not. `maybe.if:{ … }` throws `MessageNotUnderstood` when `maybe`
> is `nil` (or any non-boolean). Guard with an explicit test:
> `maybe.defined?.if:{ … }` or `(maybe == x).if:{ … }`.

---

## 9. Returns & non-local return

> **Rules**
> - A block (and a method body) evaluates to its **last expression** — no return keyword needed.
> - `^ expr` — **block return**: returns from the *current block*. At a method's top level the body *is* the method's block, so `^` there exits the method; inside a nested block it exits only that block.
> - `^^ expr` — **method return** (non-local): unwinds through any intervening blocks and returns from the enclosing method. This is how you break out of an iterator's body.
> - `^> expr` — **yield**: sugar for `Fiber.yield:expr` (Part V).

```quoin
firstBig -> { |list|
    list.each:{ |n|
        (n > 100).if:{ ^^ n }      "* ^^ returns from firstBig, ending the loop
    };
    nil                            "* fell through: nothing big
}
```

Inside the `each:` block, `^ n` would merely end that one iteration of the block
(returning `n` as the block's value, which `each:` discards) — the loop would
continue. `^^ n` is what actually exits `firstBig`. The standard `whileDo:` is
itself defined using `^^` to unwind its recursion.

> **⚠ Gotcha — `^` inside an iterator block does not break the loop.** Use `^^` to
> return out of the surrounding method from within a block passed to `each:`,
> `collect:`, etc. `^` only finishes the current block invocation.

---

Next: **[Part III — Objects](03-objects.md)**.
