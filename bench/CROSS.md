# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-06 on `perf/ic-direct-calls` @ `80a209b` (main
post #59/#60 plus the outcall-seam arc F1-D2, pre-merge), Apple
Silicon (darwin25). The previous table (same day, the materialization
arc tip `5af70cc`) is preserved below for the delta story.*
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
| btrees       | 0.320 | 0.195  | 0.120 | 0.061   | 0.61  | 0.37  | 0.19    |
| combinators  | 0.126 | 0.047  | 0.049 | 0.046   | 0.38  | 0.39  | 0.36    |
| fib_typed    | 0.028 | 0.191  | 0.117 | 0.040   | **6.79** | **4.16** | **1.42** |
| fib_untyped  | 0.016 | 0.082  | 0.059 | 0.030   | **5.08** | **3.67** | **1.87** |
| json         | 0.231 | 0.217  | 0.116 | 0.116   | 0.94  | 0.50  | 0.50    |
| maps         | 0.138 | 0.070  | 0.084 | 0.075   | 0.51  | 0.60  | 0.54    |
| richards     | 0.314 | 0.094  | 0.079 | 0.045   | 0.30  | 0.25  | 0.14    |
| sieve        | 0.101 | 0.301  | 0.187 | 0.097   | **2.97** | **1.85** | 0.96 |
| strings      | 0.077 | 0.043  | 0.071 | 0.067   | 0.57  | **0.92** | 0.88 |
| **geomean**  |       |        |       |         | **1.07** | 0.87  | 0.57 |

Previous table (2026-07-06, materialization-arc tip `5af70cc`): btrees
0.361 (py/qn 0.55), richards 0.371 (0.27), combinators 0.128, maps
0.141, json 0.239; geomeans **1.05** / 0.85 / 0.55.

## Honest reading

**The outcall-seam arc (F1-D2) moved the two seam-bound rows again**:
btrees 0.361 → 0.320 and richards 0.371 → 0.314 (−11/−15%), from the
per-site AOT IC dispatching warm compiled→compiled calls straight to
the entry, plus entry-nil deferred locals compiling `sum`/`reduce:`
(the suite's last coverage refusal). Suite geomean vs CPython edges
1.05 → **1.07**; vs Ruby 0.87; vs YJIT 0.57. maps improved to 0.138 —
its +3% on the local profiling build was static code layout, exactly
as the same-binary shim test said, and PGO washes it.

Quoin holds four rows outright (both fibs, sieve, json-at-parity) and
the geomean vs CPython. What remains behind, in measured order:

1. **The irreducible outcall shell** (btrees 1.6×, richards 3.3×
   behind CPython): window push + preconditions + invoke +
   `run_in_frame_ctx` per call even on an AOT-IC hit. The recorded
   next step is D3 (docs/OUTCALL_ARCH.md): guarded direct inner calls
   baked into the caller's native code — needs
   re-translation-on-warm-IC machinery.
2. **Combinator pipelines** (2.6×): per-element `valueWithSelfOrArg:`
   block-call seams — the same shell in block clothing.
3. **maps/json/strings** (≈2×/parity/1.7×): interpreter residue and
   C-library string ops; diminishing returns.

The dispatch and allocation frontiers that headed this list across the
last three arcs are substantially closed: untyped dispatch (S-arcs),
alloc/GC (alloc-churn), materialization (M-arcs), and now the warm
outcall lookup itself (D2).

## Reproducing

```sh
bash scripts/build-pgo.sh
python3 bench/cross.py --qn target/release/qn-pgo --runs 5
```
