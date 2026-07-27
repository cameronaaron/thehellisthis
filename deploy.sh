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
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features

# Cloudflare builds the image from Dockerfile.cloudflare with the repo root as
# its context. Clearing the builder cache avoids shipping a stale layer.
echo "🧹 Clearing Docker builder cache..."
docker builder prune -af > /dev/null 2>&1 || true

cd cloudflare

echo "📦 Installing worker dependencies..."
pnpm install --frozen-lockfile

echo "🚢 Deploying..."
pnpm exec wrangler deploy

echo "✅ Deployment complete — https://thehellisthis.com"
