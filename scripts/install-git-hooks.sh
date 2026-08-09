#!/bin/bash
# Installs a pre-commit hook that runs gitleaks against staged changes —
# catching a secret before it ever reaches a commit, rather than relying on
# CI (scripts/gitleaks.sh) to catch it after. Opt-in, not automatic: nothing
# in this repo runs at clone time, the same reasoning `index.html` being
# `include_str!`'d rather than fetched keeps every build local and explicit.
#
# Usage: scripts/install-git-hooks.sh

set -euo pipefail
cd "$(dirname "$0")/.."

HOOK=".git/hooks/pre-commit"

cat > "$HOOK" <<'HOOK_EOF'
#!/bin/bash
# Installed by scripts/install-git-hooks.sh — scans staged changes for
# secrets before they ever reach a commit. `--staged` is deliberate: this
# checks what is about to be committed, not the whole working tree, so it
# stays fast and only blocks what this commit would actually introduce.
set -euo pipefail

if ! command -v gitleaks > /dev/null 2>&1; then
    echo "pre-commit: gitleaks is not installed (brew install gitleaks) — skipping secret scan" >&2
    exit 0
fi

gitleaks protect --source . --staged --redact -v
HOOK_EOF

chmod +x "$HOOK"
echo "Installed $HOOK"
