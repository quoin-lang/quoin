# Project instructions

## Performance work: always profile before & after

For any performance change, capture before/after profiles and keep all artifacts in
`profiling/<short-task-name>/` (e.g. `profiling/unboxed-integers/`).

1. Build fresh: `cargo build --profile profiling` (never profile a stale or debug binary).
2. Record the baseline:
   `samply record --save-only --no-open -o profiling/<task>/before.json.gz target/profiling/qn benchmark`
   then immediately `cp target/profiling/qn profiling/<task>/qn-before`.
3. Make the change, rebuild, and record `after.json.gz` the same way; copy the binary to `qn-after`.
4. Write `profiling/<task>/notes.md`: hypothesis, the commit SHA (or "dirty") each binary was
   built from, headline before→after numbers, and the conclusion.

Why keep the binary: samply profiles store only addresses and symbolicate lazily from the
binary, so the matching binary is required to re-symbolicate a saved profile later — always
keep it. `[profile.profiling]` sets `debug = true`, so DWARF is inline in the binary and no
separate symbol file is needed. `profiling/` is gitignored (local artifact store), except
`profiling/analyze.py` which is tracked.

Analyze a saved profile with `python3 profiling/analyze.py <profile.json.gz>` — it auto-finds
the matching `qn-before`/`qn-after` and symbolicates (via `atos` + `rustfilt`). Add
`--compare after.json.gz` for the module-level before→after deltas.
