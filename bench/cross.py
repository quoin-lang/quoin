#!/usr/bin/env python3
"""Cross-language benchmark runner: Quoin vs Python vs Ruby.

Runs each benchmark as a whole process (the canonical Quoin measurement —
startup, load, compile, run), takes the median of N runs, verifies every
run printed its `<name>: ok` line, and prints a table with ratios
normalized to Quoin.

Usage:
    python3 bench/cross.py --qn ./qn-pgo --runs 5
    python3 bench/cross.py --only combinators,richards

The Quoin binary should be the PGO build (scripts/build-pgo.sh) — the
standing rule for cross-language comparisons. Ruby runs both plain and
with --yjit.
"""

import argparse
import json as jsonlib
import statistics
import subprocess
import sys
import time
from pathlib import Path

BENCH_DIR = Path(__file__).parent
BENCHES = [
    "btrees",
    "combinators",
    "fib_typed",
    "fib_untyped",
    "fib_untyped_32",
    "json",
    "maps",
    "richards",
    "sieve",
    "strings",
]
# json's source files dodge the stdlib-module name.
FILE_OVERRIDES = {"json": {"py": "json_bench.py", "rb": "json_bench.rb"}}


def source_for(bench: str, lang: str) -> Path:
    name = FILE_OVERRIDES.get(bench, {}).get(lang, f"{bench}.{lang}")
    return BENCH_DIR / lang / name


def run_once(cmd: list[str], ok_line: str) -> float:
    t0 = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    elapsed = time.perf_counter() - t0
    last = (proc.stdout.strip().splitlines() or [""])[-1]
    if proc.returncode != 0 or last != ok_line:
        raise RuntimeError(
            f"{' '.join(cmd)}: expected {ok_line!r}, got {last!r} "
            f"(rc={proc.returncode}, stderr={proc.stderr.strip()[:200]})"
        )
    return elapsed


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--qn", default="target/release/qn")
    ap.add_argument("--python", default="python3.13")
    ap.add_argument("--ruby", default="/opt/homebrew/opt/ruby/bin/ruby")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--only")
    ap.add_argument("--save")
    args = ap.parse_args()

    benches = args.only.split(",") if args.only else BENCHES
    langs = {
        "quoin": lambda b: [args.qn, str(BENCH_DIR / "qn" / f"{b}.qn")],
        "python": lambda b: [args.python, str(source_for(b, "py"))],
        "ruby": lambda b: [args.ruby, str(source_for(b, "rb"))],
        "ruby-yjit": lambda b: [args.ruby, "--yjit", str(source_for(b, "rb"))],
    }

    results: dict[str, dict[str, float]] = {}
    for bench in benches:
        ok_line = f"{bench}: ok"
        results[bench] = {}
        for lang, cmd_for in langs.items():
            cmd = cmd_for(bench)
            times = [run_once(cmd, ok_line) for _ in range(args.runs)]
            results[bench][lang] = statistics.median(times)
            print(f"  {bench:<12} {lang:<7} {results[bench][lang]:8.3f}s", flush=True)

    print()
    hdr = (
        f"{'bench':<13}{'quoin':>9}{'python':>9}{'ruby':>9}{'rb-yjit':>9}"
        f"{'py/qn':>8}{'rb/qn':>8}{'yjit/qn':>9}"
    )
    print(hdr)
    for bench in benches:
        r = results[bench]
        q, p, rb, ry = r["quoin"], r["python"], r["ruby"], r["ruby-yjit"]
        print(
            f"{bench:<13}{q:>9.3f}{p:>9.3f}{rb:>9.3f}{ry:>9.3f}"
            f"{p / q:>8.2f}{rb / q:>8.2f}{ry / q:>9.2f}"
        )
    import math

    geo = lambda xs: math.exp(sum(math.log(x) for x in xs) / len(xs))
    ratios = {
        lang: [results[b][lang] / results[b]["quoin"] for b in benches]
        for lang in ("python", "ruby", "ruby-yjit")
    }
    print(
        f"{'geomean':<13}{'':>9}{'':>9}{'':>9}{'':>9}"
        f"{geo(ratios['python']):>8.2f}{geo(ratios['ruby']):>8.2f}"
        f"{geo(ratios['ruby-yjit']):>9.2f}"
    )

    if args.save:
        Path(args.save).write_text(jsonlib.dumps(results, indent=2))
        print(f"saved: {args.save}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
