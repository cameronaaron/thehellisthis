#!/bin/bash
# Line-coverage gate.
#
# The floor here is **97%**, and scripts/coverage-full.sh's is **98%**. The
# difference is exactly the `#[ignore]`d time-dependent tests, which CI does not
# run: they cover the idle eviction and the housekeeping loops, and measuring
# without them and demanding their coverage anyway would be asking CI to prove
# something it was not allowed to check.
#
# Neither floor is If coverage drops the fix is a test, or — for code that
# genuinely cannot be exercised — an entry in scripts/coverage-exemptions.toml
# with a reason attached. Never a smaller number (ENGINEERING-STANDARDS.md §6).
#
# The exempted paths are excluded from measurement here and justified there;
# `coverage_exemptions_are_justified_and_current` fails if an entry loses its
# reason or names a path that no longer exists.
#
# Usage: scripts/coverage.sh [minimum-percentage]

set -euo pipefail

MINIMUM="${1:-97}"

cd "$(dirname "$0")/.."

if ! command -v cargo-tarpaulin > /dev/null 2>&1; then
    echo "cargo-tarpaulin is not installed. Install it with:"
    echo "    cargo install cargo-tarpaulin --locked"
    exit 1
fi

echo "Measuring coverage (minimum ${MINIMUM}%)..."

# --engine llvm: the ptrace engine cannot follow the threads the async tests
# spawn, and silently under-reports them.
# Kept in step with scripts/coverage-exemptions.toml, which carries the reasons.
cargo tarpaulin \
    --engine llvm \
    --out Xml --out Stdout \
    --output-dir target/coverage \
    --timeout 300 \
    --exclude-files src/tests.rs src/main.rs \
    --fail-under "$MINIMUM"
