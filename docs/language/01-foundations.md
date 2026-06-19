# Part I — Foundations

How BB code is spelled and how the most basic pieces fit together: comments,
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

BB is a small, uniformly object-oriented language in the Smalltalk lineage. If you
can read "send the message `m` to the object `x`" for every `x.m`, you can read
almost all of it. A method with no explicit receiver is sent to `self`, written
with a bare leading dot:

```buildingblocks
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
> - **Keywords**: only `nil`, `true`, `false`. Identifiers are case-sensitive.

### Comments

```buildingblocks
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

```buildingblocks
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
> | Range | `a..b` (half-open) | `1..5`, `5..1` |
> | Regex | `#/…/` | `#/^[a-z]+$/` |
> | User string | `#Name'…'` | `#ANSI'…'` |
> | User list | `#Name( … )` | (mixes in `ActAsUserList`) |
> | Block | `{ … }` | `{ |n| n * 2 }` (Part II) |
> | Booleans / nil | keywords | `true`, `false`, `nil` |

- **Numbers.** Integers have no fractional part; a number is a double only if it
  has a `.` followed by digits. There is no negative *literal* — `-3` is the
  prefix operator `-` applied to `3` (see §6).
- **Strings** use single quotes. Escapes: `\t \n \r \" \' \\`, plus `\uXXXX` and
  `\xXXXX` (four hex digits). Plain strings do **not** interpolate; interpolation
  is a separate `%` form (see [§19](06-library-and-reference.md)).
- **Symbols** are selectors-as-data: `#name`, multi-part `#when:do:`, or a quoted
  form `#'+:'` for operators and otherwise-unspellable names.
- **Lists** are space-separated (no commas): `#(1 2 3)`. **Maps** pair `key: value`
  and are string-keyed: `#{ 'foo': 100 'bar': 200 }`.
- **Ranges** are covered in §6 and Part VI; note they are **half-open** (the end is
  excluded).

> **⚠ Gotcha — `#< … >` set literals are not implemented.** The grammar parses a
> `#< … >` form, but the compiler rejects it (`Unsupported NodeValue: Set`) and the
> program panics. Don't use it; there is no set literal today.

---

## 4. Variables, scope & destructuring

> **Rules**
> - `name = expr` assigns a **local**. Assignment is a **statement, not an expression** — you cannot nest it (`b = (a = 5)` is a parse error) or use it as a condition.
> - Scope is **lexical**; blocks are closures that capture the enclosing scope. A local is created on first assignment in its frame.
> - `_` discards a value on the left-hand side.
> - **Destructuring**: multiple targets pull from a list — `a b c = #(1 2 3)`. One splat `*rest` (or `*_`) may appear in **any** position; sub-patterns nest with `( … )`.
> - `@name` is an **instance variable** (only meaningful inside class/method bodies — Part III). `[Ns]name` / `[/]name` are namespaced globals (§20).
> - `Name <- expr` defines a **constant** (redefining one throws).

```buildingblocks
x = 10
greeting = 'hi'

a b c = #(1 2 3)            "* a=1, b=2, c=3
first *rest = #(1 2 3 4)    "* first=1, rest=#(2 3 4)
a *_ z = #(1 2 3 4 5)       "* a=1, z=5, middle discarded
head (x y) = #(1 #(2 3))    "* head=1, x=2, y=3  (nested)

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

```buildingblocks
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
> - **Prefix** operators are no-argument sends on the operand: `-x`→`negated`, `!x`→`!`, `%x`→`mod`, `+x`→ no-op.
> - **Infix** operators are one-argument sends and are **all left-associative**.
> - Most infix operators are overridable methods on the receiver's type; `&&`/`||` are special short-circuit forms (not method sends).
> - **Precedence, loosest → tightest:** `||` · `&&` · `== !=` · `< <= > >=` · `~` · `* / %` · `+ -` · `..` · `<--`

### Desugaring

| You write | Compiles to | Notes |
|---|---|---|
| `a + b` `a - b` `a * b` `a / b` `a % b` | `Send("+"…)` etc. | routed through a global dispatcher that forwards to the overridable `+:` `-:` `*:` `/:` `%:` method on the receiver |
| `a == b` `a != b` `a < b` `a <= b` `a > b` `a >= b` | `Send("=="…)` etc. | comparison methods |
| `a ~ b` | `Send("~"…)` | the match protocol — checks a custom `~:` on the receiver first (Part IV) |
| `a .. b` | `Send("..:"…)` | builds a `NumberRange` |
| `a && b` `a \|\| b` | short-circuit jumps | **not** method sends; right side is skipped when the left decides the result |
| `-x` | `Send("negated")` | unary minus is `negated`, **not** `-` |
| `!x` | `Send("!")` | boolean negation (defined on `true`/`false`) |

Operators are therefore per-type customizable: define `+:` on your class and `+`
works on its instances.

> **⚠ Gotcha — operator precedence is currently WRONG (known bug).** The precedence
> ordering is non-standard and **will change**. Today:
> - `+` and `-` bind **tighter** than `*` `/` `%`: `2 + 3 * 4` is **20** (`(2+3)*4`), and `2 * 3 + 4` is **14**.
> - Arithmetic binds tighter than comparison: `1 + 2 == 3` is `true`.
> - `..` binds tighter than arithmetic: `2 .. 3 + 1` **errors** (parses as `(2..3) + 1`).
>
> **Until this is fixed, parenthesize every mixed-operator expression.** Tracked in
> `BBLIB_TODO.md` → *Bugs/Odd Behavior* (fix deferred until this doc is complete).

---

Next: **[Part II — Blocks & control flow](02-blocks-and-control.md)** — closures,
`if:`/`whileDo:`, truthiness, and the `^` / `^^` return operators.
