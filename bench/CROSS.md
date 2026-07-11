# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-10 on `perf/startup-tax` @ `106644a` (the
block-speculation arc plus the map/set small-collection tier), Apple
Silicon (darwin25). The previous table (2026-07-06, the ic-direct-calls
tip `80a209b`) is preserved below for the delta story.*
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
| btrees         | 0.296 | 0.200  | 0.124 | 0.062   | 0.68  | 0.42  | 0.21    |
| combinators    | 0.097 | 0.048  | 0.050 | 0.046   | 0.50  | 0.52  | 0.47    |
| fib_typed      | 0.035 | 0.197  | 0.122 | 0.041   | **5.64** | **3.50** | **1.18** |
| fib_untyped    | 0.021 | 0.086  | 0.062 | 0.031   | **4.12** | **2.98** | **1.47** |
| fib_untyped_32 | 0.035 | 0.197  | 0.123 | 0.041   | **5.68** | **3.54** | **1.18** |
| json           | 0.259 | 0.226  | 0.121 | 0.121   | 0.87  | 0.47  | 0.47    |
| maps           | 0.131 | 0.073  | 0.086 | 0.078   | 0.56  | 0.66  | 0.60    |
| richards       | 0.317 | 0.098  | 0.082 | 0.046   | 0.31  | 0.26  | 0.14    |
| sieve          | 0.104 | 0.309  | 0.193 | 0.100   | **2.96** | **1.85** | 0.96 |
| strings        | 0.080 | 0.045  | 0.074 | 0.069   | 0.57  | **0.93** | 0.87 |
| **geomean**    |       |        |       |         | **1.27** | **1.01**  | 0.61 |

**Geomeans are not comparable across tables with different row sets**: the
`fib_untyped_32` row (Quoin-favorable) inflates the 10-row py/qn geomean.
On the previous nine-bench set this run works out to **~1.08** — the number
comparable to the 07-06 table's 1.07 — and rb/qn crossed parity (**1.01**;
Quoin now ties plain Ruby on the suite geomean).

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

The 07-06 "drift" is now fully accounted (era binaries rebuilt at
`80a209b` and measured in the same build flavor):

1. **json's +21% was real** — bisected to PR #71 (map-any-keys), whose
   validation checked `maps.qn` steady-state but never ran json. Fixed on
   this branch: a small-collection linear tier for Map and Set (≤16 entries
   scan the cached hashes; the index builds on crossing and drops on
   shrinking), plus `vm.new_map` taking ordered pairs directly instead of a
   throwaway SipHash IndexMap every native constructor built and tore down.
   json 0.296 → 0.259, maps 0.131 (now ahead of the 07-06 0.138). The ~4%
   json residue vs 07-06 is the any-key design's inherent per-key Gc string
   allocation — a semantic feature. The pre-index reference runs the
   20k-membership guard workload in 17.6 SECONDS, so the index's purpose is
   untouched; the tier costs it +2-4% in profiling builds.
2. **Startup grew +2.2ms** (8.5 → 10.7ms AOT-on, profiling flavor) — and it
   is organic: each new auto-loaded `qnlib/core` file adds parse + compile
   + execute to every startup (the bisect's largest single step was
   11-plan.qn's 216 lines). Composition at 0.1ms sampling: ~26% eager
   Cranelift compilation of annotated stdlib candidates at load, ~13% pest
   parse, ~11% bytecode compile, the rest prelude execution. The recorded
   open: tier annotated stdlib methods lazily like speculative methods and
   block templates already are — needs a design pass on sibling
   direct-call GROUP compilation before it can land.
3. Everything else was **PGO-flavor variance**: in matched profiling-flavor
   A/B, richards/strings/maps/btrees all improved 80a209b → main.

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
