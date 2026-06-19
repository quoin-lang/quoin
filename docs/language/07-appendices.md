# Appendices

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Library & reference](06-library-and-reference.md) · **Appendices**

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
| `#< … >` | Set literal — **parses but unimplemented** (compile error) |
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
| `^` `^^` `^>` | Block return / method return / yield |
| `=` | Assign a local (statement only) |
| `==` `!=` `<` `<=` `>` `>=` | Comparison |
| `+` `-` `*` `/` `%` | Arithmetic (infix); `%` infix is modulo |
| `&&` `\|\|` | Logical, short-circuit |
| `~` | Match (Part IV) |
| `..` | Range (half-open) |
| `!` (prefix) | Boolean negation |
| `-` (prefix) | Negate (→ `negated`) |
| `%` (prefix) | String interpolation (→ `mod`) |
| `+` (prefix) | No-op |

### Operator precedence

Loosest → tightest, all left-associative:
`||` · `&&` · `== !=` · `< <= > >=` · `~` · `..` · `+ -` · `* / %` · `<--`.
Postfix sends (`.method`) bind tighter than any infix operator; prefix operators
(`-` `!` `%`) bind tightest.

---

## B. Selector / desugaring quick-reference

| Surface form | Compiles to |
|---|---|
| `a + b` (and `- * / %`) | `Send("+"…)` → overridable `+:` / `-:` / `*:` / `/:` / `%:` method |
| `a == b` (and `!= < <= > >=`) | `Send("=="…)` etc. |
| `a ~ b` | `Send("~"…)` → match protocol (custom `~:` first) |
| `a .. b` | `Send("..:"…)` → `NumberRange` |
| `a && b` / `a \|\| b` | short-circuit jumps (not a method send) |
| `-x` | `Send("negated")` |
| `!x` | `Send("!")` |
| `%x` (prefix, on a string) | `Send("mod")` — `%{…}` interpolation |
| `'fmt' % arg` | `Send("%:")` — `printf`-style substitution |
| `obj.sel:arg` | `Send("sel:"…)` |
| `.sel` | send `sel` to `self` |

---

## C. Gotchas for writing & generating BB

The consolidated list of surprising behaviors. If you're producing BB code, read
this first.

1. **Operator precedence is conventional** (`* / %` tighter than `+ -` tighter than
   comparison tighter than `&&`/`||`), with two specifics: **range `..` is looser
   than arithmetic** (`2 .. n + 1` = `2 .. (n + 1)`), and **postfix `.method` binds
   tighter than any infix operator** (`1 .. list.count` = `1 .. (list.count)`).
2. **`"` always starts a comment** — there are no double-quoted strings. A `"…"`
   block comment spans newlines, so a **stray `"` silently swallows code** until
   the next quote. Strings are `'…'`.
3. **No truthiness.** `if:`/`else:` exist only on `true`/`false`. `nil.if:{…}` and
   `42.if:{…}` are `MessageNotUnderstood`. Conditions must be real booleans — use
   `==`, `<`, `defined?`, etc.
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
   or carry a guard are kept as distinct multimethods, dispatched by argument;
   equal-specificity **guarded** variants are tried in definition order, so define
   specific guards before a catch-all.
9. **`new:{}` doesn't capture lexical scope, and `super` doesn't exist.** An empty
   `new:{}` leaves fields `nil`; only explicit assignment binds a field (its RHS is
   lexical, but it never mutates the outer variable). A plain-assignment
   `init: { |a| @a = a }` is redundant. A child sets a parent's field via `@field`.
10. **Some surface forms are stubs.** `.sealed!` is a no-op, `.can?:` is not
    implemented, and the `#< >` set literal fails to compile.
11. **`case:` matches with `~`, not `==`.** Ranges, regexes, classes, and predicate
    blocks all match; the first matching `when:` wins. Order clauses
    most-specific-first.
12. **`throw` takes any value, but catch-by-type needs `Error`s.** To dispatch with
    `e ~ TypeError`, throw `Error` subclasses (or use `Error.throw:`).
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

---

## D. Glossary

- **Selector** — a method name, including its colons: `at:put:` is one selector
  with two argument slots. Operators are selectors too (`+:`, `~:`).
- **Multimethod** — several definitions of one selector distinguished by argument
  type or guard; dispatch picks the most specific match (ties → first defined).
- **Eigenclass / singleton** — a per-object class created by `value <-- { … }`,
  holding methods for just that one object (named `$Type` internally).
- **Mixin** — a class included into another with `.mix:` / `.can:`; its methods and
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

---

[Back to the index](README.md)
