#!/bin/bash
# Secret scan over all of git history: a leak deleted in the next commit stays
# in history, so scanning HEAD alone misses the case this is for.
#
# .gitleaks.toml (picked up automatically) extends the default rules and
# allowlists two confirmed-harmless matches, each with its reason there.
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
