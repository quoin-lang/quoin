# Appendices

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · **Appendices**

---

## A. Sigil & operator cheat-sheet

### Sigils & literal markers

| Sigil | Meaning |
|---|---|
| `"… ` / `"* …` / `"…"` | Comment (line / line / block; block spans newlines) |
| `'…'` | String (the only string literal) |
| `#name` `#a:b:` `#'…'` | Symbol (selector-as-data) |
| `#( … )` | List |
| `#{ k: v … }` | Map |
| `#/ … /` | Regex |
| `#Name'…'` | User string (e.g. `#ANSI'…'`) |
| `#Name( … )` | User list |
| `#< … >` | Set literal (unique elements); a bare `>` ends the set — parenthesize `>`/`>=` elements |
| `@name` | Instance variable |
| `[NS]Name` `[/]Name` | Namespaced global / root global |
| `_` | Ignore (in lvalues and block params) |
| `*name` | Splat (destructuring) |

### Operators

| Operator | Meaning |
|---|---|
| `.` | Message send; leading `.` = send to `self` |
| `name!` `name?` | Selector suffixes (ordinary method names) |
| `<-` | Define class / subclass / constant |
| `<--` | Extend a class or a single value (eigenclass) |
| `->` | Add a method (variant) |
| `-->` | Add a method variant; errors if selector doesn't already exist |
| `use pkg:path;` | Load a `.qn` file once — a soft keyword, not a reserved word (Part IX §47) |
| `^` `^^` `^>` | Block return / method return / yield |
| `=` | Assign a local (statement only) |
| `==` `!=` `<` `<=` `>` `>=` | Comparison |
| `+` `-` `*` `/` `%` | Arithmetic (infix); `%` infix is modulo |
| `&&` `\|\|` | Logical, short-circuit |
| `~` | Match (Part IV) |
| `..` | Range (half-open) |
| `!` (prefix) | Boolean negation |
| `-` (prefix) | Negate (→ the no-arg `-` method) |
| `%` (prefix) | String interpolation (→ `mod`) |
| `+` (prefix) | Identity (→ the no-arg `+` method) |

### Operator precedence

Loosest → tightest, all left-associative:
`||` · `&&` · `== !=` · `< <= > >=` · `~` · `..` · `+ -` · `* / %` · `<--`.
Postfix sends (`.method`) bind tighter than any infix operator; prefix operators
(`-` `!` `%`) bind tightest.

---

## B. Selector / desugaring quick-reference

| Surface form | Compiles to |
|---|---|
| `a + b` (and `- * / %`) | `Send("+:"…)` → overridable `+:` / `-:` / `*:` / `/:` / `%:` method (class-first, no global fallback) |
| `a == b` (and `!= < <= > >=`) | `Send("==:"…)` → overridable `==:` / `!=:` / `<:` / `<=:` / `>:` / `>=:` methods |
| `a ~ b` | `Send("~:"…)` → match protocol (dispatches `~:` on the left operand) |
| `a .. b` | `Send("..:"…)` → `NumberRange` |
| `a && b` / `a \|\| b` | short-circuit jumps (not a method send) |
| `-x` / `+x` | `Send("-")` / `Send("+")` — the no-arg `-` (negate) / `+` (identity) methods |
| `!x` | `Send("!")` |
| `%x` (prefix, on a string) | `Send("mod")` — `%{…}` interpolation |
| `'fmt' % arg` | `Send("%:")` — `printf`-style substitution |
| `obj.sel:arg` | `Send("sel:"…)` |
| `.sel` | send `sel` to `self` |

---

## C. Gotchas for writing & generating Quoin

The consolidated list of surprising behaviors. If you're producing Quoin code, read
this first.

1. **Operator precedence is conventional** (`* / %` tighter than `+ -` tighter than
   comparison tighter than `&&`/`||`), with two specifics: **range `..` is looser
   than arithmetic** (`2 .. n + 1` = `2 .. (n + 1)`), and **postfix `.method` binds
   tighter than any infix operator** (`1 .. list.count` = `1 .. (list.count)`).
2. **`"` always starts a comment** — there are no double-quoted strings. A `"…"`
   block comment spans newlines, so a **stray `"` silently swallows code** until
   the next quote. Strings are `'…'`.
3. **Conditionals are strict; combinators coerce.** `if:`/`else:`/`whileDo:`
   require a real Boolean — `nil.if:{…}`, `42.if:{…}`, and `{5}.whileDo:{…}` all
   raise (`MessageNotUnderstood`). But `&&`/`||` short-circuit on truthiness and
   return the operand value, and `!` maps any value to a Boolean. Use `==`, `<`,
   `defined?`, etc. to build conditions.
4. **Assignment is a statement, not an expression.** `b = (a = 5)` is a parse
   error; you can't assign inside a condition or argument.
5. **Ranges are half-open.** `1..5` yields `1 2 3 4`; the end is excluded (both
   directions: `5..1` → `5 4 3 2`).
6. **`^` returns from the block, `^^` from the method.** Inside an iterator block
   (`each:`, `collect:`), `^` only ends that iteration — use `^^` to break out of
   the surrounding method.
7. **Block arity is unchecked.** Too few arguments → missing params are `nil`; too
   many → extras are ignored. No error either way.
8. **Redefining overrides; type/guard variants coexist.** A later same-signature
   definition (same param types, no guard) *replaces* the earlier — `bar -> {1}`
   then `bar --> {2}` makes `bar` return `2`. Variants that differ by parameter type
   or carry a guard are kept as distinct multimethods, dispatched by argument by
   specificity (a guarded variant outranks an equal-typed unguarded one). Definition
   order is **not** a tiebreaker: two equally-specific variants that both match raise
   `AmbiguousMethodError` — pair specific guards with one unguarded catch-all (`|x|`),
   or use `case`/`~` for ordered matching.
9. **`new:{}` doesn't capture lexical scope, and `super` doesn't exist.** An empty
   `new:{}` leaves fields `nil`; only explicit assignment binds a field (its RHS is
   lexical, but it never mutates the outer variable). A plain-assignment
   `init: { |a| @a = a }` is redundant. A child sets a parent's field via `@field`.
10. **`.sealed!` / `.abstract!` are enforced.** `.sealed!` freezes a class (or instance
    eigenclass) — no `<--`/`->`/`-->`/`.mix:`/subclassing — and `.abstract!` forbids
    instantiating the class itself (subclasses still instantiate). Call `.sealed!` *last*
    in a body, since defs after it are rejected.
11. **`case:` matches with `~`, not `==`.** Ranges, regexes, classes, and predicate
    blocks all match; the first matching `when:` wins. Order clauses
    most-specific-first.
12. **`throw` takes any value, but catch-by-type needs `Error`s.** To dispatch with
    `TypeError ~ e` (the matcher class is on the **left** of `~`), throw `Error`
    subclasses (or use `Error.throw:`).
13. **`%` has three meanings.** Infix between numbers = modulo; infix on a string =
    `printf`-style `%:`; prefix on a string = `%{…}` interpolation (`mod`).
14. **`<-` vs `=`.** `<-` defines a once-only constant/class (redefining throws);
    `=` is a reassignable local. They're not interchangeable.
15. **Fibers throw on misuse.** Resuming a `done`/`failed` fiber, yielding outside a
    fiber, or self-resuming all raise `FiberError`. Guard with `alive?`/`done?`.
16. **A leading `.` or operator continues the previous line.** A newline ends a
    statement only when unambiguous; a line starting with `.` (a message send) or an
    infix operator attaches to the line above. `method -> {}` ⏎ `.mix:Mixin` parses
    as `(method -> {}).mix:Mixin`. End the previous line with `;` when the next
    starts with `.` or an operator — which is why stdlib code uses `;` heavily.
17. **`use` loads files; it doesn't import names.** `use path;` runs a `.qn` file once
    (a repeat/cyclic `use` is a no-op), and its definitions land as ordinary `[Ns]`
    globals — there's no local import scope, and aliasing is just `X = [Ns]Name`. `use`
    is a *soft keyword* (still usable as an identifier). Packages: bare/`std:` = stdlib,
    `self:` = your project; `dir/*` globs a directory (sorted). See §47.
18. **Inside a collection literal, spacing decides whether `+ - %` is a prefix or an infix
    operator.** Elements are juxtaposed expressions (so `#(a + b  c)` is two elements),
    which makes `#(-1 -2)` ambiguous. The rule, in `#( … )`, `#Name( … )` and `#< … >`: an
    operator **detached from its left operand and glued to its right one** is a prefix and
    starts a new element. Every other spacing is infix.

    | | |
    |---|---|
    | `#(1 -2)`, `#(1 +2)`, `#('a' %'b')` | two elements |
    | `#(1 - 2)`, `#(1-2)`, `#(1- 2)` | one element — subtraction |

    So `#(-1 -2)` is two negatives, `#(5-3)` is `#(2)`, and `#('a' %'b%{x}')` is a string
    plus an interpolation. Identifiers follow the same rule (`#(a -b)` is two elements),
    and the prefix binds tighter than the infix after it, so `#(1 -2 + 3)` is `#(1 1)`.
    `!` needs no rule: its infix form is the two-character `!=`. Parenthesize when in
    doubt: `#((7-3))`.

---

## D. Glossary

- **Selector** — a method name, including its colons: `at:put:` is one selector
  with two argument slots. Operators are selectors too (`+:`, `~:`).
- **Multimethod** — several definitions of one selector distinguished by argument
  type or guard; dispatch picks the most specific match (an equally-specific tie that
  both match → `AmbiguousMethodError`).
- **Eigenclass / singleton** — a per-object class created by `value <-- { … }`,
  holding methods for just that one object (named `$Type` internally).
- **Mixin** — a class included into another with `.mix:`; its methods and
  instance vars participate in lookup (before the parent).
- **Block** — a first-class closure `{ … }`; the unit of deferred code used for
  control flow, iteration, and initialization.
- **Block-local** — a variable declared after `-` in a block header
  (`{ |args - locals| }`), initialized to `nil`.
- **Fiber** — a stackful coroutine; `Generator` and the external `Iterator` are
  built on fibers.
- **Half-open range** — a range that includes its start but excludes its end.
- **Namespaced name** — a global addressed as `[NS]Name`; bare names and `[/]Name`
  live in the root namespace.
- **Unit / package** — a `.qn` file loaded by `use pkg:path` (once, via the host's
  resolver). `std:`/bare = the stdlib, `self:` = the project. The load path is decoupled
  from the `[Ns]` namespace a unit registers under. (§47)

---

[Back to the index](README.md)
