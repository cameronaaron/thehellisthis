#!/bin/bash
# Deploy script with automatic Docker cache busting for Cloudflare Containers

set -e

echo "🚀 Deploying to Cloudflare with cache bust..."

# Clear Docker builder cache to force fresh build
echo "🧹 Clearing Docker builder cache..."
docker builder prune -af > /dev/null 2>&1 || true

# Navigate to cloudflare directory and deploy
cd "$(dirname "$0")/cloudflare"

echo "📦 Installing dependencies..."
npm ci

echo "🚢 Deploying to Cloudflare Workers + Containers..."
npx wrangler deploy

echo "✅ Deployment complete!"
echo "🌍 Live at: https://infinite-chat.cameronaaron1.workers.dev"
