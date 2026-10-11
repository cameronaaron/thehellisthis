#!/bin/bash
# Line-coverage gate.
#
# It was 100% with or without the #[ignore]d tests, once the connection tasks
# were made generic over their sink. The slow tests still prove the real
# heartbeat and housekeeping intervals fire.
#
# **Temporarily 84%** (2026-08-06): adding `--workspace` measured
# nova-operator's tests for the first time and exposed nova code the
# instrumented run never reached. Some got tests then; the rest is hard:
#   - nova-operator/src/main.rs: the DKG/FROST ceremony in `main()` needs the
#     generic-sink treatment `run_steady_state` got, plus a hand-built
#     ceremony transcript to drive it.
#   - session/nova_rln.rs: accept/duplicate/slash need a real STARK proof,
#     which only builds under --release (winterfell's debug-assertion false
#     positive), and tarpaulin instruments a debug build.
#   - session/nova_operator.rs: malicious-dealer and complaint paths need a
#     fuller live multi-operator scenario.
# Raise the floor as each gets a test; never treat 84 as normal. A drop below
# the measured number is fixed with a test, or for genuinely unreachable code
# an entry in scripts/coverage-exemptions.toml with its reason (§6).
#
# Exempted paths are excluded here and justified there;
# `coverage_exemptions_are_justified_and_current` fails on an entry without a
# reason or naming a missing path.
#
# Usage: scripts/coverage.sh [minimum-percentage]

set -euo pipefail

MINIMUM="${1:-84}"

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
# --workspace: without it tarpaulin, like `cargo test`, skips nova-operator's
# own tests, which then never count towards the floor: the instrument, not the
# code (§0.5).
cargo tarpaulin \
    --engine llvm \
    --workspace \
    --out Xml --out Stdout \
    --output-dir target/coverage \
    --timeout 300 \
    --exclude-files "src/tests/*" src/main.rs \
    --fail-under "$MINIMUM"
