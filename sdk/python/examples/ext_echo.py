#!/usr/bin/env python3
"""A tiny out-of-process Quoin extension written in Python, exercising the Tier-1 transport
from a non-Rust SDK (Slice 7; see ``tests/extension.rs``). The VM spawns it with a socket path
as ``argv[1]``; it serves two scalar ops over the ``quoin_ext`` Python SDK: ``echo`` (returns
the arg) and ``upper`` (uppercases it). A test/example fixture, not a shipped feature."""

import os
import sys

# Make the in-repo `quoin_ext` package importable (this file lives at sdk/python/examples/).
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import quoin_ext


def handler(host, op, arg):  # scalar-only: ignores `host`
    if op == "echo":
        return arg
    if op == "upper":
        return arg.upper()
    return f"unknown op: {op}"


if __name__ == "__main__":
    quoin_ext.serve(sys.argv[1], handler)
