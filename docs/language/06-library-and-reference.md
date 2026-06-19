# Part VI — Library & reference

Brief reference for the core types, string formatting, namespaces, and a map of
the standard library. For method-level detail, the stdlib `.bub` files and the
Rust `src/runtime/*.rs` modules are the source of truth — this part points you at
them rather than duplicating them.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · **Library & reference** · [Appendices](07-appendices.md)

---

## 18. Collections & core types

> **Rules**
> - These are brief, indicative lists — see the cited files for the full, current set and exact semantics.
> - Anything in the `Iterate` mixin (Part V) is also available on every iterable type below.

**String** (`src/runtime/string.rs`, `bblib/04-string.bub`) — `length`, `s`,
`contains?:`, `starts?:`, `ends?:`, `index:`, `insert:at:`, `lower`, `upper`,
`replace:with:`, `split:` (String or Regex), `to_integer`, `==:`, `<`, `>`,
`%:` (formatting, §19), `mod` (interpolation, §19).

**List** (`src/runtime/list.rs`) — `count`, `at:`, `at:put:`, `add:` (append),
`push:` (prepend), `sliceFrom:`, `sort` / `sort:`, `bind:` (destructure, §14),
`==:`, `s`. Plus all `Iterate` combinators.

**Map** (`src/runtime/map.rs`) — `at:`, `at:put:`, `containsKey?:`, `count`,
`keys`, `values`, `==:`. Iterating yields **KeyValuePair** objects (`key`,
`value`, `s`, `==:`).

**Range / NumberRange** (`bblib/03-number_range.bub`) — built by `a..b`; `each:`
(forward or backward), `~:` (membership), `s`. **Half-open** (inclusive start,
exclusive end). Plus `Iterate` combinators.

**Integer / Double** (`src/runtime/{integer,double}.rs`, `bblib/00-bootstrap.bub`)
— arithmetic operators (§6), comparisons, `sqrt`, `abs`, `next`, `integer` /
`double` (identity coercions), `negated`, `s`.

**Regex** (`src/runtime/regex.rs`, `bblib/05-regex.bub`) — literal `#/…/`;
`match:` (→ a match result supporting `.bind:` over named groups `(?<name>…)`),
`split:`, `~:` (used by `~` for `regex ~ string`), `==:`.

**IO** (`bblib/06-io.bub`, `src/runtime/io.rs`) under the `[IO]` namespace:
- `[IO]Handle` — `write:`, `writeln:`; class-side `stdout` / `stderr` / `stdin`.
- `[IO]Stdout`, `[IO]Stderr` — constant handles.
- `[IO]File` — class-side `open:`; `fullpath`, `name`, `ext`, `is_file?`.
- `[IO]Folder` — class-side `open:`; iterable (`each:`), `path`, `next`, `reset`.

---

## 19. String formatting & ANSI

> **Rules**
> - **`%:` (binary `%`)** — `'fmt' % arg` substitutes into placeholders:
>   - a bare `%` consumes the next argument value;
>   - `%1`, `%2`, … index (1-based) into a **list** argument;
>   - `%a`, `%b`, … (single letters) key into a **map** argument.
> - **`mod` (prefix `%`)** — `%'…%{expr}…'` is inline interpolation: each `%{expr}` is evaluated **in the surrounding lexical scope** and stringified with `.s`.
> - Values are converted with `.s` before insertion.
> - ANSI strings are the `#ANSI'…'` literal (a user string mixing in `ActAsUserString`); `%`-formatting works on them too.

```buildingblocks
'hello %' % 'world'                  "* 'hello world'
'%1 then %2' % #('a' 'b')            "* 'a then b'        (positional, 1-based)
'%h-%w' % #{ 'h':'hi' 'w':'world' }  "* 'hi-world'        (named, 1-char keys)

a = 'foo'; b = 'bar'
%'value is %{a + b}!'                "* 'value is foobar!' (inline, lexical)
```

> **⚠ Gotcha — two different `%`.** Binary `%` (between a string and an argument)
> is `printf`-style substitution; prefix `%` (in front of a string literal) is
> `%{…}` interpolation. They are distinct operators with distinct selectors
> (`%:` vs `mod`). And recall `%` as an *infix arithmetic* operator is modulo —
> three roles for one glyph, disambiguated by position.

---

## 20. Namespaces

> **Rules**
> - `name = value` assigns a **reassignable local**. `Name <- value` defines a **constant** global — redefining it throws (`"Global […]Name is already defined in this scope"`).
> - Namespaced names: `[NS]Name` (e.g. `[IO]File`), multi-segment `[A/B]Name`, and root `[/]Name`. A bare `Name` and `[/]Name` both refer to the **root** namespace.
> - Globals are stored by full namespace + name; namespaces are a lookup/organization mechanism, not modules with their own scope.

```buildingblocks
Pi <- 3.14159           "* constant; a second `Pi <- …` throws
radius = 2              "* local; reassignable

out = [IO]Stdout        "* namespaced global
root = [/]Object        "* explicit root; same as bare `Object`
```

> **⚠ Gotcha — constants can't be reassigned, locals can't be `<-`.** Use `<-` for
> things defined once (classes, constants) and `=` for mutable locals. Trying to
> redefine a `<-` constant is a runtime throw, not a silent overwrite.

---

## 21. Stdlib map

> **Rules** — what each file provides, and whether behavior is implemented natively
> (Rust, `src/runtime/*.rs`) or in BB (`bblib/*.bub`). Native code supplies the
> primitive payloads and operations; BB code supplies the abstractions on top.

| File | Provides |
|---|---|
| `00-bootstrap.bub` | `true`/`false`/`nil` behavior, `Object`, `Mixin`, the `Error` hierarchy, `Block` loops (`whileDo:`, `whileDefinedDo:`), numeric helpers, the `ANSI` class. (Primitive payloads/dispatch are native.) |
| `01-case.bub` | `Case` and `Object#case:` pattern matching (built on the native `~` operator). |
| `02-iterate.bub` | The `Iterate` mixin and every combinator, plus `Generator` and the external `Iterator`. (List/Map storage is native.) |
| `03-number_range.bub` | `NumberRange` (`a..b`), its `each:` and `~:` membership. |
| `04-string.bub` | String conveniences over the native string methods (e.g. `split:`). |
| `05-regex.bub` | Regex conveniences over the native regex methods (e.g. `split:`). |
| `06-io.bub` | `[IO]Stdout`/`[IO]Stderr` constants and the `[IO]Folder` iterable, over native `[IO]` handles/files. |

---

Next: **[Appendices](07-appendices.md)** — cheat-sheets, the consolidated gotchas
list, and a glossary.
