# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-10 on `perf/block-scalar-spec` @ `ed54325` (main
post #85 plus the block-speculation arc S0-S5), Apple Silicon
(darwin25); this run adds the `fib_untyped_32` row. The previous table
(2026-07-06, the ic-direct-calls tip `80a209b`) is preserved below for
the delta story.*
## Methodology

- **Whole-process wall time** (startup + load + compile + run), median of 5
  — the canonical Quoin measurement basis (`bench/README.md`).
- **Quoin: the PGO binary** (`scripts/build-pgo.sh` → `target/release/qn-pgo`),
  per the standing rule for cross-language comparisons. AOT on (the default).
- **Python 3.13.8** (`python3.13`), **Ruby 4.0.5** (Homebrew), plain and
  `--yjit`.
- Ports live in `bench/py/` and `bench/rb/`: same algorithm, same workload,
  same frozen checksums (verified every run by `bench/cross.py`). Fidelity
  notes are in each file's header; the notable resolutions: Quoin's
  `splitString:' '` keeps trailing empty fields (Ruby needed `split(/ /, -1)`,
  not the awk-form `split(' ')`); `index:` is 0-based; all three JSON
  serializers happen to emit byte-identical compact output for the test
  document, so even the json checksum is shared. richards uses each
  language's native `^ 0xD008` where Quoin hand-rolls xor (no bitwise ops);
  combinators uses closure-per-element forms in Python (`map`/`filter` with
  lambdas, not comprehensions) because closure invocation is what the
  benchmark measures — Ruby blocks are the exact analogue.
- Startup floors (empty program, median): quoin-pgo **8ms**, python3.13
  **14ms**, ruby **24ms** (±yjit). Quoin has the lowest floor, so
  whole-process ratios on the short benches are not flattered by startup.

## Results (seconds, median of 7; ratios = other/quoin, >1 ⇒ Quoin faster)

| bench          | quoin | python | ruby  | rb-yjit | py/qn | rb/qn | yjit/qn |
|----------------|------:|-------:|------:|--------:|------:|------:|--------:|
| btrees         | 0.287 | 0.193  | 0.121 | 0.061   | 0.67  | 0.42  | 0.21    |
| combinators    | 0.097 | 0.047  | 0.050 | 0.046   | 0.49  | 0.52  | 0.47    |
| fib_typed      | 0.035 | 0.191  | 0.119 | 0.041   | **5.52** | **3.44** | **1.18** |
| fib_untyped    | 0.020 | 0.083  | 0.061 | 0.032   | **4.15** | **3.02** | **1.60** |
| fib_untyped_32 | 0.035 | 0.191  | 0.119 | 0.041   | **5.51** | **3.44** | **1.18** |
| json           | 0.288 | 0.220  | 0.119 | 0.119   | 0.76  | 0.41  | 0.41    |
| maps           | 0.152 | 0.074  | 0.086 | 0.078   | 0.49  | 0.57  | 0.51    |
| richards       | 0.313 | 0.096  | 0.081 | 0.046   | 0.31  | 0.26  | 0.15    |
| sieve          | 0.105 | 0.302  | 0.192 | 0.099   | **2.89** | **1.83** | 0.95 |
| strings        | 0.079 | 0.046  | 0.073 | 0.069   | 0.58  | **0.92** | 0.87 |
| **geomean**    |       |        |       |         | **1.23** | 0.98  | 0.60 |

**Geomeans are not comparable across tables with different row sets**: this
table adds `fib_untyped_32` (a Quoin-favorable row), which alone moves the
py/qn geomean 1.04 → 1.23. On the previous nine-bench set this run measures
**1.04** — that is the number to compare against the 07-06 table's 1.07.

`fib_untyped_32` is `fib_typed`'s exact workload (n = 32) with the
annotations deleted — the row that makes the **speculation tax** directly
readable. Verdict: **zero at this scale** — 0.035s vs 0.035s, identical to
the run's precision. The tax is a fixed warm-up (the interpreted prefix
before promotion at OBSERVE_CAP plus entry preconditions), visible only on
the smaller fib_untyped row (n = 30), where ~3ms of it shows against ~13ms
of compute.

Previous table (2026-07-06, ic-direct-calls tip `80a209b`): btrees
0.320 (py/qn 0.61), combinators 0.126, fib_typed 0.028, fib_untyped
0.016, json 0.231, maps 0.138, richards 0.314, sieve 0.101, strings
0.077; geomeans **1.07** / 0.87 / 0.57.

## Honest reading

**The block-speculation arc (S0-S5) moved its target row and repaired a
regression.** combinators 0.126 → **0.100** (−21%): block-template
arguments now speculate scalar kinds through the B3a warmth window, so
`(x * 3) + 1` inside a `collect:` block devirts to native arithmetic
instead of paying two classic outcalls per element, and interpreted
fused `each:` sites route to compiled speculated blocks (the splice had
been starving them). The per-element cost ladder that scoped the arc is
now permanent (`bench/micro/`, `run.py` interleaves two binaries).

**fib_untyped 0.077 → 0.021** — not an arc win but a repair: the F1
strict-Boolean fix (PR #77) had silently evicted every untyped
conditional-bearing method from the scalar-pure set (the syntactic scan
is reachability-blind and saw the guard's dead cold span), which killed
direct self-recursion and demoted the speculated scalar return to Obj —
an unnoticed 8x on the headline row. The pure-set scan now masks
guarded-conditional cold spans.

Two drifts vs the 07-06 table are NOT explained by this branch (every
row measured flat-or-better under interleaved A/B against main):

1. **A ~5-9ms across-the-board startup tax** — visible on rows with no
   arc-relevant code at all (strings +5ms, sieve +5ms, richards +7ms,
   fib_typed +9ms). Between the tables sit ~25 merges including the
   embedded/relocatable stdlib and the language-reference-era prelude
   growth. fib_untyped's floor moved the same way (pre-F1 checkout
   measures 0.01 profiling where today's repaired build is 0.02).
2. **json +32%** (0.231 → 0.305) — exceeds the startup tax; untracked.

What remains behind, in measured order: the outcall shell (richards
3.2x behind CPython; D3's direct-edge tier ships default-off — its gate
measured net-negative), the remaining combinator gap (2x; the ladder
puts the hand-fused ceiling at 0.72 vs 0.94 on the scaled workload),
and the maps/json/strings residue — now joined by the two drifts above
as the cheapest opens.

## Reproducing

```sh
bash scripts/build-pgo.sh
python3 bench/cross.py --qn target/release/qn-pgo --runs 5
```
