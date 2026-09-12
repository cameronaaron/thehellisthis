#!/bin/bash
# Manual deploy to Cloudflare Workers + Containers.
#
# The normal path is pushing to main: Cloudflare's Git integration builds and
# deploys from the repository on its own. This script is for forcing a deploy
# out of band — a rollback, or testing the container build — and it runs the
# full gate first, because a manual deploy is the one path with nothing else
# checking it.

set -euo pipefail

cd "$(dirname "$0")"

echo "🔎 Running the gate before deploying..."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked

cargo build --release --locked
cargo test --release --all-features --locked nova_rln
scripts/coverage.sh
scripts/smoke-container.sh
cargo audit
scripts/gitleaks.sh

cd cloudflare

echo "📦 Installing worker dependencies..."
pnpm install --frozen-lockfile
pnpm exec tsc --noEmit
pnpm test
pnpm audit --audit-level=high

echo "🚢 Deploying..."
pnpm exec wrangler deploy

echo "✅ Deployment complete — https://thehellisthis.com"
