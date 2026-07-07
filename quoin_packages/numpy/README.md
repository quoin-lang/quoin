# `numpy` — NumPy-backed arrays for Quoin

An out-of-process extension package (`docs/FUTURE_EXT_ARCH.md`, `docs/EXT_PACKAGING.md`) exposing
real NumPy as the Quoin class **`[NumPy]Array`**. Arrays live in a Python subprocess; Quoin holds
opaque handles, and bulk data crosses the socket only at explicit materialization points. The
`[NumPy]` namespace marks this as the Python-backed implementation — `[Num]` is reserved for a
future native (Rust) backend behind the same Quoin-side surface.

**Requires:** a `python3` on PATH that can `import numpy, msgpack`.

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
- **No arity ceiling:** a graph's base nodes carry live-instance references (wire ext type 3),
  so one send spans any number of distinct arrays.

## API

**Creation** (class-side): `zeros:` `ones:` `eye:` `full:with:` `diag:` (from a 1-D array)
`fromList:` `fromArray:` (a host bulk `Array`, whole-buffer — the inverse of `toArray`)
`arange:` `linspace:to:count:` `logSpace:to:count:` `geomSpace:to:count:` `meshgrid:with:`
(→ a List of two grids) — shapes are an Integer or a List (`#( 2 3 )`). Random: `random:`
`randomNormal:` `randomInt:to:shape:` (`to:` exclusive, like Quoin ranges), reproducible via
`seed:`.

**Elementwise** (lazy): `+ - * /` `pow:` `mod:` `floorDiv:` `neg` `sqrt` `cbrt` `exp` `expm1`
`log` `log2` `log10` `log1p` `abs` `sin` `cos` `tan` `sinh` `cosh` `tanh` `arcSin` `arcCos`
`arcTan` `arcTan2:` `floor` `ceil` `round` `sign` `maximum:` `minimum:` `hypot:` `clip:to:`
(bounds are ordinary operands — scalars or arrays) — NumPy broadcasting and promotion rules.
Lazy dtype casts within the policy: `toFloat` `toInt` `toBool`.

**Comparisons → masks** (lazy, ELEMENTWISE — NumPy semantics): `== != < <= > >=` build bool
masks; `and:` `or:` `not` combine them; `isNan` `isInf` `isFinite` inspect floats; `select:`
(boolean indexing); `mask.where:x else:y` (functional conditional); `mask.sum` counts;
`any` / `all` reduce to a Boolean.

**Reductions:** `sum mean min max argMin argMax std variance ptp median prod countNonZero`
(plus linalg's `trace det norm`) — whole-array forms force to a scalar; axis forms (`sum: 0`,
`mean: 1`, …) return arrays and stay lazy. The running forms `cumSum`/`cumProd` (+axis) are
array-shaped and always stay in the graph.

**Shape & manipulation** (lazy): `transpose` `flatten` `reshape:` `from:to:` / `from:to:by:`
(first axis) `row:` `col:` `squeeze` `expandDims:` `swapAxes:with:` `flip`(+axis) `roll:`(+axis)
`tile:` `repeat:`(+axis); `concat:` / `stack:` (+`axis:`) take a List of further arrays — any
count, in one graph; `split:` (eager) divides along axis 0 into a List of live arrays, each of
which joins new lazy expressions.

**Sorting & searching** (lazy): `sort` / `argSort` (NumPy's last-axis default; keyword forms
pick an axis), `unique`, `searchSorted:`, `takeAt:` (fancy indexing by an index array);
`nonZero` (eager) returns one index array per dimension as a List.

**Linalg:** `matMul:` (1-D dot yields a scalar at force), lazy `solve:` `inv` `outer:`, forcing
`trace` `det` `norm` (+ lazy `norm:` for row/column norms), and the eager multi-returns `eig`
(→ `#( values vectors )`; complex spectra raise) and `svd` (→ `#( u s vt )`).

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

Measured on this machine (release build, `bench.qn`, on the MessagePack-only wire —
`profiling/msgpack-wire/notes.md`): a minimal class-dispatch call is **~15µs** (the raw UDS
round trip is 2.3µs; the rest is the ~12µs syscall/scheduling floor plus Python dispatch), and
each DAG node adds only ~3µs — the whole frame is one C-`msgpack` codec pass. The batched MSE
iteration forces in ~45µs vs ~72µs op-by-op at n=1k; deeper chains amortize further. The
architecture (one round trip per force, no intermediate handles) is the durable part; the
remaining per-call floor is architectural (syscalls), not codec.

**Anti-patterns:**
- Per-element access in a loop (`(1..n).each:{ |i| a.at:i }`) pays the full call cost per
  element — materialize once with `toList`/`toArray` instead.
- Forcing mid-chain (`.eval` between every op) reintroduces per-op round trips — force once at
  the end. `.eval` is for reusing a subexpression across many later graphs.

The extension evaluates graphs with plain NumPy today; swapping in a fusing evaluator (numexpr)
is invisible to Quoin — `eval_graph` in `main.py` is the seam.

## Design notes

The extension (`main.py`, ~300 lines) is deliberately dumb: creation, `evalGraph:`,
materialization. The brains — operators, DAG building, dedup, memoization — are pure Quoin in
`init.qn`, and are meant to be reused verbatim by the future native `[Num]` backend. The two
wire gaps this package originally tracked are closed: `fromArray:` rides the `Array` argument
kind, and live-instance references inside values (ext type 3) retired the old 8-base selector
ladder and let `split:` return a List of arrays.
