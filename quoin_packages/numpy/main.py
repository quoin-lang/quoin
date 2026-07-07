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

try:
    import numexpr
except ImportError:  # optional accelerator — the plain-NumPy path is complete on its own
    numexpr = None

# Fused evaluation is on when numexpr imports, unless QN_NUMPY_NO_NUMEXPR disables it. Graphs
# whose largest base array is below QN_NUMPY_NUMEXPR_MIN elements stay on the plain path:
# numexpr's per-evaluate overhead beats the fusion win on small arrays (the default is the
# measured crossover — profiling/numexpr/notes.md).
_NUMEXPR_ENABLED = numexpr is not None and not os.environ.get("QN_NUMPY_NO_NUMEXPR")
_NUMEXPR_MIN = int(os.environ.get("QN_NUMPY_NUMEXPR_MIN", "32768"))

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

# --- numexpr fusion: the seam `eval_graph` was designed for --------------------------------
# Ops with a numexpr spelling whose semantics match the NumPy tables above EXACTLY (probed:
# int/int division -> float64, `%` follows np.mod's sign, int**int stays int64, floor/ceil
# present). Anything not here — log2/cbrt/sign/round/the is*-masks/maximum/minimum/hypot/
# floordiv/clip, axis reductions, shape ops, sorting — evaluates on the plain NumPy path
# mid-graph. Fusion only ever changes how many passes run, never what is computed.
_NX_BINOPS = {
    "add": "({0} + {1})",
    "sub": "({0} - {1})",
    "mul": "({0} * {1})",
    "div": "({0} / {1})",
    "pow": "({0} ** {1})",
    "mod": "({0} % {1})",
    "eq": "({0} == {1})",
    "ne": "({0} != {1})",
    "lt": "({0} < {1})",
    "le": "({0} <= {1})",
    "gt": "({0} > {1})",
    "ge": "({0} >= {1})",
    "and": "({0} & {1})",
    "or": "({0} | {1})",
    "arctan2": "arctan2({0}, {1})",
}
_NX_UNOPS = {
    "neg": "(-{0})",
    "sqrt": "sqrt({0})",
    "exp": "exp({0})",
    "expm1": "expm1({0})",
    "log": "log({0})",
    "log10": "log10({0})",
    "log1p": "log1p({0})",
    "abs": "abs({0})",
    "sin": "sin({0})",
    "cos": "cos({0})",
    "tan": "tan({0})",
    "arcsin": "arcsin({0})",
    "arccos": "arccos({0})",
    "arctan": "arctan({0})",
    "sinh": "sinh({0})",
    "cosh": "cosh({0})",
    "tanh": "tanh({0})",
    "floor": "floor({0})",
    "ceil": "ceil({0})",
    "not": "(~{0})",
}
# Ops whose result is a bool mask. `and`/`or`/`not` and a `where` condition fuse only over
# known-bool operands: numexpr's & | ~ are BITWISE, which is only "logical" on real masks.
_NX_BOOL_OPS = {"eq", "ne", "lt", "le", "gt", "ge", "and", "or"}
# numexpr caps the inputs of one compiled expression; past this we materialize a subregion
# and keep going rather than erroring.
_NX_MAX_VARS = 30


def _base_value(n):
    b = n.get("v")
    if not isinstance(b, NdArray):
        raise ValueError("evalGraph: base node does not carry a [NumPy]Array")
    return b.a


def _eval_op(n, op, child):
    """Evaluate one interior graph node, with `child(j)` yielding operand j's value. The single
    source of op semantics, shared by the plain evaluator (child = list lookup) and the fused
    evaluator (child = materialize-on-demand) — so fusion can never change WHAT is computed."""
    if op in _BINOPS:
        a, b = n["a"]
        return _BINOPS[op](child(a), child(b))
    if op in _UNOPS:
        return _UNOPS[op](child(n["a"][0]))
    if op in _REDUCERS:
        axis = n.get("axis")
        if axis is None:
            return _REDUCERS[op](child(n["a"][0]))
        return _REDUCERS[op](child(n["a"][0]), axis=axis)
    if op in _CUMS:
        return _CUMS[op](child(n["a"][0]), axis=n.get("axis"))
    if op == "clip":
        x, lo, hi = n["a"]
        return np.clip(child(x), child(lo), child(hi))
    if op == "sort":
        return np.sort(child(n["a"][0]), axis=n.get("axis", -1))
    if op == "argsort":
        return np.argsort(child(n["a"][0]), axis=n.get("axis", -1))
    if op == "unique":
        return np.unique(child(n["a"][0]))
    if op == "searchsorted":
        a, v = n["a"]
        return np.searchsorted(child(a), child(v))
    if op == "take":
        x, idx = n["a"]
        return np.take(child(x), child(idx))
    if op == "concat":
        return np.concatenate([child(j) for j in n["a"]], axis=n.get("axis", 0))
    if op == "stack":
        return np.stack([child(j) for j in n["a"]], axis=n.get("axis", 0))
    if op == "tile":
        reps = n["reps"]
        return np.tile(child(n["a"][0]), tuple(reps) if isinstance(reps, list) else reps)
    if op == "repeat":
        return np.repeat(child(n["a"][0]), n["n"], axis=n.get("axis"))
    if op == "flip":
        return np.flip(child(n["a"][0]), axis=n.get("axis"))
    if op == "roll":
        return np.roll(child(n["a"][0]), n["shift"], axis=n.get("axis"))
    if op == "squeeze":
        return np.squeeze(child(n["a"][0]))
    if op == "expanddims":
        return np.expand_dims(child(n["a"][0]), n["axis"])
    if op == "swapaxes":
        return np.swapaxes(child(n["a"][0]), n["a1"], n["a2"])
    if op == "transpose":
        return np.transpose(child(n["a"][0]))
    if op == "flatten":
        return np.ravel(child(n["a"][0]))
    if op == "reshape":
        return np.reshape(child(n["a"][0]), tuple(n["shape"]))
    if op == "slice":
        return child(n["a"][0])[n["start"] : n["stop"] : n.get("step")]
    if op == "index":
        return child(n["a"][0])[n["i"]]
    if op == "col":
        return child(n["a"][0])[:, n["i"]]
    if op == "select":
        x, mask = n["a"]
        return child(x)[np.asarray(child(mask), dtype=bool)]
    if op == "where":
        c, x, y = n["a"]
        return np.where(child(c), child(x), child(y))
    raise ValueError(f"evalGraph: unknown op '{op}'")


def _fusion_wanted(nodes):
    """Fuse only when enabled AND the graph touches a base array big enough that one numexpr
    pass beats its per-evaluate overhead (the QN_NUMPY_NUMEXPR_MIN crossover)."""
    if not _NUMEXPR_ENABLED:
        return False
    biggest = 0
    for n in nodes:
        if n.get("op") == "base":
            b = n.get("v")
            if isinstance(b, NdArray) and b.a.size > biggest:
                biggest = b.a.size
    return biggest >= _NUMEXPR_MIN


def _eval_fused(nodes, root_idx):
    """The fused evaluator: fusible elementwise nodes accumulate a numexpr expression string
    instead of executing; a fused region materializes — ONE numexpr.evaluate, a single
    blocked + multithreaded pass with no intermediate temporaries — only when something
    non-fusible needs its value: a plain-path consumer, a diamond (shared node, still
    evaluated exactly once), the input-count cap, or the graph root. A whole-array sum/prod
    at the ROOT folds into the fused pass itself (numexpr allows a full reduction only
    outermost, so reductions are never inlined into a bigger expression). Node semantics are
    `_eval_op`'s; fusion only changes how many passes run."""
    uses = [0] * len(nodes)
    for n in nodes:
        for j in n.get("a", ()):
            uses[j] += 1

    vals = [None] * len(nodes)
    have = [False] * len(nodes)
    fused = [None] * len(nodes)  # i -> (expr, kind, {var name: node idx}) while deferred

    def force(i):
        """The node's VALUE, running its deferred fused region now if it still has one."""
        if have[i]:
            return vals[i]
        expr, _, var_map = fused[i]
        vals[i] = numexpr.evaluate(
            expr, local_dict={nm: force(j) for nm, j in var_map.items()}
        )
        have[i] = True
        return vals[i]

    def operand(i):
        """A child's contribution to its consumer's expression: the child's own
        (expr, kind, vars) inlined when deferred and single-use; otherwise a bound
        variable, 'bool'-kinded only when statically or runtime-known."""
        if not have[i] and fused[i] is not None and uses[i] == 1:
            return fused[i]
        name = f"v{i}"
        if have[i]:
            v = vals[i]
            kind = "bool" if isinstance(v, np.ndarray) and v.dtype == np.bool_ else "num"
        elif fused[i] is not None:
            kind = fused[i][1]
        else:
            kind = "num"
        return (name, kind, {name: i})

    def try_fuse(n, op):
        """(expr, kind, vars) for a fusible node, or None to leave it on the plain path."""
        a = n.get("a", ())
        if op in _NX_BINOPS and len(a) == 2:
            ea, ka, va = operand(a[0])
            eb, kb, vb = operand(a[1])
            if op in ("and", "or") and not (ka == "bool" and kb == "bool"):
                return None
            kind = "bool" if op in _NX_BOOL_OPS else "num"
            return (_NX_BINOPS[op].format(ea, eb), kind, {**va, **vb})
        if op in _NX_UNOPS and len(a) == 1:
            ea, ka, va = operand(a[0])
            if op == "not" and ka != "bool":
                return None
            return (_NX_UNOPS[op].format(ea), "bool" if op == "not" else "num", va)
        if op == "where" and len(a) == 3:
            ec, kc, vc = operand(a[0])
            ex, _, vx = operand(a[1])
            ey, _, vy = operand(a[2])
            if kc != "bool":
                return None
            return (f"where({ec}, {ex}, {ey})", "num", {**vc, **vx, **vy})
        if op in ("sum", "prod") and len(a) == 1 and n.get("axis") is None:
            # Reached only for the root (see the loop below): fold the whole-array
            # reduction over a deferred single-use child into one fused pass.
            j = a[0]
            if not have[j] and fused[j] is not None and uses[j] == 1:
                ej, _, vj = fused[j]
                return (f"{op}({ej})", "num", vj)
        return None

    for i, n in enumerate(nodes):
        op = n["op"]
        if op == "base":
            vals[i] = _base_value(n)
            have[i] = True
        elif op == "const":
            vals[i] = n["v"]
            have[i] = True
        else:
            fusible = i == root_idx or op not in ("sum", "prod")
            f = try_fuse(n, op) if fusible else None
            if f is not None and len(f[2]) <= _NX_MAX_VARS:
                fused[i] = f
            else:
                vals[i] = _eval_op(n, op, force)
                have[i] = True
    return force(root_idx)


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
        either side. When numexpr is importable and the graph is big enough to profit
        (QN_NUMPY_NUMEXPR_MIN elements; QN_NUMPY_NO_NUMEXPR=1 disables), maximal elementwise
        regions run as single fused passes (`_eval_fused`) instead of node-at-a-time. Returns a
        new NdArray instance (array root) or a scalar (reduction root). The receiver is just
        the dispatch anchor (it also appears as a base node)."""
        if not isinstance(tree, dict) or "nodes" not in tree or "root" not in tree:
            raise ValueError("evalGraph: expects #{ 'nodes': ..., 'root': ... }")
        nodes = tree["nodes"]
        if _fusion_wanted(nodes):
            root = _eval_fused(nodes, tree["root"])
        else:
            vals = [None] * len(nodes)
            for i, n in enumerate(nodes):
                op = n["op"]
                if op == "base":
                    vals[i] = _base_value(n)
                elif op == "const":
                    vals[i] = n["v"]
                else:
                    vals[i] = _eval_op(n, op, vals.__getitem__)
            root = vals[tree["root"]]
        if isinstance(root, np.ndarray):
            # A 0-d array IS a scalar to Quoin (numexpr's full reductions come back 0-d).
            if root.ndim == 0:
                return root.item()
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
