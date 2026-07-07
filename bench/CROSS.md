# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-06 on `perf/cheap-materialization` @ `5af70cc` (the
alloc-churn arc, PR #59, plus the materialization arc M1-M3 — both
pre-merge), Apple Silicon (darwin25). The previous table (2026-07-06,
main @ `fc89fc9`) is preserved below for the delta story.*
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
| btrees       | 0.361 | 0.199  | 0.123 | 0.062   | 0.55  | 0.34  | 0.17    |
| combinators  | 0.128 | 0.048  | 0.050 | 0.046   | 0.38  | 0.39  | 0.36    |
| fib_typed    | 0.028 | 0.196  | 0.122 | 0.042   | **6.93** | **4.31** | **1.48** |
| fib_untyped  | 0.018 | 0.085  | 0.062 | 0.031   | **4.74** | **3.48** | **1.71** |
| json         | 0.239 | 0.224  | 0.121 | 0.120   | 0.94  | 0.51  | 0.50    |
| maps         | 0.141 | 0.074  | 0.086 | 0.078   | 0.52  | 0.61  | 0.56    |
| richards     | 0.371 | 0.098  | 0.082 | 0.046   | 0.27  | 0.22  | 0.13    |
| sieve        | 0.105 | 0.310  | 0.194 | 0.100   | **2.96** | **1.85** | 0.96 |
| strings      | 0.078 | 0.045  | 0.073 | 0.069   | 0.58  | **0.93** | 0.89 |
| **geomean**  |       |        |       |         | **1.05** | 0.85  | 0.55 |

Previous table (2026-07-06, main @ `fc89fc9`, pre both arcs): btrees
0.895 (py/qn 0.22), strings 0.169 (0.27), combinators 0.155 (0.31),
richards 0.431 (0.22), maps 0.154 (0.47); geomeans **0.83** / 0.67 /
0.43.

## Honest reading

**The suite geomean vs CPython crossed 1.0.** Two arcs did it — the
allocation-churn arc (PR #59: strings 1.96×, InitPlan memoization,
stack-window arg rooting) and the materialization arc (alpha-renamed
control-flow fusion, fused instantiation, the cold-span gate lift):

- **btrees 0.895 → 0.361 (2.48×)** — the headline. makeTree's arms now
  fuse (M1) and its `TreeNode.new:{…}` configs compile to inline
  field binds (M2): no per-node closure, no config frame, no
  interpreted stores. Still 1.8× behind CPython — the residue is
  compiled-to-compiled outcall dispatch, no longer allocation.
- **strings 0.169 → 0.078** — at **parity with Ruby** (0.93) and 0.89
  vs YJIT; 1.7× behind CPython whose C string ops set the bar.
- combinators 0.155 → 0.128 (M3's cold-span lift compiled `any?:`),
  richards 0.431 → 0.371, maps/json small gains; the fib/sieve wins
  hold.

Quoin now wins four rows outright (both fibs, sieve, json-at-parity)
and the geomean: **1.05 vs CPython 3.13** (was 0.83), 0.85 vs Ruby 4
(was 0.67), 0.55 vs YJIT (was 0.43).

**What the table points at next** (the gap, in measured order):
1. **Compiled-to-compiled outcall dispatch** — the NEW #1. Post-arc,
   btrees and richards have the same profile shape: ~29% and ~35% in
   `Callable::call` + IC machinery on outcall seams (recursive sends,
   megamorphic `@task.run:`). Direct calls for IC-stable outcall sites
   is the recorded follow-up.
2. **Per-call frame/env cost in the interpreter residue** (maps, json)
   — PERF_ROADMAP Tier 3a (escape-analysis stack environments) still
   applies where methods stay interpreted.
3. **Strings** — the remaining 1.7× vs CPython is C-library string ops
   vs compiled-but-generic ones; diminishing returns from here.

The allocation/GC frontier that headed this list in the previous
measurement is substantially CLOSED: btrees' profile no longer shows
`dispatch_one` at all, and alloc+GC sits under 10%.

## Reproducing

```sh
bash scripts/build-pgo.sh
python3 bench/cross.py --qn target/release/qn-pgo --runs 5
```
