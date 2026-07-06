# Cross-language results: Quoin vs Python vs Ruby

*Measured 2026-07-05 on the block-template-AOT branch tip (post-B3b,
combinators 2.99× arc complete), Apple Silicon (darwin25).*

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

## Results (seconds, median of 5; ratios = other/quoin, >1 ⇒ Quoin faster)

| bench        | quoin | python | ruby  | rb-yjit | py/qn | rb/qn | yjit/qn |
|--------------|------:|-------:|------:|--------:|------:|------:|--------:|
| btrees       | 0.883 | 0.193  | 0.120 | 0.060   | 0.22  | 0.14  | 0.07    |
| combinators  | 0.183 | 0.048  | 0.050 | 0.046   | 0.26  | 0.27  | 0.25    |
| fib_typed    | 0.028 | 0.190  | 0.119 | 0.041   | **6.77** | **4.24** | **1.45** |
| fib_untyped  | 0.551 | 0.082  | 0.059 | 0.030   | 0.15  | 0.11  | 0.05    |
| json         | 0.236 | 0.218  | 0.117 | 0.117   | 0.92  | 0.50  | 0.50    |
| maps         | 0.146 | 0.070  | 0.084 | 0.076   | 0.48  | 0.58  | 0.52    |
| richards     | 0.492 | 0.095  | 0.080 | 0.045   | 0.19  | 0.16  | 0.09    |
| sieve        | 0.102 | 0.304  | 0.190 | 0.098   | **2.97** | **1.85** | 0.95 |
| strings      | 0.178 | 0.045  | 0.071 | 0.068   | 0.25  | 0.40  | 0.38    |
| **geomean**  |       |        |       |         | 0.54  | 0.44  | 0.28    |

## Honest reading

**Where Quoin wins is exactly where its guarantees engage.** fib_typed
(typed params + sealed class → AOT-compiled) beats CPython 3.13 by 6.8×,
plain Ruby by 4.2×, and YJIT by 1.45×. sieve (typed + checked generics →
compiled) beats Python 3× and plain Ruby 1.9×, and matches YJIT. json is
at parity with Python's C-accelerated stdlib (Quoin's parser is pure).

**Everywhere else, 2026-era interpreters are ahead.** The same benchmarks
untyped or allocation-bound run 2–7× slower than CPython 3.13:
fib_untyped (full dynamic dispatch) 6.7×, btrees (GC/allocation) 4.6×,
richards (megamorphic sends + accessors) 5.2×, combinators 3.9×, strings
4×, maps 2.1×. Ruby 4 with YJIT extends all of those further. Note the
combinators number *is* the block-template-AOT arc's 3× win — before the
arc it would have been ~11× behind Python rather than 3.9×.

Older "4.5–6.8× faster than Python" parity claims in the project notes
were measured against earlier CPython on a narrower typed-path suite and
do not describe this matrix; this table supersedes them.

**What the table points at next** (the gap, in measured order):
1. **Untyped dispatch** (fib_untyped, richards): every `+`/send through
   the IC still costs ~7× CPython's specialized bytecode. This is the
   register-VM / broader-AOT-candidacy frontier.
2. **Allocation/GC** (btrees, and every per-object workload): ~1.3M
   short-lived objects cost 0.88s vs Python's 0.19s — frame/object
   allocation and the collector are the bill.
3. **Strings** (two GC allocations per string, as the bench header says).

## Reproducing

```sh
bash scripts/build-pgo.sh
python3 bench/cross.py --qn target/release/qn-pgo --runs 5
```
