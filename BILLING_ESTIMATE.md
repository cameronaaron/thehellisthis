# Cloudflare Billing Estimate for thehellisthis

## Current Setup (as of Jan 14, 2026)
- **Workers**: 1 Worker (infinite-chat)
- **Containers**: 1 max instance, Rust backend (~30MB binary + runtime overhead)
- **Region**: Global (Cloudflare edge)
- **Deployed**: Jan 14, 2026 at 11:53 PM UTC

---

## Pricing Breakdown

### Workers Plan: $5.00/month minimum
- First 10 million requests/month included
- $0.50 per additional million requests
- **Current usage**: Likely < 100K requests/month (hobby site)
- **Estimated Workers cost**: **$5.00/month** (flat minimum)

### Durable Objects (Container)
**Per-request charges:**
- $0.15 per million requests to Durable Objects
- Estimate: ~500 requests/day = 15K/month = **$0.002/month** (negligible)

**Compute time (main cost):**
- $0.015 per GB-hour of active memory
- Your container: ~256MB (0.25GB) typical Rust app + runtime
- **1 max instance** configured

#### Memory Cost Scenarios:

**Current (low traffic):**
- Idle most of the time, short bursts when someone visits
- ~2 hours/day active = 60 hours/month
- 0.25 GB × 60 hours × $0.015 = **$0.225/month**

**Light hobby use (10-20 visitors/day):**
- Active ~6 hours/day = 180 hours/month
- 0.25 GB × 180 hours × $0.015 = **$0.675/month**

**Moderate use (100 concurrent users, 8 hours/day):**
- Active ~8 hours/day = 240 hours/month
- 0.5 GB × 240 hours × $0.015 = **$1.80/month**

**Popular (24/7 uptime):**
- 720 hours/month continuous
- 0.5 GB × 720 hours × $0.015 = **$5.40/month**

---

## Total Monthly Estimates

| Usage Scenario | Workers | Container | Total |
|---------------|---------|-----------|-------|
| **Current (idle most days)** | $5.00 | $0.23 | **$5.23** |
| **Light hobby (10-20 daily visitors)** | $5.00 | $0.68 | **$5.68** |
| **Moderate (100 concurrent, 8hr/day)** | $5.00 | $1.80 | **$6.80** |
| **Popular (24/7 activity)** | $5.00 | $5.40 | **$10.40** |
| **Viral (1M requests, 24/7)** | $5.50 | $10.80* | **$16.30** |

*Viral scenario assumes higher memory usage (1GB) and sustained load.

---

## Current Bill Estimate (January 2026)

You deployed on **January 14** (mid-month), so pro-rated:
- **Workers**: $5.00 (charged monthly, not pro-rated)
- **Container**: ~$0.10-0.30 (17 days × light usage)
- **Estimated January bill**: **$5.10-5.30**

---

## Future Monthly Bills (February onwards)

Based on your hobby project profile:
- **Most likely**: **$5.25-6.00/month** (low traffic, occasional visitors)
- **Worst case** (if popular): **$10-15/month** (24/7 sustained load)

---

## How to Monitor Actual Usage

Run this command to see real usage (requires API token):
```bash
cd cloudflare
npx wrangler tail --format pretty
```

Or check your [Cloudflare Dashboard](https://dash.cloudflare.com/) → Workers & Pages → Analytics

---

## Cost Optimization Options

### Option 1: Stay on Cloudflare (Current)
- **Cost**: $5-10/month
- **Pros**: Simple, global edge, low latency
- **Action**: None needed, already optimized with 1 instance

### Option 2: Switch to Fly.io Free Tier (FREE)
- **Cost**: $0/month
- **Specs**: 3 shared-CPU VMs, 256MB RAM each
- **Action**: Run `fly deploy` (already configured in fly.toml)
- **Trade-off**: ~1-2s cold start on first request after idle

### Option 3: Hybrid (Cloudflare Worker + Fly.io backend)
- **Cost**: $0/month (Workers free tier + Fly.io free tier)
- **Action**: Modify cloudflare/src/index.ts to proxy to Fly.io
- **Trade-off**: Slightly more complex setup

---

## Recommendation

For a hobby project with current traffic:
1. **Keep current setup**: $5-6/month is reasonable for a globally distributed chat
2. **If cost is still too high**: Switch to Fly.io free tier (genuinely $0/month)
3. **Monitor first bill**: Check actual usage after 30 days

Your January bill will arrive around **February 1st**. Check Cloudflare Dashboard for real-time usage.
