# Quoin Playground

A static page that runs the real Quoin VM in the browser as WebAssembly. The
interpreter, GC, compiler, formatter, and the full value-level stdlib are the same
code the native `qn` binary runs; OS-bound features (sockets, files, subprocesses,
fibers, workers) raise ordinary catchable errors instead. See
`src/direct_runner.rs` for the execution model (coroutine-less, scheduler-less).

## Build

```sh
scripts/build_wasm.sh        # wasm-pack → playground/pkg (gitignored)
```

Requires `wasm-pack` (`brew install wasm-pack`) and the target
(`rustup target add wasm32-unknown-unknown`).

## Try locally

```sh
python3 -m http.server -d playground 8000   # then http://localhost:8000
```

Any static server works; the page is fully self-contained (no external hosts, no
CDN), so a strict CSP is fine. The one server requirement is the standard
`application/wasm` MIME type for `.wasm` (anything modern, including Cloudflare,
does this).

## Verify

```sh
node scripts/wasm_smoke.mjs            # end-to-end bundle behavior
node scripts/wasm_examples_check.mjs   # every example on the page still runs
```

## How Stop works

The VM runs in a Web Worker (`worker.js`); the page never blocks. Stop is
`worker.terminate()` + booting a fresh VM — which is also what makes infinite
loops harmless. `quoin-wasm`'s `run()` additionally accepts a `maxBatches` cap
(unused by the page today) as a belt-and-suspenders budget.

## Deploy

Build, then publish the `playground/` directory (including the generated `pkg/`)
as static assets — e.g. copy into the site repo or point a Cloudflare static
deployment at it.
