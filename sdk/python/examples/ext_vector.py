#!/usr/bin/env python3
"""A *Python* extension that provides the Quoin class ``Vector`` (Phase 3, extension-backed classes
— the Python parity of ``src/bin/ext_vector.rs``; see ``tests/extension.rs``). The SDK owns the
instances, so the class is just a plain Python class plus a selector -> method mapping:

- ``Vector ofFloats: aList`` (class-side constructor) -> a new ``Vector`` instance.
- ``v sum`` / ``v length`` (instance methods) -> a Double / an Integer.
- ``v scale: f`` (instance method) -> a new ``Vector``; the SDK detects the returned instance with
  ``isinstance`` and wraps it as a resource — no explicit ``makes`` needed (unlike the Rust SDK).

A test/example fixture, not a shipped feature."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import quoin_ext


class Vector:
    """A plain Python class — the SDK keeps instances in its object table keyed by an opaque id."""

    def __init__(self, data):
        self.data = [float(x) for x in data]

    def sum(self):
        return sum(self.data)

    def length(self):
        return len(self.data)

    def at(self, i):
        # A *fallible* method: out-of-range raises, which the SDK turns into a `CallReturnError` so
        # the host raises a catchable Quoin error and the extension stays alive.
        i = int(i)
        if i < 0 or i >= len(self.data):
            raise IndexError(f"index {i} out of range (length {len(self.data)})")
        return self.data[i]

    def scale(self, factor):
        return Vector([x * factor for x in self.data])

    def dot(self, other):
        # `other` is a live `Vector` instance — an ext-object argument.
        return sum(a * b for a, b in zip(self.data, other.data))

    def map(self, block):
        # `block` is a callable wrapping a host block — apply it to each element.
        return Vector([block(x) for x in self.data])


class Matrix:
    """A second class — ``row`` returns a ``Vector``, exercising cross-class returns (the SDK detects
    the returned ``Vector`` with ``isinstance`` and names it so the host wraps it correctly)."""

    def __init__(self, rows):
        self.rows = [[float(x) for x in row] for row in rows]

    def rowCount(self):
        return len(self.rows)

    def row(self, i):
        return Vector(self.rows[int(i)])


if __name__ == "__main__":
    ext = quoin_ext.Extension()
    ext.register(
        "Vector",
        Vector,
        constructors={"ofFloats:": Vector},
        methods={
            "sum": Vector.sum,
            "length": Vector.length,
            "at:": Vector.at,
            "scale:": Vector.scale,
            "dot:": Vector.dot,
            "map:": Vector.map,
        },
    )
    ext.register(
        "Matrix",
        Matrix,
        constructors={"ofRows:": Matrix},
        methods={"rowCount": Matrix.rowCount, "row:": Matrix.row},
    )
    ext.serve(sys.argv[1])
