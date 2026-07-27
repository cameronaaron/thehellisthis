#!/bin/bash
# The tests CI does not run.
#
# `#[ignore]`d tests wait on a real interval — a heartbeat tick, a housekeeping
# pass. They are excluded from `cargo test`, and therefore from CI, because a
# shared runner under load is exactly where a time-dependent test becomes a
# flake, and a suite that fails for reasons unrelated to the change stops being
# read (§6.4).
#
# They exist because the branches they cover cannot be reached any other way:
# the alternatives are an injectable clock (rejected, §9.4) or asserting on
# timing (rejected, §6.4). Run them locally before pushing anything that touches
# the heartbeat, the housekeeping loops, or the idle eviction.

set -euo pipefail
cd "$(dirname "$0")/.."

echo "Running the time-dependent tests CI skips..."
cargo test --all-features -- --ignored --test-threads=1

echo
echo "✅ Slow tests passed."
