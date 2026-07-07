# `numpy` — NumPy-backed arrays for Quoin

An out-of-process extension package (`docs/FUTURE_EXT_ARCH.md`, `docs/EXT_PACKAGING.md`) exposing
real NumPy as the Quoin class **`[NumPy]Array`**. Arrays live in a Python subprocess; Quoin holds
opaque handles, and bulk data crosses the socket only at explicit materialization points. The
`[NumPy]` namespace marks this as the Python-backed implementation — `[Num]` is reserved for a
future native (Rust) backend behind the same Quoin-side surface.

**Requires:** a `python3` on PATH that can `import numpy, flatbuffers`.

```quoin
use numpy:*;

var a = [NumPy]Array.fromList:#( 1.0 2.0 3.0 );
var b = [NumPy]Array.random:#( 3 );

((a - b) * (a - b)).mean;          "one socket round trip"
(a.select:(a > 2.0)).toList;       "#( 3.0 )"
```

## The lazy model — batching via operators

Operators never call the extension. They build a host-side **expression DAG** (`[NumPy]Expr`
nodes, `init.qn`); a **force point** serializes the whole DAG and ships it in **one**
`evalGraph:` round trip. The extension evaluates the node list with NumPy and returns a new
resident array — or a scalar, for a reduction root.

- **Force points:** `eval`, whole-array reductions (`sum`/`mean`/`min`/`max`/`argMin`/`argMax`/
  `std`/`prod`/`any`/`all`), and materializations (`toList`/`toArray`/`at:`/`shape`/`dtype`/
  `size`/`ndim`/`s`).
- **Stays lazy:** elementwise ops, comparisons, `matMul:`, axis reductions (`sum: 0` returns an
  array), shape ops, slicing, masks.
- **Diamonds evaluate once:** a shared subexpression is serialized and computed a single time.
- **`eval` memoizes:** a forced expr holds its materialized array and re-enters later graphs as a
  cheap leaf.
- One graph carries at most **8 distinct arrays** (the base-argument selector ladder). Wider
  expressions get a catchable error — `.eval` a subexpression to fold it into one base.

## API

**Creation** (class-side): `zeros:` `ones:` `fromList:` `arange:` `linspace:to:count:` `random:`
— shapes are an Integer or a List (`#( 2 3 )`).

**Elementwise** (lazy): `+ - * /` `pow:` `mod:` `neg` `sqrt` `exp` `log` `abs` `sin` `cos` `tan`
`floor` `ceil` `round` `sign` — NumPy broadcasting and promotion rules.

**Comparisons → masks** (lazy, ELEMENTWISE — NumPy semantics): `== != < <= > >=` build bool
masks; `and:` `or:` `not` combine them; `select:` (boolean indexing); `mask.where:x else:y`
(functional conditional); `mask.sum` counts; `any` / `all` reduce to a Boolean.

**Reductions:** whole-array forms force to a scalar; axis forms (`sum: 0`, `mean: 1`, …) return
arrays and stay lazy.

**Shape & slicing** (lazy): `transpose` `flatten` `reshape:` `from:to:` (first axis) `row:`
`col:`; `matMul:` for matrix/vector products (1-D dot yields a scalar at force).

**Materialization:** `toList` (nested Lists; masks become Booleans), `toArray` (the host bulk
`Array`, 1-D row-major; masks cross as int64 0/1), `at:` (scalar for 1-D, a row instance for
n-D), `s` (shape + dtype + preview).

## Semantics notes

- **dtypes:** `float64`, `int64`, `bool` (masks only). Narrower numerics widen on entry.
- **Immutable values:** every op returns a new array/expr; there is no in-place mutation.
- **`==` on arrays is elementwise** — identity comparison of two arrays is gone (the NumPy
  trade). Comparing a *forced* scalar (`(v.matMul:v).eval == 2.0`) works normally.
- **The array goes on the operator's LEFT.** `a * 2.0` works; `2.0 * a` cannot dispatch —
  `Integer`/`Double` are sealed (typed-devirt soundness), so no arm can be added to them.
- **Errors surface at force time** as catchable Quoin errors carrying NumPy's message (e.g.
  broadcast shape mismatches); the extension survives them. Arrays do NOT survive an extension
  crash — they are compute values, not durable state.

## Performance model

Measured on this machine (release build, `bench.qn`): a minimal extension call is **~45µs**
(pure-Python FlatBuffers SDK; the raw UDS round trip is 2.3µs), and each DAG node adds roughly
**50–75µs** of serialization. Batching therefore wins ~1.4–1.6× on a 3-op chain today and more on
deeper chains; transport tuning (binary node encoding, C-accelerated flatbuffers, or the native
`[Num]` backend) multiplies the win — the architecture (one round trip per force, no
intermediate handles) is the durable part.

**Anti-patterns:**
- Per-element access in a loop (`(1..n).each:{ |i| a.at:i }`) pays the full call cost per
  element — materialize once with `toList`/`toArray` instead.
- Forcing mid-chain (`.eval` between every op) reintroduces per-op round trips — force once at
  the end. `.eval` is for reusing a subexpression across many later graphs, or splitting a
  >8-array graph.

The extension evaluates graphs with plain NumPy today; swapping in a fusing evaluator (numexpr)
is invisible to Quoin — `eval_graph` in `main.py` is the seam.

## Design notes

The extension (`main.py`, ~300 lines) is deliberately dumb: creation, `evalGraph:`,
materialization. The brains — operators, DAG building, dedup, memoization — are pure Quoin in
`init.qn`, and are meant to be reused verbatim by the future native `[Num]` backend. Known
protocol gaps tracked for later: `fromArray:` (host bulk `Array` as a *method argument*) needs an
`ArgKind.Array` wire extension; a `DvResource` DataValue kind would retire the 8-base selector
ladder and allow returning lists of arrays.
