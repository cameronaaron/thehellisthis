# Cloudflare Containers Deployment Guide

This guide explains how to deploy the Infinite Chat application to Cloudflare Containers.

## What is Cloudflare Containers?

Cloudflare Containers allows you to run Docker containers at the edge with WebSocket support, making it perfect for real-time applications like this chat server.

## Prerequisites

1. **Cloudflare Account** with Containers enabled (currently in beta)
2. **Node.js** 18+ installed
3. **Docker** installed and running
4. **wrangler CLI** installed globally or via npx

## Project Structure

```
thehellisthis/
├── cloudflare/           # Cloudflare Workers + Containers config
│   ├── src/
│   │   └── index.ts      # Worker that proxies to container
│   ├── package.json
│   ├── tsconfig.json
│   └── wrangler.jsonc    # Wrangler configuration
├── Dockerfile.cloudflare # Container image for Cloudflare
├── src/                  # Rust source code
└── ...
```

## Deployment Steps

### 1. Navigate to the cloudflare directory

```bash
cd cloudflare
```

### 2. Install dependencies

```bash
npm install
```

### 3. Login to Cloudflare

```bash
npx wrangler login
```

### 4. Test locally (optional)

```bash
npm run dev
```

This will:
- Build the Docker container locally
- Start the Cloudflare Worker in dev mode
- Proxy requests to your local container

### 5. Deploy to Cloudflare

```bash
npm run deploy
```

This will:
- Build and push the Docker image to Cloudflare's container registry
- Deploy the Worker that proxies traffic to the container
- Provision the Durable Object for container management

## How It Works

```
User Request → Cloudflare Edge → Worker → Container (Rust Server)
                                   ↓
                            WebSocket Proxy
```

1. **Cloudflare Worker** receives all HTTP/WebSocket requests
2. **Worker proxies** requests to the **Durable Object** container
3. **Container** runs the Rust WebSocket chat server
4. **WebSocket connections** are transparently proxied through

## Configuration Options

### wrangler.jsonc

- `max_instances`: Maximum concurrent container instances (default: 5)
- `sleepAfter`: Idle timeout before container sleeps (default: 30m)

### Environment Variables

The container receives these environment variables:
- `PORT=3000` - Port the Rust server listens on
- `RUST_LOG=info` - Logging level

### Adding Secrets

For sensitive configuration:

```bash
npx wrangler secret put MY_SECRET
```

## Monitoring

View logs in real-time:

```bash
npx wrangler tail
```

Or check the Cloudflare Dashboard under **Workers & Pages** → **Your Worker** → **Logs**.

## Scaling

Cloudflare Containers automatically:
- **Scale up** new instances based on demand
- **Scale to zero** when idle (after `sleepAfter` period)
- **Cold start** in ~2-5 seconds for new instances

## Costs

Cloudflare Containers pricing includes:
- Container runtime (CPU/Memory time)
- Worker invocations
- Data transfer

Check [Cloudflare Pricing](https://www.cloudflare.com/pricing/) for current rates.

## Troubleshooting

### Container not starting

1. Check Docker is running locally for dev mode
2. Verify Dockerfile builds successfully: `docker build -f Dockerfile.cloudflare .`
3. Check wrangler logs: `npx wrangler tail`

### WebSocket connection fails

1. Ensure the Worker is proxying WebSocket upgrade requests
2. Check the container is listening on the correct port (3000)
3. Verify CORS headers if connecting from a different domain

### Slow cold starts

The Rust binary needs to be loaded into a new container. To minimize:
- Keep the Docker image small (using multi-stage builds)
- Consider keeping at least 1 instance warm with periodic health checks

## Comparison with Other Platforms

| Feature | Cloudflare Containers | Fly.io | Heroku |
|---------|----------------------|--------|--------|
| WebSocket Support | ✅ Native | ✅ Native | ✅ Native |
| Scale to Zero | ✅ Yes | ❌ No (min 1) | ✅ Yes (paid) |
| Cold Start | ~2-5s | ~1-2s | ~5-30s |
| Edge Locations | 300+ | 30+ | Limited |
| Custom Domains | ✅ Yes | ✅ Yes | ✅ Yes |

## Additional Resources

- [Cloudflare Containers Documentation](https://developers.cloudflare.com/containers/)
- [Cloudflare Workers Documentation](https://developers.cloudflare.com/workers/)
- [Wrangler CLI Documentation](https://developers.cloudflare.com/workers/wrangler/)
