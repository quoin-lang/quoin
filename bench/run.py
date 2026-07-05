#!/usr/bin/env python3
"""Whole-process benchmark runner for the Quoin benchmark suite.

Times each bench/qn/*.qn program as a full `qn <file>` process (startup +
qnlib load + compile + run), which is the project's canonical way to measure
real performance -- `qn benchmark` and Timer.time: both hide the per-batch
driver-yield tax and startup costs.

Every benchmark self-checks and prints `<name>: ok` as its last line; the
runner treats anything else as a failure. Checksums are frozen in the .qn
files, so a run that "speeds up" by computing the wrong answer fails loudly.

Usage:
  python3 bench/run.py                          # all benches, target/release/qn
  python3 bench/run.py --bin path/to/qn         # a specific binary
  python3 bench/run.py --compare path/to/other  # A/B two binaries
  python3 bench/run.py --only richards,strings  # subset
  python3 bench/run.py --runs 10                # more samples
  python3 bench/run.py --save results.json      # machine-readable output
"""

import argparse
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent / "qn"
REPO_ROOT = Path(__file__).resolve().parent.parent


def discover():
    return sorted(p.stem for p in BENCH_DIR.glob("*.qn"))


def run_once(binary, bench):
    path = BENCH_DIR / f"{bench}.qn"
    t0 = time.perf_counter()
    proc = subprocess.run(
        [str(binary), str(path)],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    elapsed = time.perf_counter() - t0
    out = proc.stdout.strip().splitlines()
    last = out[-1] if out else ""
    if proc.returncode != 0 or last != f"{bench}: ok":
        detail = last or proc.stderr.strip().splitlines()[-1:] or "<no output>"
        raise RuntimeError(f"{bench}: bad run ({detail!r}, exit {proc.returncode})")
    return elapsed


def run_bench(binary, bench, runs):
    times = [run_once(binary, bench) for _ in range(runs)]
    return {"min": min(times), "median": statistics.median(times), "times": times}


def fmt_s(seconds):
    return f"{seconds:7.3f}s"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", default=str(REPO_ROOT / "target/release/qn"))
    ap.add_argument("--compare", help="second qn binary to A/B against --bin")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--only", help="comma-separated bench names")
    ap.add_argument("--save", help="write results to a JSON file")
    ap.add_argument("--list", action="store_true", help="list benches and exit")
    args = ap.parse_args()

    benches = discover()
    if args.list:
        print("\n".join(benches))
        return 0
    if args.only:
        wanted = args.only.split(",")
        missing = [w for w in wanted if w not in benches]
        if missing:
            print(f"unknown bench(es): {', '.join(missing)}", file=sys.stderr)
            return 2
        benches = wanted

    bins = [("A", Path(args.bin))]
    if args.compare:
        bins.append(("B", Path(args.compare)))
    for _, b in bins:
        if not b.exists():
            print(f"binary not found: {b}", file=sys.stderr)
            return 2

    for label, b in bins:
        print(f"{label}: {b}")
    print(f"runs per bench: {args.runs}\n")

    results = {}
    failed = False
    for bench in benches:
        row = {}
        for label, b in bins:
            try:
                row[label] = run_bench(b, bench, args.runs)
            except RuntimeError as e:
                print(f"FAIL  {e}", file=sys.stderr)
                failed = True
                row[label] = None
        results[bench] = row

    name_w = max(len(b) for b in benches)
    if args.compare:
        print(f"{'bench':<{name_w}}  {'A min':>8} {'B min':>8} {'Δmin':>7}   {'A med':>8} {'B med':>8} {'Δmed':>7}")
        for bench, row in results.items():
            a, b = row.get("A"), row.get("B")
            if not (a and b):
                print(f"{bench:<{name_w}}  (failed)")
                continue
            dmin = (b["min"] - a["min"]) / a["min"] * 100
            dmed = (b["median"] - a["median"]) / a["median"] * 100
            print(
                f"{bench:<{name_w}}  {fmt_s(a['min'])} {fmt_s(b['min'])} {dmin:+6.1f}%"
                f"   {fmt_s(a['median'])} {fmt_s(b['median'])} {dmed:+6.1f}%"
            )
    else:
        print(f"{'bench':<{name_w}}  {'min':>8} {'median':>8}")
        for bench, row in results.items():
            a = row.get("A")
            if not a:
                print(f"{bench:<{name_w}}  (failed)")
                continue
            print(f"{bench:<{name_w}}  {fmt_s(a['min'])} {fmt_s(a['median'])}")

    if args.save:
        payload = {
            "bins": {label: str(b) for label, b in bins},
            "runs": args.runs,
            "results": results,
        }
        Path(args.save).write_text(json.dumps(payload, indent=2))
        print(f"\nsaved: {args.save}")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
