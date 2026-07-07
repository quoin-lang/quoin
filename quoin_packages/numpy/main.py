#!/usr/bin/env python3
"""The `numpy` extension — NumPy-backed n-dimensional arrays as the Quoin class `[NumPy]Array`.

Slice 1 (eager skeleton): creation, introspection, and the materialization exit ramps. Every
`[NumPy]Array` instance lives in this process (the SDK's object table); Quoin holds an opaque
handle and each method send is one socket round-trip. Bulk data crosses the boundary only at the
explicit exit ramps (`toArray` / `toList`) — whole-array ops keep the data resident here
(docs/FUTURE_EXT_ARCH.md §8). The lazy expression DAG (`evalGraph:`) and operators arrive in the
next slice; the Quoin-side glue lives in `init.qn`.

Dtype policy (v1): every array is `float64` or `int64` (the two wire `ArrowDType`s). Other
integer/float widths are widened on entry; bool (masks) arrives with comparisons in a later
slice; anything else is a clear error.

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


def _coerce(a):
    """Coerce an ndarray to the v1 dtype policy (float64 | int64), or raise a clear error."""
    if a.dtype == np.float64 or a.dtype == np.int64:
        return a
    if a.dtype == np.bool_:
        raise ValueError("bool arrays arrive with comparisons/masks (not yet supported)")
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

    # --- materialization exit ramps (bulk leaves this process here, and only here) ---

    def toList(self):
        return self.a.tolist()

    def toArray(self):
        # The host bulk `Array` is a 1-D column; n-D flattens row-major (shape travels via
        # `shape`). `<f8`/`<i8` pins little-endian (the Arrow layout contract) — a no-op copy
        # on LE hosts.
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


if __name__ == "__main__":
    ext = quoin_ext.Extension()
    ext.register(
        "Array",
        NdArray,
        constructors={
            "zeros:": zeros,
            "ones:": ones,
            "fromList:": from_list,
            "arange:": arange,
            "linspace:to:count:": linspace,
            "random:": random,
        },
        methods={
            "shape": NdArray.shape,
            "dtype": NdArray.dtype,
            "size": NdArray.size,
            "ndim": NdArray.ndim,
            "s": NdArray.s,
            "at:": NdArray.at,
            "toList": NdArray.toList,
            "toArray": NdArray.toArray,
        },
    )
    ext.serve(sys.argv[1])
