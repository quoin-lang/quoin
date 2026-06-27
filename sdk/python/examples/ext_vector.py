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

    def scale(self, factor):
        return Vector([x * factor for x in self.data])


if __name__ == "__main__":
    ext = quoin_ext.Extension()
    ext.register(
        "Vector",
        Vector,
        constructors={"ofFloats:": Vector},
        methods={"sum": Vector.sum, "length": Vector.length, "scale:": Vector.scale},
    )
    ext.serve(sys.argv[1])
