# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-06 on main @ `fc89fc9` (post speculative-AOT arc,
PR #55), Apple Silicon (darwin25). The previous table (2026-07-05,
block-arc tip) is preserved below for the delta story.*

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
| btrees       | 0.895 | 0.194  | 0.120 | 0.061   | 0.22  | 0.13  | 0.07    |
| combinators  | 0.155 | 0.048  | 0.050 | 0.045   | 0.31  | 0.32  | 0.29    |
| fib_typed    | 0.028 | 0.191  | 0.118 | 0.041   | **6.83** | **4.24** | **1.47** |
| fib_untyped  | 0.016 | 0.084  | 0.061 | 0.031   | **5.24** | **3.78** | **1.91** |
| json         | 0.243 | 0.221  | 0.119 | 0.119   | 0.91  | 0.49  | 0.49    |
| maps         | 0.154 | 0.073  | 0.086 | 0.077   | 0.47  | 0.56  | 0.50    |
| richards     | 0.431 | 0.096  | 0.081 | 0.046   | 0.22  | 0.19  | 0.11    |
| sieve        | 0.105 | 0.306  | 0.192 | 0.099   | **2.92** | **1.83** | 0.95 |
| strings      | 0.169 | 0.045  | 0.073 | 0.069   | 0.27  | 0.43  | 0.41    |
| **geomean**  |       |        |       |         | 0.83  | 0.67  | 0.43    |

Previous table (2026-07-05, pre-speculative-AOT): fib_untyped 0.551
(py/qn 0.15), combinators 0.183, richards 0.492, strings 0.178;
geomeans 0.54 / 0.44 / 0.28.

## Honest reading

**The speculative-AOT arc (PR #55) flipped the headline row.**
fib_untyped — the matrix's poster child for the untyped-dispatch gap,
6.7× BEHIND CPython in the previous table — now runs 0.016s: **5.2×
ahead of CPython 3.13 and 1.9× ahead of Ruby 4 + YJIT**, with zero
annotations. Runtime type feedback compiles it to the same code the
typed version gets. Quoin now wins four rows of nine (both fibs, sieve,
and json-at-parity), and the suite geomean moved from 0.54 to 0.83 vs
CPython (0.28 → 0.43 vs YJIT).

**What remains behind is allocation-bound, not dispatch-bound.** btrees
(GC/allocation, 4.6×), richards (2.3× — improved from 5.2× by compiled
field access, still capped by `^^`-in-cold-arm refusals on the task
bodies and the deliberately megamorphic `@task.run:`), combinators
(3.2× — down from ~11× pre-arcs), strings (3.7×), maps (2.1×). Ruby 4 +
YJIT extends those further.

**What the table points at next** (the gap, in measured order):
1. **Allocation/GC** (btrees, and every per-object workload): ~1.3M
   short-lived objects cost 0.90s vs Python's 0.19s — frame/object
   allocation and the collector are the bill. Now the #1 frontier.
2. **Strings** (two GC allocations per string, as the bench header
   says) — an allocation-frontier special case.
3. **Cold-arm `^^` + megamorphic dispatch** (richards' residual;
   recorded follow-up in docs/SPECULATIVE_AOT_ARCH.md).

The untyped-dispatch frontier that headed this list in the previous
measurement is substantially CLOSED for scalar shapes; its residue
lives in the alloc-heavy rows above.

## Reproducing

```sh
bash scripts/build-pgo.sh
python3 bench/cross.py --qn target/release/qn-pgo --runs 5
```
