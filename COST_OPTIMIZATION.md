# Cost Optimization Guide

## Current Setup: Cloudflare (Modified)
- **Workers**: $5/month minimum
- **Containers (1 instance, 256MB, 5m sleep)**: ~$0.50-2/month
- **Total**: ~$5-7/month
- **Pros**: Simple, globally distributed, no cold starts
- **Cons**: Minimum $5/month for Workers

## Alternative: Fly.io Free Tier
- **Cost**: FREE (genuinely)
- **Specs**: 3 shared CPU machines, 256MB each, auto-scaling
- **Pros**: No credit card needed, fully managed, includes DNS
- **Cons**: ~1-2s cold start on first request (not usually noticeable for chat)

## How to Switch to Fly.io (Optional)

1. **Install Fly CLI** (if not already):
   ```bash
   curl -L https://fly.io/install.sh | sh
   ```

2. **Authenticate**:
   ```bash
   fly auth login
   ```

3. **Deploy**:
   ```bash
   fly deploy
   ```

4. **Update DNS** (if using custom domain):
   - Point your domain to Fly.io's IP or use their SSL tunnel

## Recommended: Hybrid Approach (Best of Both)
Keep Cloudflare Workers as public entry point (free tier OK) + proxy to Fly.io backend:
- **Cloudflare Workers**: ~$0 (free tier, just routing)
- **Fly.io**: FREE
- **Total**: FREE for hobby usage

This setup is what you currently have mostly ready! You'd just:
1. Deploy Rust backend to Fly.io: `fly deploy`
2. Update `cloudflare/src/index.ts` to proxy to Fly.io instead of local container
3. Remove Containers from wrangler.jsonc

Would you like me to:
A) Keep Cloudflare Containers (now optimized at ~$5-7/month)?
B) Switch to Fly.io free tier?
C) Hybrid approach (free)?
