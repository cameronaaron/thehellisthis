#!/bin/bash
# Line-coverage gate.
#
# **100%**, and the same with or without the `#[ignore]`d tests — making the
# connection tasks and `run_session` generic over their sink moved every branch
# that used to need a real socket into an ordinary test. The slow tests still
# earn their place: they are the only thing proving the *real* heartbeat and
# housekeeping intervals fire, rather than that the code inside them works.
#
# The floor is If coverage drops the fix is a test, or — for code that
# genuinely cannot be exercised — an entry in scripts/coverage-exemptions.toml
# with a reason attached. Never a smaller number (ENGINEERING-STANDARDS.md §6).
#
# The exempted paths are excluded from measurement here and justified there;
# `coverage_exemptions_are_justified_and_current` fails if an entry loses its
# reason or names a path that no longer exists.
#
# Usage: scripts/coverage.sh [minimum-percentage]

set -euo pipefail

MINIMUM="${1:-100}"

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
    --exclude-files "src/tests/*" src/main.rs \
    --fail-under "$MINIMUM"
