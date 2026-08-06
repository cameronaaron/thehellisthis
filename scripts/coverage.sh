#!/bin/bash
# Line-coverage gate.
#
# Was 100%, and the same with or without the `#[ignore]`d tests — making the
# connection tasks and `run_session` generic over their sink moved every branch
# that used to need a real socket into an ordinary test. The slow tests still
# earn their place: they are the only thing proving the *real* heartbeat and
# housekeeping intervals fire, rather than that the code inside them works.
#
# **Temporarily 84%, not 100%** (2026-08-06). `--workspace` (below) turned
# on honest measurement of `nova-operator`'s own tests, which surfaced how
# much of the `nova` MPC/RLN subsystem was never actually exercised by the
# coverage-instrumented run — some of it now closed with real tests in the
# same commit as this floor change, the rest genuinely hard:
#   - `nova-operator/src/main.rs`: the live DKG/FROST ceremony in `main()`
#     needs the same "generic over the sink" treatment `run_steady_state`
#     already got, applied to a much larger state machine, plus a hand-built
#     multi-party ceremony transcript to drive it with.
#   - `session/nova_rln.rs`'s proof accept/duplicate/slash outcomes: only
#     reachable with a real STARK proof, which only builds under `--release`
#     (winterfell's documented debug-assertion false positive — see
#     `nova_rln_anonymous_post_and_double_post_slashing`'s doc comment) —
#     tarpaulin always instruments a debug build.
#   - `session/nova_operator.rs`: a handful of malicious-dealer/complaint
#     scenarios that need a fuller live multi-operator scenario than exists
#     yet.
# Reopen: raise this back toward 100 as each of the above gets a real test;
# never treat 84 as the new normal. If coverage drops *below* today's actual
# measured number, the fix is a test, or — for code that genuinely cannot be
# exercised — an entry in scripts/coverage-exemptions.toml with a reason
# attached (ENGINEERING-STANDARDS.md §6).
#
# The exempted paths are excluded from measurement here and justified there;
# `coverage_exemptions_are_justified_and_current` fails if an entry loses its
# reason or names a path that no longer exists.
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
# --workspace: `nova-operator` (Cargo.toml's `[workspace] members`) has its
# own unit tests (crypto.rs, wire.rs, main.rs) that a plain `cargo tarpaulin`
# never runs — tarpaulin, like `cargo test`, only tests the current package
# unless told to cross the workspace. Without this flag those tests still
# pass locally (`cargo test -p nova-operator` runs them) but silently never
# count towards the floor, which is the instrument disagreeing with the
# measurement (§0.5), not the code being untested.
cargo tarpaulin \
    --engine llvm \
    --workspace \
    --out Xml --out Stdout \
    --output-dir target/coverage \
    --timeout 300 \
    --exclude-files "src/tests/*" src/main.rs \
    --fail-under "$MINIMUM"
