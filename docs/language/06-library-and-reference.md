# Part VI — Library & reference

Brief reference for the core types, string formatting, namespaces, and a map of
the standard library. For method-level detail, the stdlib `.qn` files and the
Rust `src/runtime/*.rs` modules are the source of truth — this part points you at
them rather than duplicating them.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · **Library & reference** · [Appendices](07-appendices.md)

---

## 18. Collections & core types

> **Rules**
> - These are brief, indicative lists — see the cited files for the full, current set and exact semantics.
> - Anything in the `Iterate` mixin (Part V) is also available on every iterable type below.

**String** (`src/runtime/string.rs`, `qnlib/core/04-string.qn`) — `length`, `s`,
`contains?:`, `starts?:`, `ends?:`, `index:`, `insert:at:`, `lower`, `upper`,
`replace:with:`, `split:` (String or Regex), `to_integer`, `==:`, `<`, `>`,
`%:` (formatting, §19), `mod` (interpolation, §19).

**List** (`src/runtime/list.rs`) — `count`, `at:`, `at:put:`, `add:` (append),
`push:` (prepend), `sliceFrom:`, `sort` / `sort:`, `bind:` (destructure, §14),
`==:`, `s`. Plus all `Iterate` combinators.

**Map** (`src/runtime/map.rs`) — `at:`, `at:put:`, `containsKey?:`, `count`,
`keys`, `values`, `==:`. Iterating yields **KeyValuePair** objects (`key`,
`value`, `s`, `==:`).

**Symbol** (`src/runtime/symbol.rs`) — literal `#name` / `#multi:part:` / `#'…'`;
**interned** (compared by identity), a distinct type from String. Methods: `s`
(→ the name, no `#`), `asString`, `asSymbol`, `==:`. `Block#name` and
`Method#selector`/`name` return symbols.

**Set** (`src/runtime/set.rs`, algebra in `qnlib/core/02-iterate.qn`) — literal
`#< … >`, unique by `hash`+`==:` (override both for value-equality), insertion-ordered; `count`, `add:`, `remove:`,
`contains?:`, `each:`, `s`, `==:` (order-independent), plus `union:`,
`intersection:`, `difference:`, `subset?:`, `superset?:` and all `Iterate`
combinators. Membership is O(n) — a simple reference set, not hashed.

**Range / NumberRange** (`qnlib/core/03-number_range.qn`) — built by `a..b`; `each:`
(forward or backward), `~:` (membership), `s`. **Half-open** (inclusive start,
exclusive end). Plus `Iterate` combinators.

**Integer / Double** (`src/runtime/{integer,double}.rs`, `qnlib/core/00-bootstrap.qn`)
— arithmetic operators (§6), comparisons, `sqrt`, `abs`, `next`, `integer` /
`double` (identity coercions), unary `-`, `s`.

**Regex** (`src/runtime/regex.rs`) — literal `#/…/`;
`split:` (split a string on the pattern), `~:` (used by `~` to test `regex ~ string`),
`==:`.

**IO** (`qnlib/core/06-io.qn`, `src/runtime/io.rs`) under the `[IO]` namespace:
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
> - **`mod` (prefix `%`)** — `%'…%{expr}…'` is inline interpolation: each `%{expr}` is evaluated over the surrounding **locals and parameters** and stringified with `.s`. Note: `self`, a leading-dot send (`%{.name}`), and instance fields (`%{@name}`) are **not** in scope inside `%{…}` — they resolve as `nil`/`MessageNotUnderstood`. Bind what you need to a local first.
> - Values are converted with `.s` before insertion.
> - ANSI strings are the `#ANSI'…'` literal (a user string mixing in `ActAsUserString`); `%`-formatting works on them too.

```quoin
'hello %' % 'world'                  "* -> 'hello world'
'%1 then %2' % #('a' 'b')            "* -> 'a then b'
'%h-%w' % #{ 'h':'hi' 'w':'world' }  "* -> 'hi-world'
var a = 'foo'; var b = 'bar';        "* the ; matters: the next line starts with an operator
%'value is %{a + b}!'                "* -> 'value is foobar!'
```

> **⚠ Gotcha — two different `%`.** Binary `%` (between a string and an argument)
> is `printf`-style substitution; prefix `%` (in front of a string literal) is
> `%{…}` interpolation. They are distinct operators with distinct selectors
> (`%:` vs `mod`). And recall `%` as an *infix arithmetic* operator is modulo —
> three roles for one glyph, disambiguated by position.

---

## 20. Namespaces

> **Rules**
> - `var name = value` declares a **reassignable local** (§4). `Name <- value` defines a **constant** global — redefining it throws (`"Global […]Name is already defined in this scope"`).
> - Namespaced names: `[NS]Name` (e.g. `[IO]File`), multi-segment `[A/B]Name`, and root `[/]Name`. A bare `Name` and `[/]Name` both refer to the **root** namespace.
> - Globals are stored by full namespace + name; namespaces are a lookup/organization mechanism, not modules with their own scope.

```quoin
Pi <- 3.14159           "* constant; a second `Pi <- …` throws
var radius = 2          "* local; reassignable

var out = [IO]Stdout    "* namespaced global
var root = [/]Object    "* explicit root; same as bare `Object`
```

> **⚠ Gotcha — constants can't be reassigned, locals can't be `<-`.** Use `<-` for
> things defined once (classes, constants) and `var` for mutable locals. Trying to
> redefine a `<-` constant is a runtime throw, not a silent overwrite.

---

## 21. File loading & packages (`use`)

> **Rules**
> - `use (pkg:)? path;` loads a `.qn` file **once** — a repeat `use` (or a cyclic one) is a no-op. It's a statement that runs when reached and evaluates to `nil`. `use` is a **soft keyword**: special only here, an ordinary identifier everywhere else.
> - **Path is the load address** (with `.qn` implied, `/`-separated); the **`[Ns]` namespace is the logical name** a file's definitions register under. The two are independent — a file may define classes, extend existing ones, add mixins, anything.
> - **Package qualifier** (`pkg:`): bare or **`std:`** = the standard library; **`self:`** = the current project; any other name is a (reserved) package, not yet resolvable.
> - **`dir/*`** globs a directory, loading every `.qn` in it in **UTF-8-sorted** order.
> - Loading is filesystem-**agnostic**: resolution goes through a host-supplied resolver (disk on the CLI; host-provided units on WASM / embedded). There is no way to load an arbitrary OS path.

These forms are illustrative — `self:` paths resolve against *your* project
(`self:helpers` names a `helpers.qn` this document doesn't ship), so the block
isn't runnable as pasted:

```quoin norun
use core/*;             "* every .qn in the stdlib's core/ dir, in sorted order
use self:helpers;       "* the current project's helpers.qn
use std:io/file;        "* explicit stdlib; `std:` and bare are the same package

MyFile <- [IO]File;     "* aliasing is just an ordinary definition — not a `use` concern
```

> **⚠ Gotcha — `use` loads, the namespace names.** `use` does not pull symbols into a
> local scope (there isn't one). It runs a file, whose `<-`/`<--` definitions register
> as ordinary namespaced globals — so you reference what a file defined by its
> `[Ns]Name`, exactly as if it had always been loaded. A second `use` of the same unit
> does nothing (definitions aren't re-run), so it can't trigger a "redefine" error.

---

## 22. Stdlib map

> **Rules** — what each file provides, and whether behavior is native (Rust,
> `src/runtime/*.rs`) or Quoin (`qnlib/`). The **core library lives in `qnlib/core/`**
> and loads as the prelude (`qnlib/prelude.qn` → `use core/*`); the test framework and
> entry points sit at the `qnlib/` root. Native code supplies primitive payloads and
> operations; Quoin code supplies the abstractions on top.

| File | Provides |
|---|---|
| `prelude.qn` | The prelude entry — `use core/*` loads the core library below (sorted == numeric). |
| `core/00-bootstrap.qn` | `true`/`false`/`nil` behavior, `Object`, `Mixin`, the `Error` hierarchy, `Block` loops (`whileDo:`, `whileDefinedDo:`), numeric helpers, the `ANSI` class. (Primitive payloads/dispatch are native.) |
| `core/01-case.qn` | `Case` and `Object#case:` pattern matching (built on the native `~` operator). |
| `core/02-iterate.qn` | The `Iterate` mixin and every combinator, plus `Generator`, the external `Iterator`, and `Set` algebra (`union:`/`intersection:`/…). (List/Map/Set storage is native.) |
| `core/03-number_range.qn` | `NumberRange` (`a..b`), its `each:` and `~:` membership. |
| `core/04-string.qn` | String conveniences over the native string methods (e.g. `split:`). |
| `core/06-io.qn` | `[IO]Stdout`/`[IO]Stderr` constants and the `[IO]Folder` iterable, over native `[IO]` handles/files. |
| `test.qn` | The test framework — `TestSuite`/`TestRunner`/reporters/assertions; suites self-register into `[Test]Suites`, run by `main.qn` via `use std:tests/*`. |

---

Next: **[Appendices](07-appendices.md)** — cheat-sheets, the consolidated gotchas
list, and a glossary.
