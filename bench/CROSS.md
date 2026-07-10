# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-10 on `perf/block-scalar-spec` @ `41a1c80` (main
post #85 plus the block-speculation arc S0-S5), Apple Silicon
(darwin25). The previous table (2026-07-06, the ic-direct-calls tip
`80a209b`) is preserved below for the delta story.*
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

| bench        | quoin | python | ruby  | rb-yjit | py/qn | rb/qn | yjit/qn |
|--------------|------:|-------:|------:|--------:|------:|------:|--------:|
| btrees       | 0.295 | 0.200  | 0.124 | 0.063   | 0.68  | 0.42  | 0.21    |
| combinators  | 0.100 | 0.049  | 0.051 | 0.050   | 0.49  | 0.51  | 0.49    |
| fib_typed    | 0.037 | 0.198  | 0.124 | 0.043   | **5.39** | **3.36** | **1.16** |
| fib_untyped  | 0.021 | 0.085  | 0.063 | 0.032   | **4.03** | **2.98** | **1.50** |
| json         | 0.305 | 0.227  | 0.122 | 0.122   | 0.75  | 0.40  | 0.40    |
| maps         | 0.155 | 0.074  | 0.088 | 0.080   | 0.48  | 0.57  | 0.52    |
| richards     | 0.321 | 0.099  | 0.083 | 0.047   | 0.31  | 0.26  | 0.15    |
| sieve        | 0.106 | 0.317  | 0.196 | 0.102   | **2.98** | **1.84** | 0.96 |
| strings      | 0.082 | 0.047  | 0.074 | 0.071   | 0.57  | **0.91** | 0.86 |
| **geomean**  |       |        |       |         | **1.03** | 0.84  | 0.55 |

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
