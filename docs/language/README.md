# Quoin Language Reference

A semantics reference for the Quoin language, grounded in how the
VM actually behaves. Near-term purpose: a record of how to write Quoin correctly so
that a reader (human or a fresh tooling session) doesn't have to reverse-engineer
the interpreter. Longer-term, this is intended to grow into end-user documentation.

**Format.** Pedagogical order — read top to bottom to learn the language — but
every section opens with a terse **Rules** box so it also works for lookup.
Parts I–V teach the language core; Parts VI–VIII cover networking & the web,
the gradual type system, and the tooling; Part IX tours the standard library,
Part X covers packages and installation, and the appendices consolidate the
cheat-sheets and gotchas. Method-level
reference is kept brief: the API reference is generated — run `qn doc`, or ask
the REPL with `$doc Name` / `$doc Name.selector` — rather than duplicated here.

> Status: full draft (Parts I–X + appendices) pending review. Claims are verified
> against the parser grammar, `src/vm.rs`, `qnlib/`, and the test suite — the
> surprising ones were confirmed by running the VM directly.

## Contents

### Part I — Foundations · [`01-foundations.md`](01-foundations.md)
1. Mental model — the few ideas everything reduces to
2. Lexical structure — comments, separators, identifiers
3. Literals & data types — numbers, strings, symbols, lists, maps, ranges, regex, ANSI, blocks
4. Variables, scope & destructuring — `=`, `_`, splat, namespaced globals
5. Messages & call syntax — unary / keyword / multi-part selectors, `self` and `.`
6. Operators & precedence — desugaring to selectors

### Part II — Blocks & control flow · [`02-blocks-and-control.md`](02-blocks-and-control.md)
7. Blocks & closures — params, typed params, arity, `.value`
8. Control flow is a library, not syntax — `if:`/`else:`/`whileDo:`, truthiness
9. Returns & non-local return — `^` vs `^^`

### Part III — Objects · [`03-objects.md`](03-objects.md)
10. Classes, methods & extension — `<-`, `<--`, `->`, `-->`, `@var`, `.meta`
11. Construction & initialization — `new`/`new:{}`, the `init`/`init:` chain, **block scoping corner cases**
12. Inheritance & mixins — subclassing, method-resolution order, `.mix:`/`can?:`, `.sealed!`/`.abstract!`
13. Multimethod dispatch — typed params, guard blocks, resolution

### Part IV — Patterns & errors · [`04-patterns-and-errors.md`](04-patterns-and-errors.md)
14. Pattern matching & `case` — `when:do:`, the `~` protocol, `.bind:`
15. Errors & stack traces — `throw`/`catch:`, the `Error` hierarchy

### Part V — Concurrency & iteration · [`05-concurrency-and-iteration.md`](05-concurrency-and-iteration.md)
16. Fibers & generators — `Fiber.new:`, `^>`, `Generator.from:`, external `Iterator`
17. The iteration protocol — the `Iterate` mixin, `each:` as the one primitive, custom iterables
18. Tasks & the cooperative scheduler — `Task.spawn:`, `join`/`cancel`, round-robin at yield boundaries, parking parks the task
19. Async — `sleep:`, `gather:`, `timeout:do:` (+ `onCancel:`), `joinAll:`
20. Channels — rendezvous & buffered CSP, `close`, deadlock detection
21. Workers, Parallel & Plan — isolates, `parallelCollect:`, task graphs, `Worker.host:`

### Part VI — Networking & the web · [`06-networking-and-web.md`](06-networking-and-web.md)
22. The I/O model — park-don't-block; what `use std:net/*` / `use std:web/*` load (tasks, gather & timeouts: Part V)
23. Sockets & streams — `TcpSocket` / `TcpListener` / `TlsSocket`, `ByteStream` / `StringStream`, write-through vs. buffered
24. `TcpServer` — a minimal concurrent TCP server
25. The `[HTTP]` client — verbs, the request builder, bodies & JSON, streaming, redirects
26. Serving HTTP — the `[HTTP]Server` transport and the `[Web]App` framework
27. End to end — a JSON service unit-tested in-process via `handle:`, then served

### Part VII — The gradual type system · [`07-types.md`](07-types.md)
28. Types are optional — dynamic by default; checker, dispatch & optimizer as the three consumers
29. Annotation syntax — typed params, `^Ret` headers, `var x: T`, nullable `T?`, generics, `Block(args ^Ret)`
30. The checker: `qn check` — reading diagnostics; mismatches, compile-time MNU, override covariance
31. Nullable types & nil narrowing — `T?`, `defined?` guards, flow-sensitive narrowing
32. Types at dispatch time — typed multimethods, guards, tag-aware dispatch
33. Checked generic collections — element tags, `of:`/`ensure:`/`elementType`, combinators
34. Sealing — `sealed!`/`abstract!`, the sealed built-ins, why devirtualization needs them
35. Errors at runtime, warnings at compile time — `NameError`, `Class.exists?:`, MNU candidates

### Part VIII — Tooling · [`08-tooling.md`](08-tooling.md)
36. Running programs — `qn`, `-e`, `Runtime.arguments`, the exit-code contract, environment
37. The REPL — `qn repl`: persistent sessions, editing/completion, the `$`-commands, `~/.quoinrc`
38. Tests — `qn test`: suites & assertions, coverage reports, exit-code gating
39. Static checking — `qn check`: diagnostics without running
40. Formatting — `qn fmt`: the opinionated, self-verifying formatter
41. The API reference — `qn doc`: generated docs and the doc-example harness (which checks this book)
42. The debugger — `qn debug`: breakpoints, stepping, exception breakpoints, `--dap`
43. Syntax highlighting — `qn highlight`: ANSI and HTML rendering

### Part IX — The standard library · [`09-library-and-reference.md`](09-library-and-reference.md)
44. The library by area — collections & `Iterate`, strings, numbers, time, data formats, bytes, I/O & streams, OS, ids; the generated API reference (`qn doc` / REPL `$doc`)
45. Value rendering & string formatting — `s` vs `pp`, `%` formatting, `%'…%{expr}'`, ANSI
46. Namespaces — `[IO]`, `[/]`, `[Y]`
47. File loading & packages — `use (pkg:)? path`, the resolver seam, directory globs
48. Stdlib map — one line per unit: the `core/*` prelude, `net/`, `web/`; native vs Quoin

### Part X — Quoin packages · [`10-packages.md`](10-packages.md)
49. What a package is — `quoin.toml`, extension vs program packages, the search roots, namespacing
50. Installing packages — `qn pkg install`/`list`, `$QUOIN_HOME` (default `~/.quoin`), `bin/` on the PATH
51. Writing packages — the SDKs, `init.qn` glue, `[bin]` programs, what's deliberately deferred

### Appendices · [`11-appendices.md`](11-appendices.md)
- A. Sigil & operator cheat-sheet
- B. Selector / desugaring quick-reference
- C. **Gotchas for writing & generating Quoin** — all corner cases consolidated
- D. Glossary
