#!/bin/bash
# Line coverage including the tests CI does not run.
#
# `scripts/coverage.sh` is the CI gate and measures what CI actually runs. This
# one adds the `#[ignore]`d time-dependent tests, so it reaches the branches
# that only happen after an interval elapses — the idle eviction, the
# housekeeping loops.
#
# The floor here is separate and higher, because the extra tests are exactly the
# ones that cover the otherwise-unreachable code. It only ever moves up.
#
# Usage: scripts/coverage-full.sh [minimum-percentage]

set -euo pipefail

MINIMUM="${1:-100}"

cd "$(dirname "$0")/.."

if ! command -v cargo-tarpaulin > /dev/null 2>&1; then
    echo "cargo-tarpaulin is not installed. Install it with:"
    echo "    cargo install cargo-tarpaulin --locked"
    exit 1
fi

echo "Measuring coverage including ignored tests (minimum ${MINIMUM}%)..."

cargo tarpaulin \
    --engine llvm \
    --out Xml --out Stdout \
    --output-dir target/coverage-full \
    --timeout 300 \
    --exclude-files "src/tests/*" src/main.rs \
    --fail-under "$MINIMUM" \
    -- --include-ignored
