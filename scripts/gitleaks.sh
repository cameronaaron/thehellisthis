#!/bin/bash
# Secret scan over the full git history, not just the working tree — a leak
# that gets deleted in the next commit is still in history forever unless
# something rewrites it out, so scanning HEAD alone would miss exactly the
# case this exists to catch.
#
# Config: .gitleaks.toml (repo root, picked up automatically) extends
# gitleaks' default rule set and allowlists two confirmed-harmless matches —
# RFC 6455's own example WebSocket handshake key (used verbatim in tests)
# and the literal string "ChaCha20-Poly1305" (an algorithm name, not a
# secret) — both documented with their reasoning in that file, the same
# "an exclusion without a reason is a leak being hidden" rule
# scripts/coverage-exemptions.toml already follows.
#
# Usage: scripts/gitleaks.sh

set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v gitleaks > /dev/null 2>&1; then
    echo "gitleaks is not installed. Install it with:"
    echo "    brew install gitleaks"
    exit 1
fi

gitleaks detect --source . --log-opts="--all" --redact -v
