#!/usr/bin/env bash
# Profile-guided-optimization build for qn.
#
# Three phases: build an instrumented binary, train it on the benchmark suite
# (bench/qn/*.qn), then rebuild with the merged profile. Produces
# target/release/qn-pgo; the normal target/release/qn is left untouched
# (the PGO phases build into their own target dir).
#
# Requires the llvm-tools rustup component:  rustup component add llvm-tools
#
# Usage:  scripts/build-pgo.sh
set -euo pipefail

cd "$(dirname "$0")/.."

PGO_DIR="$(pwd)/target/pgo-profiles"
BUILD_DIR="target/pgo-build"

HOST=$(rustc -vV | sed -n 's/^host: //p')
PROFDATA="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-profdata"
if [[ ! -x "$PROFDATA" ]]; then
    echo "error: llvm-profdata not found at $PROFDATA" >&2
    echo "install it with:  rustup component add llvm-tools" >&2
    exit 1
fi

rm -rf "$PGO_DIR"
mkdir -p "$PGO_DIR"

echo "== phase 1/3: instrumented build"
RUSTFLAGS="-Cprofile-generate=$PGO_DIR" \
    cargo build --release --target-dir "$BUILD_DIR"

echo "== phase 2/3: training on bench/qn/*.qn"
for f in bench/qn/*.qn; do
    echo "   $f"
    "./$BUILD_DIR/release/qn" "$f" > /dev/null
done

"$PROFDATA" merge -o "$PGO_DIR/merged.profdata" "$PGO_DIR"

echo "== phase 3/3: optimized build"
RUSTFLAGS="-Cprofile-use=$PGO_DIR/merged.profdata" \
    cargo build --release --target-dir "$BUILD_DIR"

cp "$BUILD_DIR/release/qn" target/release/qn-pgo
echo "done: target/release/qn-pgo"
echo "compare against the plain release build with:"
echo "  python3 bench/run.py --compare target/release/qn-pgo"
