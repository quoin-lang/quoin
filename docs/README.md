# Quoin documentation

## Start here

The **[language reference](language/)** is the user-facing documentation: syntax, objects,
blocks, errors, concurrency, and the library reference.

- [`ENV_FLAGS.md`](ENV_FLAGS.md) — every environment variable `qn` reads, user-facing and
  internal.

## Internal design notes

Everything below is a **design and implementation record for contributors**, not user
documentation. These files were written before or during the work they describe, so many are
written in the future tense about things that now exist. Each one opens with a *Status* line
saying what actually shipped; **trust the status line over the prose.** They are not linked from
the language reference, and they are not part of the release.

Statuses were re-verified against the tree on 2026-07-09. The vocabulary:

| status | meaning |
|---|---|
| SHIPPED | everything the document describes is in the tree |
| PARTIAL | some slices landed, some did not — the status line says which |
| DESIGN | nothing built yet |
| SUPERSEDED | a later document replaced it; kept for lineage |
| REFERENCE | a reference table, not a design |

### The VM core

- [`FIBER_REDESIGN.md`](FIBER_REDESIGN.md) — SHIPPED. The stackful-fiber (corosensei) migration
  that produced today's architecture. The foundational document.
- [`FIBER_API_DESIGN.md`](FIBER_API_DESIGN.md) — SHIPPED. Guest `Fiber`, the `^>` operator, and
  the generator/iterator bridge.
- [`LINTER_DESIGN.md`](LINTER_DESIGN.md) — SHIPPED. The Dylint passes that mechanically enforce
  the GC-across-yield rules the fiber redesign wrote down.
- [`INTROSPECTION.md`](INTROSPECTION.md) — SHIPPED. The read-only `src/introspect.rs` API.

### Concurrency and I/O

- [`ASYNC_ARCH.md`](ASYNC_ARCH.md) — SHIPPED. Stages 0–8: sockets, TLS, streams, listeners,
  timeouts, and CSP channels over the cooperative scheduler.
- [`CONCURRENCY_ARCH.md`](CONCURRENCY_ARCH.md) — SHIPPED (C1 + C2). Compute-offload pool and
  worker isolates. A shared heap was rejected on gc_arena grounds.
- [`WEB_ARCH.md`](WEB_ARCH.md) — SHIPPED. The `[Web]` framework over `[HTTP]Server`.
- [`USE_ARCH.md`](USE_ARCH.md) — SHIPPED. `use`, `self:`, globs, and package resolution.

### Types

- [`TYPE_SYSTEM_ARCH.md`](TYPE_SYSTEM_ARCH.md) — MOSTLY SHIPPED. The gradual checker.
- [`GENERICS_ARCH.md`](GENERICS_ARCH.md) — SHIPPED (G0–G4). Checked generic collections.
- [`TYPED_DEVIRT_ARCH.md`](TYPED_DEVIRT_ARCH.md) — PARTIAL. The devirtualization tier shipped;
  unboxed structs are deferred.

### Performance

- [`PERF_ROADMAP.md`](PERF_ROADMAP.md) — the live ranked portfolio. Start here.
- [`FUTURE_ARCH.md`](FUTURE_ARCH.md) — SUPERSEDED by the above. Kept for lineage.
- [`AOT_ARCH.md`](AOT_ARCH.md) — SHIPPED. Native compilation of the typed subset, on by default.
- [`SPECULATIVE_AOT_ARCH.md`](SPECULATIVE_AOT_ARCH.md) — SHIPPED (S0–S3, S5). Type-feedback
  compilation for untyped code.
- [`BLOCK_AOT_ARCH.md`](BLOCK_AOT_ARCH.md) — SHIPPED (B0–B3). Compiling the combinator tier.
- [`ALLOC_ARCH.md`](ALLOC_ARCH.md) — SHIPPED (A1, A2a–d). Allocation churn.
- [`MATERIALIZATION_ARCH.md`](MATERIALIZATION_ARCH.md) — SHIPPED (M1–M3). Fusion and thin
  closures.
- [`OUTCALL_ARCH.md`](OUTCALL_ARCH.md) — SHIPPED. The outcall seam.
- [`DIRECT_CALLS_ARCH.md`](DIRECT_CALLS_ARCH.md) — PARTIAL. All slices landed; the direct-edge
  tier ships default-off because the gate measured net-negative.
- [`WINDOW_ARENA_ARCH.md`](WINDOW_ARENA_ARCH.md) — PARTIAL. A1–A3 landed and the arc closed
  there: the remaining crossing removal did not pay.

### Extensions

- [`FUTURE_EXT_ARCH.md`](FUTURE_EXT_ARCH.md) — PARTIAL. Out-of-process polyglot extensions.
  Remaining: publishing the SDK crates. Deferred: fd-passing, WASM.
- [`EXT_PACKAGING.md`](EXT_PACKAGING.md) — SHIPPED. An extension as a `use`-able folder.

### Tooling

- [`DEBUGGER_ARCH.md`](DEBUGGER_ARCH.md) — SHIPPED. `qn debug`, and DAP over stdio.
- [`REPL_DESIGN.md`](REPL_DESIGN.md) — SHIPPED (P0–P2). `qn repl`.
- [`DOCS_ARCH.md`](DOCS_ARCH.md) — DESIGN. Reference docs from comment blocks: `qn doc`
  (HTML + JSON) over the introspection layer, one pipeline for Quoin, native, and extension
  classes; `qn highlight --html` shares its code styles.

### Stdlib

These three are implementation records for shipped API; the language reference is the place to
learn the API itself.

- [`STDLIB_NUMBERS.md`](STDLIB_NUMBERS.md) — SHIPPED. Math, Statistics, BigDecimal, BigInteger.
- [`STDLIB_TIME.md`](STDLIB_TIME.md) — SHIPPED. Duration, Instant, Timestamp, DateTime, TimeZone.
- [`STDLIB_DATA_FORMATS.md`](STDLIB_DATA_FORMATS.md) — SHIPPED. JSON, YAML, TOML, CSV,
  MessagePack, base64, hex.

### Process

- [`RELEASE_PREP.md`](RELEASE_PREP.md) — the v0.1.0 checklist and its as-built records.
