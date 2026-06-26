#!/usr/bin/env python3
"""A Python extension exercising the *full* host surface — proving the Python SDK is at parity
with the Rust one (Slice 7b; see ``tests/extension.rs``). It implements the same ops as the Rust
fixtures, across every capability:

- ``compute`` — host-ops: ``make_string`` + ``call_method`` (``+:`` then ``upper``). ``"ab" -> "AB!"``.
- ``mapUpper`` — batched callback: ``invoke_block`` runs the passed host block over ``a,b,c`` in one
  round-trip (the block arrives as ``host.handles()[0]``). ``-> "A,B,C"``.
- ``new`` / ``inc`` / ``live`` — ext-resource handles: a counter registry; ``new`` returns a
  ``Resource``, ``inc`` reads ``host.resources()[0]``, ``live`` counts; dropped resources arrive via
  ``host.releases()`` and are freed at the top of each call.
- ``sum`` / ``scale`` — the Array data plane: read ``host.arrays()[0]`` and return a scalar or a new
  ``ArrowArray``.

A test/example fixture, not a shipped feature."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import quoin_ext
from quoin_ext import ArrowArray, Resource


class FullHandler:
    def __init__(self):
        self._counters = {}  # id -> value
        self._next_id = 1

    def __call__(self, host, op, arg):
        # Free any resources the host has dropped (batched onto this call).
        for rid in host.releases():
            self._counters.pop(rid, None)

        if op == "compute":
            base = host.make_string(arg)
            suffix = host.make_string("!")
            joined = host.call_method(base, "+:", [suffix])  # "<arg>!"
            upper = host.call_method(joined, "upper", [])
            return host.handle_to_string(upper)

        if op == "mapUpper":
            block = host.handles()[0]
            batches = [[host.make_string(s)] for s in ("a", "b", "c")]
            results = host.invoke_block(block, batches)
            return ",".join(host.handle_to_string(h) for h in results)

        if op == "new":
            rid = self._next_id
            self._next_id += 1
            self._counters[rid] = 0
            return Resource(rid)

        if op == "inc":
            rid = host.resources()[0]
            self._counters[rid] += 1
            return str(self._counters[rid])

        if op == "live":
            return str(len(self._counters))

        if op == "sum":
            return str(sum(host.arrays()[0].as_floats()))

        if op == "scale":
            factor = float(arg)
            return ArrowArray.from_floats([x * factor for x in host.arrays()[0].as_floats()])

        return f"unknown op: {op}"


if __name__ == "__main__":
    quoin_ext.serve(sys.argv[1], FullHandler())
