#!/bin/bash
# Manual deploy to Cloudflare Workers + Containers.
#
# The normal path is pushing to main, which deploys via
# .github/workflows/deploy.yml only after the CI gate passes. This script is
# for forcing a deploy or testing this specific path, and it runs the same gate
# first — a manual deploy is not an excuse to skip it.

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
