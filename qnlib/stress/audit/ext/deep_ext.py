#!/usr/bin/env python3
# A buggy/malicious extension that returns a DEEPLY NESTED structured value, to probe whether
# the host's recursive decode (decode_dv / wire_to_runtime / data_to_value, and planus verify)
# overflows the host VM's stack -> host crash (SIGSEGV/abort) triggered by extension output.
import sys
import os
import threading

sys.path.insert(0, "/Users/damon/code/building_blocks_vm/sdk/python")
sys.setrecursionlimit(2_000_000)

from quoin_ext import serve

DEPTH = 200_000


def build_deep(n):
    v = 0
    for _ in range(n):
        v = [v]  # nest one list deep, iteratively (no Python recursion here)
    return v


def handler(host, op, arg):
    if op == "deep":
        # Encoding recurses in Python; do it on a big-stack thread.
        result = {}

        def enc():
            from quoin_ext import _encode_call_return_data  # noqa
            result["frame"] = _encode_call_return_data(build_deep(DEPTH))

        t = threading.Thread(target=enc)
        t.start()
        t.join()
        # Return via the normal path: the SDK will re-encode. To avoid double Python recursion
        # cost, just return the nested list and let serve() encode it on a big-stack thread too.
        return build_deep(DEPTH)
    return "pong"


# Run serve on a thread with a large stack so Python-side encode recursion doesn't crash US;
# the point is to crash the HOST, not the extension.
threading.stack_size(512 * 1024 * 1024)
t = threading.Thread(target=lambda: serve(sys.argv[1], handler))
t.start()
t.join()
