# BuildingBlocks Language Reference

A semantics reference for the BuildingBlocks (BB) language, grounded in how the
VM actually behaves. Near-term purpose: a record of how to write BB correctly so
that a reader (human or a fresh tooling session) doesn't have to reverse-engineer
the interpreter. Longer-term, this is intended to grow into end-user documentation.

**Format.** Pedagogical order — read top to bottom to learn the language — but
every section opens with a terse **Rules** box so it also works for lookup. Method
reference is kept brief and points at the stdlib (`bblib/*.bub`) as the source of
truth rather than duplicating it.

> Status: full draft (Parts I–VI + appendices) pending review. Claims are verified
> against the parser grammar, `src/vm.rs`, `bblib/`, and the test suite — the
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
12. Inheritance & mixins — subclassing, method-resolution order, `.mix:`/`.can:`, `.sealed!`
13. Multimethod dispatch — typed params, guard blocks, resolution

### Part IV — Patterns & errors · [`04-patterns-and-errors.md`](04-patterns-and-errors.md)
14. Pattern matching & `case` — `when:do:`, the `~` protocol, `.bind:`
15. Errors & stack traces — `throw`/`catch:`, the `Error` hierarchy

### Part V — Concurrency & iteration · [`05-concurrency-and-iteration.md`](05-concurrency-and-iteration.md)
16. Fibers & generators — `Fiber.new:`, `^>`, `Generator.from:`, external `Iterator`
17. The iteration protocol — the `Iterate` mixin, `each:` as the one primitive, custom iterables

### Part VI — Library & reference · [`06-library-and-reference.md`](06-library-and-reference.md)
18. Collections & core types — brief tables, pointers to stdlib
19. String formatting & ANSI — `%` interpolation, `%'…%{expr}'`
20. Namespaces — `[IO]`, `[/]`, `[Y]`
21. Stdlib map — what each `bblib/*.bub` provides; native vs BB

### Appendices · [`07-appendices.md`](07-appendices.md)
- A. Sigil & operator cheat-sheet
- B. Selector / desugaring quick-reference
- C. **Gotchas for writing & generating BB** — all corner cases consolidated
- D. Glossary
