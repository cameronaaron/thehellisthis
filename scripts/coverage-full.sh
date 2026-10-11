#!/bin/bash
# Line coverage including the #[ignore]d time-dependent tests CI skips, so it
# reaches what only happens after an interval (idle eviction, housekeeping).
# scripts/coverage.sh is the CI gate.
#
# Its floor should sit at or above coverage.sh's and only move up. Both are
# temporarily 84% (2026-08-06); coverage.sh's comment lists the nova gaps.
# Raise both together.
#
# Usage: scripts/coverage-full.sh [minimum-percentage]

set -euo pipefail

MINIMUM="${1:-84}"

cd "$(dirname "$0")/.."

if ! command -v cargo-tarpaulin > /dev/null 2>&1; then
    echo "cargo-tarpaulin is not installed. Install it with:"
    echo "    cargo install cargo-tarpaulin --locked"
    exit 1
fi

echo "Measuring coverage including ignored tests (minimum ${MINIMUM}%)..."

cargo tarpaulin \
    --engine llvm \
    --workspace \
    --out Xml --out Stdout \
    --output-dir target/coverage-full \
    --timeout 300 \
    --exclude-files "src/tests/*" src/main.rs \
    --fail-under "$MINIMUM" \
    -- --include-ignored
