#!/usr/bin/env python3
# A well-behaved extension SDK-wise, but each call sleeps briefly before replying, so two
# concurrent host calls on the same connection are forced to overlap. Echoes `arg`.
import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "..", "..",
                                "code", "building_blocks_vm", "sdk", "python"))
# Fall back to absolute
sys.path.insert(0, "/Users/damon/code/building_blocks_vm/sdk/python")
from quoin_ext import serve


def handler(host, op, arg):
    time.sleep(0.2)
    return f"{op}:{arg}"


serve(sys.argv[1], handler)
