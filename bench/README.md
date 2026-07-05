# Quoin benchmark suite

Whole-process benchmarks for the VM. Each `qn/*.qn` file is a standalone
program that runs a workload, verifies a frozen checksum, and prints
`<name>: ok` (or `<name>: FAIL got ...`) as its last line. `run.py` times the
full `qn <file>` process — startup, qnlib load, compile, run — which is the
canonical way to measure real Quoin performance (`qn benchmark` and
`Timer.time:` both hide the driver-yield and startup costs).

## Running

```sh
cargo build --release
python3 bench/run.py                       # all benches against target/release/qn
python3 bench/run.py --compare ./qn-other  # A/B two binaries
python3 bench/run.py --only richards --runs 10
```

The runner exits non-zero if any benchmark fails its checksum, so it can
gate CI or an A/B comparison: a binary that gets faster by computing the
wrong answer fails loudly.

## The benchmarks

Three carried over from `qnlib/benchmark.qn` (the canonical trio all past
profiling work was tuned against), six added to cover what that trio never
exercised:

| bench | workload | what it stresses |
|---|---|---|
| `fib_typed` | recursive fib(32), typed | devirtualized Integer ops, typed calling path, inlining |
| `fib_untyped` | recursive fib(30), untyped | full dynamic dispatch: inline cache + method cache |
| `sieve` | sieve of Eratosthenes, 400×10k | while-loops, List `at:`/`at:put:` devirt |
| `btrees` | CLBG binary trees, depth 12 | object allocation + GC (the alloc-bound bench) |
| `richards` | Octane Richards port, 50 rounds | **polymorphic dispatch** (one send site over 4 task classes), accessors, linked lists |
| `combinators` | collect:/select:/sum/detect: pipelines | **the pure-Quoin Iterate mixin**: closure creation, block calls |
| `strings` | concat/split/search/case/join | **string representation** (currently 2 GC allocs per string) |
| `maps` | word-frequency count + keys walk | string-keyed native Map churn |
| `json` | JSON.generate:/parse: round-trip | native↔Quoin conversion, mixed Map/List/String docs |

Sizing: each bench targets roughly 0.3–1.5s on a release build so
whole-process timing is meaningful but the suite stays fast.

`richards` is a faithful port of the Octane benchmark — the idle task's
`(v1 >> 1) ^ 0xD008` is emulated arithmetically (Quoin has no bitwise
operators) and the TCB state bitmask is a state integer + held flag, but the
scheduling behavior is bit-exact: the canonical checksums (queueCount 2322,
holdCount 928 per round) verify every run.

## Adding a benchmark

1. Write `bench/qn/<name>.qn`: deterministic workload, accumulate a checksum,
   end with exactly `'<name>: ok'.print` / `'<name>: FAIL got ...'.print`.
2. Freeze the checksum by running it once and copying the value in.
3. Run it through `qn fmt`.
4. Calibrate iterations to ~0.3–1.5s release-mode whole-process.
