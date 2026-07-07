#!/usr/bin/env python3
"""The `numpy` extension — NumPy-backed n-dimensional arrays as the Quoin class `[NumPy]Array`.

Slice 1 (eager skeleton): creation, introspection, and the materialization exit ramps. Every
`[NumPy]Array` instance lives in this process (the SDK's object table); Quoin holds an opaque
handle and each method send is one socket round-trip. Bulk data crosses the boundary only at the
explicit exit ramps (`toArray` / `toList`) — whole-array ops keep the data resident here
(docs/FUTURE_EXT_ARCH.md §8). The lazy expression DAG (`evalGraph:`) and operators arrive in the
next slice; the Quoin-side glue lives in `init.qn`.

Dtype policy: every array is `float64`, `int64`, or `bool` (masks, born from comparisons).
Other integer/float widths are widened on entry; anything else is a clear error. `toArray`
crosses a mask as int64 0/1 (the wire has no bool dtype); `toList` yields real Booleans.

Dev-arrangement note: the SDK is imported from the in-repo `sdk/python` (the same arrangement as
`sdk/python/examples/*`); a published package would vendor or pip-install it instead.
"""

import os
import sys

sys.path.insert(
    0,
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "sdk", "python"),
)

import numpy as np

import quoin_ext

_RNG = np.random.default_rng()

# The expression-DAG op tables (`evalGraph:`). Elementwise ops broadcast NumPy-style; reducers
# collapse an array to a scalar (no axis) or reduce along one axis (an `'axis'` key on the node),
# staying in the graph as an array. `matmul` is binary but not elementwise; it shares the binary
# table shape. The Quoin-side layer (init.qn) only ever names ops from these tables, so an
# unknown op is a glue bug, not a user error.
_BINOPS = {
    "add": np.add,
    "sub": np.subtract,
    "mul": np.multiply,
    "div": np.true_divide,
    "pow": np.power,
    "mod": np.mod,
    "floordiv": np.floor_divide,
    "matmul": np.matmul,
    "eq": np.equal,
    "ne": np.not_equal,
    "lt": np.less,
    "le": np.less_equal,
    "gt": np.greater,
    "ge": np.greater_equal,
    "and": np.logical_and,
    "or": np.logical_or,
    "maximum": np.maximum,
    "minimum": np.minimum,
    "arctan2": np.arctan2,
    "hypot": np.hypot,
    "solve": np.linalg.solve,
    "outer": np.outer,
}
_UNOPS = {
    "neg": np.negative,
    "sqrt": np.sqrt,
    "cbrt": np.cbrt,
    "exp": np.exp,
    "expm1": np.expm1,
    "log": np.log,
    "log2": np.log2,
    "log10": np.log10,
    "log1p": np.log1p,
    "abs": np.abs,
    "sin": np.sin,
    "cos": np.cos,
    "tan": np.tan,
    "sinh": np.sinh,
    "cosh": np.cosh,
    "tanh": np.tanh,
    "arcsin": np.arcsin,
    "arccos": np.arccos,
    "arctan": np.arctan,
    "floor": np.floor,
    "ceil": np.ceil,
    "round": np.round,
    "sign": np.sign,
    "not": np.logical_not,
    "isnan": np.isnan,
    "isinf": np.isinf,
    "isfinite": np.isfinite,
    "inv": np.linalg.inv,
    # Casts within the dtype policy (float64 | int64 | bool). float -> int truncates toward
    # zero (NumPy astype); anything -> bool is the != 0 test.
    "tofloat": lambda x: np.asarray(x).astype(np.float64),
    "toint": lambda x: np.asarray(x).astype(np.int64),
    "tobool": lambda x: np.asarray(x).astype(np.bool_),
}
_REDUCERS = {
    "sum": np.sum,
    "mean": np.mean,
    "min": np.min,
    "max": np.max,
    "argmin": np.argmin,
    "argmax": np.argmax,
    "std": np.std,
    "var": np.var,
    "ptp": np.ptp,
    "median": np.median,
    "countnonzero": np.count_nonzero,
    "prod": np.prod,
    "any": np.any,
    "all": np.all,
    # Linalg scalars ride the reducer path too. `norm` also takes an axis (row/column norms);
    # `trace`/`det` do not — an axis form would TypeError into a catchable error, and init.qn
    # doesn't expose one.
    "trace": np.trace,
    "det": np.linalg.det,
    "norm": np.linalg.norm,
}
# Running (cumulative) forms: array-shaped results, so they stay IN the graph — with no axis
# NumPy flattens first (its own convention), with an axis they run along it.
_CUMS = {
    "cumsum": np.cumsum,
    "cumprod": np.cumprod,
}


def _coerce(a):
    """Coerce an ndarray to the dtype policy (float64 | int64 | bool — the last from
    comparisons/masks), or raise a clear error."""
    if a.dtype == np.float64 or a.dtype == np.int64 or a.dtype == np.bool_:
        return a
    if np.issubdtype(a.dtype, np.integer):
        return a.astype(np.int64)
    if np.issubdtype(a.dtype, np.floating):
        return a.astype(np.float64)
    raise ValueError(f"unsupported element type ({a.dtype}) — expected numbers")


def _shape(x):
    """A Quoin shape argument (an Int or a List of Ints) as a numpy shape tuple."""
    if isinstance(x, int):
        return (x,)
    if isinstance(x, list) and all(isinstance(d, int) for d in x):
        return tuple(x)
    raise ValueError("shape must be an Integer or a List of Integers")


class NdArray:
    """A plain Python wrapper over one `numpy.ndarray` — the SDK keeps instances in its object
    table; methods returning an `NdArray` are detected with `isinstance` and become new
    ext-side instances (cross-class returns)."""

    def __init__(self, a):
        self.a = _coerce(np.asarray(a))

    # --- introspection (small data; crosses as structured values) ---

    def shape(self):
        return list(self.a.shape)

    def dtype(self):
        return str(self.a.dtype)

    def size(self):
        return int(self.a.size)

    def ndim(self):
        return int(self.a.ndim)

    def s(self):
        dims = "x".join(str(d) for d in self.a.shape)
        body = " ".join(np.array2string(self.a, threshold=8, edgeitems=2).split())
        return f"Array({self.a.dtype} {dims}) {body}"

    # --- element access (a scalar for 1-D; a row NdArray for n-D) ---

    def at(self, i):
        v = self.a[i]
        if isinstance(v, np.ndarray):
            return NdArray(v)
        return v.item()

    # --- the batching core: evaluate a whole expression DAG in ONE round trip ---

    def eval_graph(self, tree):
        """Evaluate a serialized expression DAG (built lazily by init.qn's operator layer):
        `#{ 'nodes': #( node... ), 'root': i }`, each node one of
        `{'op':'base','v':<array>}` (a live-instance reference on the wire, already resolved to
        the NdArray by the SDK's table-aware decode — so a graph references any number of
        distinct arrays), `{'op':'const','v':x}`, or `{'op':<table op>,'a':#( child-indices )}`.
        Nodes arrive in dependency order (children first) and each is evaluated once — a shared
        subexpression (diamond) costs one evaluation, and intermediates never become handles on
        either side. Returns a new NdArray instance (array root) or a scalar (reduction root).
        The receiver is just the dispatch anchor (it also appears as a base node)."""
        if not isinstance(tree, dict) or "nodes" not in tree or "root" not in tree:
            raise ValueError("evalGraph: expects #{ 'nodes': ..., 'root': ... }")
        nodes = tree["nodes"]
        vals = [None] * len(nodes)
        for i, n in enumerate(nodes):
            op = n["op"]
            if op == "base":
                b = n.get("v")
                if not isinstance(b, NdArray):
                    raise ValueError("evalGraph: base node does not carry a [NumPy]Array")
                vals[i] = b.a
            elif op == "const":
                vals[i] = n["v"]
            elif op in _BINOPS:
                a, b = n["a"]
                vals[i] = _BINOPS[op](vals[a], vals[b])
            elif op in _UNOPS:
                vals[i] = _UNOPS[op](vals[n["a"][0]])
            elif op in _REDUCERS:
                axis = n.get("axis")
                if axis is None:
                    vals[i] = _REDUCERS[op](vals[n["a"][0]])
                else:
                    vals[i] = _REDUCERS[op](vals[n["a"][0]], axis=axis)
            elif op in _CUMS:
                vals[i] = _CUMS[op](vals[n["a"][0]], axis=n.get("axis"))
            elif op == "clip":
                x, lo, hi = n["a"]
                vals[i] = np.clip(vals[x], vals[lo], vals[hi])
            elif op == "sort":
                vals[i] = np.sort(vals[n["a"][0]], axis=n.get("axis", -1))
            elif op == "argsort":
                vals[i] = np.argsort(vals[n["a"][0]], axis=n.get("axis", -1))
            elif op == "unique":
                vals[i] = np.unique(vals[n["a"][0]])
            elif op == "searchsorted":
                a, v = n["a"]
                vals[i] = np.searchsorted(vals[a], vals[v])
            elif op == "take":
                x, idx = n["a"]
                vals[i] = np.take(vals[x], vals[idx])
            elif op == "concat":
                vals[i] = np.concatenate([vals[j] for j in n["a"]], axis=n.get("axis", 0))
            elif op == "stack":
                vals[i] = np.stack([vals[j] for j in n["a"]], axis=n.get("axis", 0))
            elif op == "tile":
                reps = n["reps"]
                vals[i] = np.tile(
                    vals[n["a"][0]], tuple(reps) if isinstance(reps, list) else reps
                )
            elif op == "repeat":
                vals[i] = np.repeat(vals[n["a"][0]], n["n"], axis=n.get("axis"))
            elif op == "flip":
                vals[i] = np.flip(vals[n["a"][0]], axis=n.get("axis"))
            elif op == "roll":
                vals[i] = np.roll(vals[n["a"][0]], n["shift"], axis=n.get("axis"))
            elif op == "squeeze":
                vals[i] = np.squeeze(vals[n["a"][0]])
            elif op == "expanddims":
                vals[i] = np.expand_dims(vals[n["a"][0]], n["axis"])
            elif op == "swapaxes":
                vals[i] = np.swapaxes(vals[n["a"][0]], n["a1"], n["a2"])
            elif op == "transpose":
                vals[i] = np.transpose(vals[n["a"][0]])
            elif op == "flatten":
                vals[i] = np.ravel(vals[n["a"][0]])
            elif op == "reshape":
                vals[i] = np.reshape(vals[n["a"][0]], tuple(n["shape"]))
            elif op == "slice":
                vals[i] = vals[n["a"][0]][n["start"] : n["stop"] : n.get("step")]
            elif op == "index":
                vals[i] = vals[n["a"][0]][n["i"]]
            elif op == "col":
                vals[i] = vals[n["a"][0]][:, n["i"]]
            elif op == "select":
                x, mask = n["a"]
                vals[i] = vals[x][np.asarray(vals[mask], dtype=bool)]
            elif op == "where":
                c, x, y = n["a"]
                vals[i] = np.where(vals[c], vals[x], vals[y])
            else:
                raise ValueError(f"evalGraph: unknown op '{op}'")
        root = vals[tree["root"]]
        if isinstance(root, np.ndarray):
            return NdArray(root)
        if isinstance(root, np.generic):
            return root.item()
        return root

    # --- structure ---

    def non_zero(self):
        """The indices of the non-zero elements, one index array per dimension (NumPy's
        `nonzero` tuple) — a List of live instances on the wire."""
        return [NdArray(ix) for ix in np.nonzero(self.a)]

    def eig(self):
        """Eigenvalues + right eigenvectors (`np.linalg.eig`) as a List of two live arrays.
        Complex results with (numerically) zero imaginary parts are realified; a genuinely
        complex spectrum is outside the dtype policy and raises a clear, catchable error."""
        w, v = np.linalg.eig(self.a)

        def realify(x):
            if np.iscomplexobj(x):
                if np.abs(x.imag).max() > 1e-9:
                    raise ValueError(
                        "eig: complex eigenvalues are not representable (float64/int64 only)"
                    )
                x = x.real
            return NdArray(x)

        return [realify(w), realify(v)]

    def svd(self):
        """Singular value decomposition (`np.linalg.svd`): a List of [U, S, Vt] live arrays."""
        u, s, vt = np.linalg.svd(self.a)
        return [NdArray(u), NdArray(s), NdArray(vt)]

    def split(self, n):
        """Split into `n` near-equal parts along axis 0 (`np.array_split`), returned as a List of
        new resident arrays — instances inside a structured value (live references on the wire)."""
        if not isinstance(n, int) or n < 1:
            raise ValueError("split: expects a positive Integer")
        return [NdArray(p) for p in np.array_split(self.a, n)]

    # --- materialization exit ramps (bulk leaves this process here, and only here) ---

    def toList(self):
        return self.a.tolist()

    def toArray(self):
        # The host bulk `Array` is a 1-D column; n-D flattens row-major (shape travels via
        # `shape`), and a bool mask crosses as int64 0/1 (the wire has no bool dtype).
        # `<f8`/`<i8` pins little-endian (the Arrow layout contract) — a no-op copy on LE hosts.
        if self.a.dtype == np.bool_:
            flat = np.ascontiguousarray(self.a).astype("<i8")
            return quoin_ext.ArrowArray(quoin_ext.ArrowArray.INT64, flat.tobytes())
        if self.a.dtype == np.float64:
            flat = np.ascontiguousarray(self.a).astype("<f8", copy=False)
            return quoin_ext.ArrowArray(quoin_ext.ArrowArray.FLOAT64, flat.tobytes())
        flat = np.ascontiguousarray(self.a).astype("<i8", copy=False)
        return quoin_ext.ArrowArray(quoin_ext.ArrowArray.INT64, flat.tobytes())


# --- class-side constructors ---


def zeros(shape):
    return NdArray(np.zeros(_shape(shape)))


def ones(shape):
    return NdArray(np.ones(_shape(shape)))


def from_list(xs):
    if not isinstance(xs, list):
        raise ValueError("fromList: expects a List of numbers (nested Lists for n-D)")
    return NdArray(np.array(xs))


def arange(n):
    return NdArray(np.arange(n))


def linspace(start, stop, count):
    return NdArray(np.linspace(start, stop, count))


def random(shape):
    return NdArray(_RNG.random(_shape(shape)))


def from_array(arr):
    """A host bulk `Array` (the data plane) as a resident ndarray — the inverse of `toArray`.
    The buffer is little-endian by the wire contract; `frombuffer` wraps it without a copy
    (read-only, which is fine: every operation here produces a new array)."""
    if not isinstance(arr, quoin_ext.ArrowArray):
        raise ValueError("fromArray: expects an Array")
    dt = "<f8" if arr.dtype == quoin_ext.ArrowArray.FLOAT64 else "<i8"
    return NdArray(np.frombuffer(arr.data, dtype=dt))


def eye(n):
    return NdArray(np.eye(n))


def full(shape, value):
    return NdArray(np.full(_shape(shape), value))


def diag(v):
    """A diagonal matrix from a 1-D array (or the diagonal of a 2-D one) — an instance
    argument to a class-side selector."""
    if not isinstance(v, NdArray):
        raise ValueError("diag: expects a [NumPy]Array")
    return NdArray(np.diag(v.a))


def meshgrid(x, y):
    """Coordinate grids for two 1-D axes — a List of two live instances (the class-side
    non-instance-return path)."""
    if not (isinstance(x, NdArray) and isinstance(y, NdArray)):
        raise ValueError("meshgrid:with: expects two [NumPy]Arrays")
    gx, gy = np.meshgrid(x.a, y.a)
    return [NdArray(gx), NdArray(gy)]


def log_space(start, stop, count):
    return NdArray(np.logspace(start, stop, count))


def geom_space(start, stop, count):
    return NdArray(np.geomspace(start, stop, count))


def random_normal(shape):
    return NdArray(_RNG.standard_normal(_shape(shape)))


def random_int(lo, hi, shape):
    """Uniform int64 in [lo, hi) — `to:` is exclusive, like Quoin ranges."""
    return NdArray(_RNG.integers(lo, hi, _shape(shape)))


def seed(n):
    """Reseed the extension's RNG so random:/randomNormal:/randomInt:to:shape: replay."""
    global _RNG
    _RNG = np.random.default_rng(n)
    return None


if __name__ == "__main__":
    ext = quoin_ext.Extension()
    ext.register(
        "Array",
        NdArray,
        constructors={
            "zeros:": zeros,
            "ones:": ones,
            "eye:": eye,
            "full:with:": full,
            "diag:": diag,
            "fromList:": from_list,
            "fromArray:": from_array,
            "arange:": arange,
            "linspace:to:count:": linspace,
            "logSpace:to:count:": log_space,
            "geomSpace:to:count:": geom_space,
            "meshgrid:with:": meshgrid,
            "random:": random,
            "randomNormal:": random_normal,
            "randomInt:to:shape:": random_int,
            "seed:": seed,
        },
        methods={
            "shape": NdArray.shape,
            "dtype": NdArray.dtype,
            "size": NdArray.size,
            "ndim": NdArray.ndim,
            "s": NdArray.s,
            "at:": NdArray.at,
            # One selector, any number of distinct arrays: the graph's base nodes carry
            # live-instance references, so there is no argument-arity ceiling.
            "evalGraph:": NdArray.eval_graph,
            "split:": NdArray.split,
            "nonZero": NdArray.non_zero,
            "eig": NdArray.eig,
            "svd": NdArray.svd,
            "toList": NdArray.toList,
            "toArray": NdArray.toArray,
        },
    )
    ext.serve(sys.argv[1])
