# Part I — Foundations

How Quoin code is spelled and how the most basic pieces fit together: comments,
literals, names, message sends, and operators. Read this first; everything else
builds on it.

> Each section opens with a **Rules** box (the terse version for lookup) followed
> by prose and examples. **⚠ Gotcha** boxes flag behavior that is surprising or a
> common source of wrong code. Every claim here is verified against the pest
> grammar (`src/parser/pest/`), the VM (`src/vm.rs`), and the test suite.

Nav: **Foundations** · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Library & reference](06-library-and-reference.md) · [Appendices](07-appendices.md)

---

## 1. Mental model

> **Rules**
> - Everything is an **object**. All computation is **messages** (method calls) sent to objects with `.`.
> - A leading dot with no receiver — `.foo` — means `self.foo`.
> - Classes are defined with `<-`, methods with `->`. Blocks `{ … }` are first-class closures.
> - There are **no control-flow keywords**. `if:`, `whileDo:`, `case:` are ordinary methods.
> - Comments start with `"`. Strings are single-quoted `'…'`. There are **no** double-quoted strings.
> - A newline ends a statement when unambiguous; use `;` when the next line would otherwise continue the expression (i.e. it starts with `.` or an operator).

Quoin is a small, uniformly object-oriented language in the Smalltalk lineage. If you
can read "send the message `m` to the object `x`" for every `x.m`, you can read
almost all of it. A method with no explicit receiver is sent to `self`, written
with a bare leading dot:

```quoin
.print: 'hello'        "* sends print: to self
person.greet           "* sends greet to person
(1..5).collect:{ |n| n * 10 }
```

Because control flow is just methods (Part II), there is very little dedicated
syntax to memorize — most of this document is about *which messages exist*, not
about statements.

---

## 2. Lexical structure

> **Rules**
> - **Comments** (three forms):
>   - `"* …` — line comment, runs to end of line.
>   - `"…"` — block comment; **may span multiple lines**; ends at the next unescaped `"`.
>   - `""` — empty comment.
> - **No double-quoted strings exist** — a `"` always begins a comment.
> - **Separators**: a newline ends a statement *when unambiguous*. A `;` is required when the next line would otherwise continue the expression — i.e. it begins with `.` (a message send) or an infix operator.
> - **Identifiers**: start with a letter or `_`, then letters/digits/`_`/`?`. So `done?` and `my_var` are valid names. `!` is *not* part of a name (it's a selector suffix — see §5).
> - **Reserved identifiers**: only `nil`, `true`, `false` (can't be reassigned). The *keywords* are all soft keywords (reserved only as a statement prefix, so ordinary uses of the word are unaffected): `use` (§21) and `var`/`let` (local declarations; §4). Identifiers are case-sensitive.

### Comments

```quoin
"* A line comment — everything to the end of this line is ignored.

"
A block comment. It opens with a quote that is NOT followed by '*',
spans as many lines as you like, and closes at the next quote.
"

x = 1 "this trailing block comment ends here" + 2   "* x is 3
```

> **⚠ Gotcha — a stray `"` swallows code.** Because the `"…"` block comment runs
> until the *next* `"`, an unbalanced quote silently comments out everything up to
> the following quote (possibly many lines away). If a chunk of code mysteriously
> "doesn't run," look for an earlier lone `"`. There is no double-quoted string to
> fall back on — `"` is always a comment.

### Separators

A newline ends a statement **when the result is unambiguous**. A `;` is needed
when the next line would otherwise continue the previous expression —
specifically when it begins with `.` (a message send) or an infix operator:

```quoin
A <- {
    method -> {};          "* the ; ends the statement
    .mix:Mixin
}
```

Without the `;` on the `method` line, this parses as `(method -> {}).mix:Mixin` —
the `.mix:` send attaches to the method definition instead of starting a new
statement.

> **⚠ Gotcha — a leading `.` or operator continues the previous line.** A line that
> starts with `.` (a message send) or an infix operator (`+`, `-`, …) is read as a
> continuation of the line above, not a new statement. Terminate the previous line
> with `;` in those cases. (This is why stdlib code ends most lines with `;`.)

---

## 3. Literals & data types

> **Rules**
>
> | Kind | Syntax | Example |
> |---|---|---|
> | Integer | digits (`0`, or `1`–`9` then digits) | `42`, `1000000` |
> | Double | digits with a `.` and fractional digits | `3.14`, `42.0`, `.5` |
> | String | `'…'` (single quotes only) | `'hello'` |
> | Symbol | `#name`, `#multi:part:`, `#'…'` | `#x`, `#when:do:`, `#'+:'` |
> | List | `#( … )` space-separated | `#(1 2 3)`, `#()` |
> | Map | `#{ key: value … }` | `#{ 'a': 1 'b': 2 }` |
> | Set | `#< … >` space-separated, unique | `#<1 2 3>`, `#<>` |
> | Range | `a..b` (half-open) | `1..5`, `5..1` |
> | Regex | `#/…/` | `#/^[a-z]+$/` |
> | User string | `#Name'…'` | `#ANSI'…'` |
> | User list | `#Name( … )` | (mixes in `ActAsUserList`) |
> | Block | `{ … }` | `{ |n| n * 2 }` (Part II) |
> | Booleans / nil | reserved identifiers | `true`, `false`, `nil` |

- **Numbers.** Integers have no fractional part; a number is a double only if it
  has a `.` followed by digits. There is no negative *literal* — `-3` is the
  prefix operator `-` applied to `3` (see §6).
- **Strings** use single quotes. Escapes: `\t \n \r \" \' \\`, plus `\uXXXX` and
  `\xXXXX` (four hex digits). Plain strings do **not** interpolate; interpolation
  is a separate `%` form (see [§19](06-library-and-reference.md)).
- **Symbols** are interned selector-like names: `#name`, multi-part `#when:do:`,
  or a quoted form `#'+:'` for operators and otherwise-unspellable names. They are a
  **distinct type** (`#foo.class == Symbol`), compared by identity — `#foo == #foo`
  is true, but `#foo == 'foo'` is **false**; `#foo.s` yields the name `'foo'`.
  `Block#name` and `Method#selector` (alias `Method#name`) return symbols.
- **Lists** are space-separated (no commas): `#(1 2 3)`. **Maps** pair `key: value`
  and are string-keyed: `#{ 'foo': 100 'bar': 200 }`. **Sets** are space-separated
  and hold unique elements (deduplicated by `==:`): `#<1 2 3>`, empty `#<>`.
- **Ranges** are covered in §6 and Part VI; note they are **half-open** (the end is
  excluded).

> **⚠ Gotcha — inside `#< … >`, a bare `>` ends the set.** Because the closing `>`
> would otherwise collide with the greater-than operator, `>` and `>=` are not
> treated as operators inside a set literal — the first bare `>` terminates it. To
> use them in an element, parenthesize: `#<(a > b) c>` is a two-element set. Every
> other operator works unparenthesized (`#<a + b  c>`).

---

## 4. Variables, scope & destructuring

> **Rules**
> - **Declare a local with `var` (mutable) or `let` (immutable)**, always with an initializer: `var x = 5`, `let pi = 3.14`. A plain `name = expr` **reassigns** an already-declared local — assigning an *undeclared* name, or reassigning a `let`, is a compile error. (`var`/`let` are soft keywords, like `use`: `variable`/`letter` are still ordinary identifiers.)
> - Declaration/assignment is a **statement, not an expression** — you cannot nest it (`b = (a = 5)` is a parse error) or use it as a condition.
> - Scope is **lexical**; blocks are closures that capture the enclosing scope. `var`/`let` may **shadow** an outer binding but cannot redeclare a name in the same scope. A recursive reference works — `var f = { … f … }` sees its own name.
> - A single-target declaration may carry a **type**: `var n: Integer = 5` (drives the typed/unboxed tier). The type may be namespaced — `var f: [IO]File = …`; a bare name means the root namespace. Destructuring targets are untyped.
> - `_` discards a value on the left-hand side.
> - **Destructuring**: `var` declares multiple targets from a list — `var a b c = #(1 2 3)`. One splat `*rest` (or `*_`) may appear in **any** position; sub-patterns nest with `( … )`. Plain (keyword-less) `a b c = …` reassigns already-declared targets.
> - `@name` is an **instance variable** (only meaningful inside class/method bodies — Part III; declared in the class header). `[Ns]name` / `[/]name` are namespaced globals (§20).
> - `Name <- expr` defines a **constant** (redefining one throws).

```quoin
var x = 10
let greeting = 'hi'
x = x + 1                   "* reassign a `var`

var n: Integer = 42         "* typed local

var a b c = #(1 2 3)        "* a=1, b=2, c=3
var first *rest = #(1 2 3 4) "* first=1, rest=#(2 3 4)
var a *_ z = #(1 2 3 4 5)   "* a=1, z=5, middle discarded
var head (x2 y) = #(1 #(2 3)) "* head=1, x2=2, y=3  (nested)

Pi <- 3.14159               "* a constant; a second `Pi <- …` would throw
```

Only one splat is allowed per pattern, but it may lead, sit in the middle, or
trail. `_` (and `*_`) ignore the corresponding element(s).

> **⚠ Gotcha — assignment is statement-only.** `a = 5` does not produce a value you
> can feed into another expression. Write conditions and arguments as plain
> expressions; do the assignment on its own line.

---

## 5. Messages & call syntax

> **Rules**
> - **Unary** send: `receiver.selector` → `42.abs`, `list.first`.
> - **Keyword** send: `receiver.selector:arg`. Multi-part selectors are *one* name: `map.at:'k' put:v` sends `at:put:`.
> - A keyword argument is a **whole expression** (greedy) — `obj.m: 1 + 2` passes `1 + 2`.
> - **Leading dot** `.selector` sends to `self`.
> - **Suffix selectors**: `name!` (e.g. `.sealed!`) and `name?` (e.g. `fiber.done?`) are ordinary method names.
> - Method (postfix) sends bind **tighter** than infix operators: `a.b + c.d` is `(a.b) + (c.d)`.

```quoin
42.abs                         "* unary
'a,b,c'.split:','              "* one keyword arg
scores.at:'amy' put:95         "* selector is at:put:, two args
.print:'sum =' and:(2 + 2)     "* leading dot = self; parenthesize operator args
```

Because a keyword argument greedily consumes a full expression, you usually
parenthesize an operator expression that you want to pass as a single argument
(as with `(2 + 2)` above). Multi-part selectors let methods read like prose:
`coll.when:cond do:action` is a single send of `when:do:`.

---

## 6. Operators & precedence

> **Rules**
> - **Prefix** operators are no-argument sends on the operand: `-x`→`-`, `+x`→`+` (identity), `!x`→`!`, `%x`→`mod`.
> - **Infix** operators are one-argument sends and are **all left-associative**.
> - Most infix operators are overridable methods on the receiver's type; `&&`/`||` are special short-circuit forms (not method sends).
> - **Precedence, loosest → tightest:** `||` · `&&` · `== !=` · `< <= > >=` · `~` · `..` · `+ -` · `* / %` · `<--`. Postfix sends (`.method`) bind tighter than any infix operator.

### Desugaring

| You write | Compiles to | Notes |
|---|---|---|
| `a + b` `a - b` `a * b` `a / b` `a % b` | `Send("+:"…)` etc. | the overridable `+:` `-:` `*:` `/:` `%:` method on the receiver's type (resolved class-first; no global fallback) |
| `a == b` `a != b` `a < b` `a <= b` `a > b` `a >= b` | `Send("==:"…)` etc. | overridable `==:` `!=:` `<:` `<=:` `>:` `>=:` methods |
| `a ~ b` | `Send("~:"…)` | the match protocol — dispatches `~:` on the left operand (Part IV) |
| `a .. b` | `Send("..:"…)` | builds a `NumberRange` |
| `a && b` `a \|\| b` | short-circuit jumps | **not** method sends; right side is skipped when the left decides the result |
| `-x` | `Send("-")` | unary minus is the no-arg `-` method (binary `-` is `-:`); `+x` is `Send("+")`, the identity `+` method |
| `!x` | `Send("!")` | boolean negation (`Object#'!'` / `Nil#'!'` / the booleans) |

Operators are therefore per-type customizable: define `+:` on your class and `+`
works on its instances.

> **Note — precedence is conventional, with two specifics worth knowing.**
> Multiplicative (`* / %`) binds tighter than additive (`+ -`), which binds tighter
> than comparison, which binds tighter than `&&`/`||` — as you'd expect, so
> `2 + 3 * 4` is `14` and `1 + 2 == 3` is `true`. Beyond that: **range `..` is looser
> than arithmetic**, so `2 .. n + 1` means `2 .. (n + 1)`; and **postfix `.method`
> binds tighter than every infix operator**, so `1 .. list.count` is
> `1 .. (list.count)` and `a.x * b.y` is `(a.x) * (b.y)`.

---

Next: **[Part II — Blocks & control flow](02-blocks-and-control.md)** — closures,
`if:`/`whileDo:`, truthiness, and the `^` / `^^` return operators.
