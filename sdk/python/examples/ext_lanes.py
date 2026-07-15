#!/usr/bin/env python3
"""A lane-declaring Python extension (see ``tests/extension.rs``): ``Extension(lanes=2)``
invites the host to open two connections, each served on its own thread — so two calls to
*different* ``Slot`` instances overlap even from Python (the handler blocks in
``time.sleep``, which releases the GIL, standing in for a DB driver or socket wait).
A test/example fixture, not a shipped feature."""

import os
import sys
import time

# Make the in-repo `quoin_ext` package importable (this file lives at sdk/python/examples/).
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import quoin_ext


class Slot:
    def __init__(self, tag):
        self.tag = tag


def slow_tag(slot):
    time.sleep(0.15)
    return slot.tag


def main():
    ext = quoin_ext.Extension(lanes=2)
    ext.register(
        "Slot",
        Slot,
        constructors={"make:": Slot},
        methods={"tag": lambda s: s.tag, "slowTag": slow_tag},
    )
    ext.serve(sys.argv[1])


if __name__ == "__main__":
    main()
