#!/bin/bash
# Line-coverage gate.
#
# The floor only ever moves up (ENGINEERING-STANDARDS.md §6): raise it when a
# change earns the headroom, never lower it to make a commit pass. If coverage
# drops, the fix is a test, not a smaller number.
#
# Usage: scripts/coverage.sh [minimum-percentage]

set -euo pipefail

MINIMUM="${1:-94}"

cd "$(dirname "$0")/.."

if ! command -v cargo-tarpaulin > /dev/null 2>&1; then
    echo "cargo-tarpaulin is not installed. Install it with:"
    echo "    cargo install cargo-tarpaulin --locked"
    exit 1
fi

echo "Measuring coverage (minimum ${MINIMUM}%)..."

# --engine llvm: the ptrace engine cannot follow the threads the async tests
# spawn, and silently under-reports them.
cargo tarpaulin \
    --engine llvm \
    --out Xml --out Stdout \
    --output-dir target/coverage \
    --timeout 300 \
    --fail-under "$MINIMUM"
