# Strata — a lazy graph-ORM over ADBC

## 1. What and why

Strata maps Quoin classes to database tables and makes traversing a *graph* of
related rows idiomatic without paying a round trip per row. The design invariant,
and the reason the package exists:

> **Round trips scale with the number of association edges the program traverses,
> never with row count.**

It is the ORM transplant of the `[NumPy]Array` architecture
(`quoin_packages/numpy/README.md`): operators build a host-side lazy plan with
zero socket traffic, and a force point lowers the whole plan in one flush. Where
NumPy flushes an operator DAG to one `evalGraph:` call, Strata lowers a
relational plan to parameterized SQL and runs it through the ADBC extension
(`crates/adbc/DESIGN.md`). The compute lives in the database, so — unlike NumPy —
**Strata needs no extension process of its own**: it is a pure `[lib]` source
package. The adbc extension loads lazily at first `[Strata]Repo` construction
(via `Class.exists?:` + a `use adbc:*` eval), so the pure layers — registry,
predicate compiler, SQL lowering — load, work, and test without the extension
binary or a database driver installed.

```
var adults = ((User.where:{ |u| u.age > 21 && u.name ~ 'A%' })
    .orderBy:#name)
    .limit:50;                "* lazy: nothing has touched the DB

adults.each:{ |u|             "* force 1: one SELECT
    u.posts.each:{ |p| … }    "* force 2: ONE IN-query for all 50 users' posts
};
```

## 2. Layering

Everything is Quoin, in `quoin_packages/strata/` (`[lib]`, no `[extension]`):

- `[Strata]Model` — base class for mapped entities; class-side declaration DSL.
- `[Strata]Relation` — the lazy plan node (the `[NumPy]Expr` analog).
- `[Strata]Repo` — a connection + dialect; ambient default with per-query override.
- `[Strata]Dialect` (+ `Sqlite` / `Postgres` subclasses) — the SQL differences.
- `[Strata]State` — module registry Map (the `[Log]State` pattern; classes have
  no class-side ivars).
- `[Strata]Error` — one error class for v1; carries the SQL + driver text when a
  force fails.

## 3. Declaring a model

A class body executes leading-dot class-side sends (the `.mix:`/`.sealed!`
mechanism), so declarations are ordinary meta methods inherited from
`[Strata]Model`:

```
[Strata]Model <- Post <- {
    .table:'posts';                      "* default: lowercased class name + 's'
    .col:#id; .col:#authorId; .col:#title; .col:#published type:Boolean;
    .belongsTo:#author of:#User via:#authorId;
}

[Strata]Model <- User <- {
    .table:'users' key:#id;              "* key: defaults to #id
    .col:#id; .col:#name; .col:#age type:Integer;
    .hasMany:#posts of:#Post via:#authorId;
}
```

- Declarations write into the `[Strata]State` registry keyed by class name.
  Association targets are **symbols** (`of:#Post`), resolved lazily at first
  use, so mutually referential models declare in any order.
- Column names map camelCase→snake_case by default (`#authorId` → `author_id`);
  override with `.col:#authorId as:'author_id'`. `type:` is optional and
  checker-facing.
- **Accessors are generated, lazily.** On a model's first use (any relation
  constructor or hydration — a registry flag), Strata emits one `Runtime.eval:`
  reopen defining `name` / `name:` per column over `@row`, skipping any selector
  the class already implements (`can?:`) so hand-written typed accessors win.
  Generating at first use rather than inside `.col:` sidesteps
  reopen-during-`DefineClass` ordering and lets hand-written methods appear
  anywhere in the body. The generic `at:` (`u.at:#name`) always works, no
  generation needed. Known trade: generated accessors are invisible to
  `qn check` — write the accessor by hand where checking matters.
- Instances hold `@row` (the ADBC row Map), `@orig` (hydration snapshot, for
  dirty tracking), `@rel` (association cache), `@batch` (hydration cohort, §6).
  Hydration wraps the row Map directly — no per-column copying.

## 4. Relations and force points

`[Strata]Relation` is immutable; every combinator returns a new node, so
relations share and compose like `[NumPy]Expr` nodes. Combinators (lazy):
`where:` `orderBy:` `orderByDesc:` `limit:` `skip:` `distinct` `with:` `via:`.
Class-side sends on a model (`User.where:…`, `User.all`) mint the root node.

Force points — exactly the NumPy rule, a selector forces iff its result leaves
the lazy world: `toList` `first` `each:` `count` `exists?` `pluck:` `s`
(slice 3 adds the write forces `update:` / `delete`). `toList` memoizes into the
node (`@cache`), so re-forcing is free; `first` lowers with `LIMIT 1` when not
already materialized; aggregates always query (they are cheaper than caching
staleness semantics).

A force lowers the plan through the repo's dialect to one parameterized
`SELECT`, runs `[ADBC]Connection query:params:`, and hydrates. Errors surface at
force time (the NumPy trade), wrapped in `[Strata]Error` with the SQL attached;
the mitigations are build-time predicate errors (§5) and the SQL text in the
error.

## 5. Predicates: blocks are the query language

`where:` takes a block; Strata compiles it to SQL, not runs it. The mechanism:
`block.code` (source survives compilation — the compiler records
`SourceInfo.source_text`; worker shipping already depends on it) is parsed with
`[Lang]Parser` (`use std:lang/ast`), and the tree is lowered against the model's
registry:

- A unary send to the block's row param (`u.age`) must name a declared column —
  anything else is a **build-time** error naming the known columns.
- `> < >= <= == !=` → SQL comparisons (`== nil` → `IS NULL`); `&& || !` →
  `AND OR NOT`; `~` → `LIKE` (SQL wildcard patterns, `'A%'`); arithmetic on
  columns passes through; `col.in:expr` → `IN`, `col.defined?` →
  `IS NOT NULL`, `col.between:a and:b` → `BETWEEN`.
- **Any maximal subtree that does not mention the row param is evaluated
  host-side** — its span's source text runs through `Runtime.eval:bindings:`
  against the block's captured environment, and the value binds as a SQL
  parameter. Captured locals, globals, arbitrary sends (`cutoff.iso`) all work
  because the real interpreter evaluates them.
- The captured environment comes from **`Block#captures`** — a small VM
  reflection method added for this (a lax `scan_portable`: collect free reads,
  resolve through the capture chain, missing reads mirror the interpreter's
  nil). The data always existed; portable-block shipping snapshots the same
  thing.
- Restrictions, all loud at build time: predicates may not reference `self` or
  `@ivars` (bind to a local first — captures cannot see instance state), and a
  block with no recorded source (runtime-assembled) is refused with a pointer
  to the fallback tiers.

Fallback tiers, always available: `where:#{ 'age': 21 }` (equality Map, ANDed)
and `where:'age > ?' params:#(…)` (raw SQL fragment, dialect-translated
placeholders). Multiple `where:` calls AND together.

## 6. The graph: sibling batching

Hydration tags every instance with its cohort (`@batch` — the list hydrated by
one force) and with the repo that produced it, so association walks on a
`via:`-routed graph stay on that repo. `u.posts` answers a lazy `[Strata]Assoc`
node; its first force gathers the foreign keys of **all** batch siblings and
issues one `SELECT … WHERE author_id IN (…)`, partitioning results into each
sibling's `@rel` cache (childless owners memoize as empty; batched children
come back in key order). The loaded children form a **union batch across all
owners**, so the next level (`p.comments` inside the nested loop) again batches
across every post of every user. Implicit traversal is fully batched at every
depth — the N+1 shape is unwritable.

The Assoc node is a Relation subclass whose own plan is already the per-owner
query (`WHERE fk = ownerKey`), so every combinator derives a **plain** relation
from it: a refinement (`u.posts.where:…`) queries for that owner alone —
refined per-owner results are not shared state. `belongsTo` answers no node:
a single row is not a relation, so the accessor itself is the force point
(`p.author.name` just works) — one IN-query over the cohort's foreign keys,
each owner caching its parent, a NULL fk a memoized nil.

`with:#posts` preloads eagerly at the parent force (same IN-query mechanism),
with scoping (`with:#posts scope:{ |p| p.published }` — the scoped subset is
what `posts` then answers on those instances) and nested paths
(`with:'posts.comments'`, one query per level). `[Strata]Repo#queries` counts
SELECT round trips so the batching invariant is pinned in tests as exact
query-count deltas, not just result contents.

Same mechanism, later slices: batched association aggregates (`u.posts.count`
→ one `GROUP BY` for the cohort), lane-parallel independent preload branches
(`Async.gather:` across pooled connections — ADBC's `lanes(8)` makes that real
parallelism), and Postgres `json_agg` fusion to collapse a whole preload tree
into literally one round trip.

## 7. Repo and dialects

`[Strata]Repo.sqlite:path` / `.sqliteMemory` / `.postgres:conn` construct the
`[ADBC]Database`, connect, and pick the dialect; `.database:db dialect:d` is the
escape hatch. A repo holds one connection in v1 (pooling is a later slice) and
exposes `transaction:` (delegating to the ADBC sugar) and `run:params:` (raw).

Binding is **ambient default + override**: `[Strata]Repo.default:repo` once
(module state, per-isolate — workers set their own); any relation forces against
it unless routed with `.via:otherRepo`.

Dialects own what the two drivers genuinely disagree on: placeholder style
(`?` vs `$1`), insert-id return (`RETURNING` vs `last_insert_rowid()`),
identifier quoting. Temporal values follow ADBC v1: ISO-8601 strings.

## 8. Writes (slice 3)

Explicit, no unit-of-work: `User.create:#{ … }` (INSERT, key backfilled per
dialect), dirty-column-only `save` (`@row` vs `@orig` diff), `delete`, and
relation-level `update:` / `delete` force points. Repeated inserts reuse a
prepared `[ADBC]Statement` (ADBC has no bulk bind). A `createTable` DDL
generator from the `.col:` declarations serves the test suite and small apps —
not migrations.

## 9. Typing

Public surfaces annotate with the gradual checker in mind: `first` → `^Model?`
patterns at the model layer, materialized lists tagged via `List.of:Post`
(a real runtime guarantee), association declarations giving `u.posts` a
documented element type. `Relation` type-var annotations are checker-only
(user generics don't dispatch) and used where they help reading.

## 10. Slices

1. **Core** (this PR): `Block#captures` VM method; registry + model DSL + lazy
   accessors + hydration; `Relation` combinators/force points; predicate
   compiler with all three tiers; repo + both dialects; `[Strata]Error`;
   tests on `sqliteMemory` behind the ADBC readiness probe.
2. **Graph**: associations, sibling batching, `with:` preloads (+ scoped and
   nested), assoc refinement.
3. **Writes**: create/save/delete, dirty tracking, relation update/delete,
   transactions, `createTable`.
4. **Deferred**: batched assoc aggregates; connection pool + lane-parallel
   preloads; `json_agg` graph fusion; correlated subqueries
   (`u.posts.any?:{…}` → `EXISTS`); plan cache (block identity → compiled SQL +
   param extractor, skipping re-parse on hot query builds); identity map;
   typed DB errors when ADBC grows them; migrations/introspection (blocked on
   ADBC's deferred `get_objects`).

## 11. Status (2026-07-18)

**Slice 1 complete** on `feat/strata-orm` (two commits: `Block#captures` in the
VM, then this package + `qnlib/tests/81-strata.qn`), not yet pushed / no PR.
Full matrix verified: qnlib suites 2511/0 (pure suite runs everywhere; e2e suite
ran for real against SQLite — hydration, generated accessors, memoization,
count/pluck/exists?, `via:` routing), nextest 810/810, clippy, wasm clippy +
cdylib link, dylint, `qn doc --check` (stdlib + book), `qn fmt`.

Implementation notes that amend or sharpen the design above:

- **The `@ivar` hole is real and closed.** In the AST, `@min` parses as an
  `identifier` whose `name` field drops the `@` — only `.text` keeps it — and a
  top-level `Runtime.eval:` reads an undefined ivar as nil, so an ivar in a
  predicate would have *silently bound NULL*. `hostValue:` therefore walks every
  host subtree (`allNodes`) and refuses any `@`-text identifier or receiverless
  send with a "bind it to a local first" error, before eval.
- **ADBC refuses an empty bind batch** ("must either specify a row count or at
  least one column"): the repo skips the `params:` send entirely when the list
  is empty.
- **The `;`-glue rule bites the DSL itself**: consecutive class-body declaration
  lines (and any method-body line followed by a `.`-leading line) need explicit
  `;` or the next send swallows into the previous one — this caused three
  distinct bugs during the build, one inside the package's own `on:`. The model
  doc-comment example now warns about it.
- **Two test suites, not one**: `TestSuite#skip:` marks a *whole* suite skipped,
  so the pure tier (lowering + refusals, no database) is its own suite and the
  e2e tier gates on the 43-adbc readiness probe separately.
- The symbolic-proxy alternative was confirmed dangerous before rejection: `&&`
  on a truthy non-Boolean silently returns the right operand (see §12), so the
  AST path was the correct fork.

**Slice 2 (the graph) is also complete** — `hasMany:of:via:` /
`belongsTo:of:via:`, the `[Strata]Assoc` node (`lib/06-assoc.qn`),
sibling-batched IN loading with per-owner `@rel` memoization, `with:` preloads
(scoped + dotted nested paths), assoc refinement degrading to plain per-owner
relations, repo threading through hydration, and `Repo#queries`. The e2e graph
suite pins every invariant as an exact query-count delta (nested two-level
traversal = 2 queries; `with:'posts.comments'` = 3 including the parent;
childless/NULL-fk asks after the batch = 0).

Additional findings from slice 2:

- **A VM closure bug, worked around**: in a package (runner-compiled) unit, a
  constructor-block local that shadows its own capture by name (`.new:{ var
  model = model … }`) **with at least one more `var` in the block** pins the
  FIRST invocation's binding — every later call reuses it (`Relation.on:Post`
  minted User relations once `on:User` had run; eval/REPL-compiled units are
  immune, which is why slice 1's monoculture tests never saw it). Workaround
  everywhere in this package: never self-shadow in a `.new:` block (`on:` takes
  `|m|`, the assoc mint copies through `o`). Deserves a VM issue + fix; minimal
  repro: a two-var `.new:{ var x = x; var y = 1 }` in a `[lib]` package unit
  called from two sites.
- `belongsTo` deliberately answers the instance (or nil), not a node — the
  asymmetry with `hasMany` is the ergonomic point (`p.author.name`); its force
  happens at the accessor, still cohort-batched.
- Batched IN loads append `ORDER BY key` — deterministic child ordering for
  free, and content assertions in tests don't depend on driver row order.

## 12. Decisions record

- **Name**: Strata (layered rock; tables as strata). Damon's pick, 2026-07-18.
- **Predicates**: AST-compile via `block.code` + new `Block#captures`, over a
  symbolic column proxy — because `&&` short-circuits on truthiness
  (`Value::is_truthy`), a proxy DSL would make `condA && condB` silently emit
  only `condB`; the AST path fails loud and keeps native `&&`/`||`.
- **Repo binding**: ambient default + `.via:` — script ergonomics; per-isolate
  module state keeps workers explicit.
- **No new extension**: the DB is the compute engine; ADBC is already the wire.
