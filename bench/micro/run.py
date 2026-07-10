#!/usr/bin/env python3
"""Per-element cost ladder for the block/combinator seam (bench/micro/*.qn).

Each file visits 20M elements and isolates one layer of the per-element cost:

    fused-loop      the floor: hand-fused at:-loop in a compiled method
    block-identity  + one block call and a list append per element
    block-arith     + scalar sends INSIDE the block body
    each-arith      the same block via a direct `data.each:` site
    each-capture    + a captured-accumulator write per element

Whole-process wall time (startup included), median of N runs. With --vs the
two binaries run INTERLEAVED (A B A B ...) — sequential sweeps drift 2-3%.

    python3 bench/micro/run.py --qn target/release/qn --runs 5
    python3 bench/micro/run.py --qn ./qn-before --vs ./qn-after
"""

import argparse
import statistics
import subprocess
import sys
import time
from pathlib import Path

FILES = [
    "fused-loop",
    "block-identity",
    "block-arith",
    "each-arith",
    "each-capture",
]
ELEMENTS = 20_000_000


def run_once(qn: str, path: Path) -> float:
    t0 = time.monotonic()
    out = subprocess.run(
        [qn, str(path)], capture_output=True, text=True, check=False
    )
    dt = time.monotonic() - t0
    if out.returncode != 0 or ": ok" not in out.stdout:
        sys.exit(f"{qn} {path.name}: FAILED\n{out.stdout}{out.stderr}")
    return dt


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--qn", default="target/release/qn")
    ap.add_argument("--vs", help="second binary, interleaved A/B")
    ap.add_argument("--runs", type=int, default=5)
    args = ap.parse_args()

    root = Path(__file__).parent
    binaries = [args.qn] + ([args.vs] if args.vs else [])
    times: dict[tuple[str, str], list[float]] = {}
    for name in FILES:
        path = root / f"{name}.qn"
        for _ in range(args.runs):
            for qn in binaries:  # interleave binaries within each run
                times.setdefault((qn, name), []).append(run_once(qn, path))

    width = max(len(n) for n in FILES)
    header = f"{'file':<{width}}" + "".join(f"  {qn:>18}" for qn in binaries)
    print(header + ("  {:>8}".format("delta") if args.vs else ""))
    for name in FILES:
        meds = [statistics.median(times[(qn, name)]) for qn in binaries]
        cells = "".join(
            f"  {m:8.3f}s {m / ELEMENTS * 1e9:5.1f}ns/el" for m in meds
        )
        delta = (
            f"  {(meds[1] - meds[0]) / meds[0] * 100:+7.1f}%" if args.vs else ""
        )
        print(f"{name:<{width}}{cells}{delta}")


if __name__ == "__main__":
    main()
