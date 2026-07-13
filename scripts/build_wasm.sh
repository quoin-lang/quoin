#!/usr/bin/env bash
# Build the playground wasm bundle: crates/quoin-wasm -> playground/pkg (gitignored).
# The bundle is --target web (ES module + .wasm); the same output runs in the browser
# and under Node (scripts/wasm_smoke.mjs). Requires wasm-pack (brew install wasm-pack)
# and the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown).
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo "error: wasm-pack not found — brew install wasm-pack" >&2
    exit 1
fi

wasm-pack build crates/quoin-wasm \
    --target web \
    --release \
    --out-dir ../../playground/pkg

ls -lh playground/pkg/quoin_wasm_bg.wasm
