# Deployment & Operations Guide

## Pre-Deployment Checklist

- [x] Code passes `cargo fmt --all -- --check`
- [x] Code passes `cargo clippy --all-targets --all-features -- -D warnings`
- [x] All tests pass: `cargo test --all-features`
- [x] Release build succeeds: `cargo build --release`
- [x] Binary size optimized (~7.5 MB)
- [x] GitHub Actions CI/CD configured
- [x] Heroku deployment files aligned (heroku.yml, Procfile)

## Local Development

```bash
# Development build (debug)
cargo run

# Release build (optimized)
cargo build --release
./target/release/infinite-chat

# Tests
cargo test

# Lint & format
cargo fmt --all
cargo clippy --all-targets --all-features
```

## Environment Variables

- `PORT` — Server port (default: `3000`)
- `RUST_LOG` — Tracing level (optional, default: no filter)

## Deployment Targets

### Heroku

Binary name in [Procfile](Procfile) and [heroku.yml](heroku.yml): `infinite-chat`

```bash
git push heroku main
```

### Docker

Release binary is production-ready:
- ~7.5 MB executable
- Minimal attack surface (no runtime dependencies beyond libc)
- Optimized for low-latency WebSocket handling

### Binary Execution

```bash
PORT=3000 ./target/release/infinite-chat
```

## Resource Limits

Configured in [src/main.rs](src/main.rs):

- **Rooms**: Max 100 active
- **Room size**: Max 500 messages (30-day retention)
- **Memory**: 400 MB global cap
- **Connections**: 3 per IP, 100 per room
- **Message size**: 8 KB max (after sanitization)
- **Rate limiting**: 30 messages per 60s per user
- **Heartbeat**: 5s interval, 6s timeout

## Monitoring & Observability

- **Logs**: Structured via `tracing` crate to stdout
- **Metrics**: Prometheus exporter available (metrics crate integrated)
- **Security**: IP-based banning on 10+ suspicious activities

To enable Prometheus metrics export, wire the exporter in `main()`:

```rust
let prometheus_builder = metrics_exporter_prometheus::PrometheusBuilder::new();
prometheus_builder.install_recorder().unwrap();
```

## Security Posture

- **Message sanitization**: Markdown → HTML → Ammonia cleanup
- **Cookie security**: HttpOnly, SameSite=Strict, Secure
- **IP bans**: Automatic after repeated violations
- **Connection pooling**: Per-IP rate limiting
- **Input validation**: Room names and message sizes enforced

## Performance Tuning

### Memory Cleanup

- Runs every 60 seconds to trim inactive users and old messages
- Room cleanup (2-hour inactivity threshold) runs hourly
- Global memory tracker enforces 400 MB ceiling

### WebSocket Throughput

- Heartbeat every 5 seconds (configurable)
- Broadcast channel capacity: 1000 messages per room
- Message parsing: JSON validation + HTML sanitization

## Known Limitations

- Rooms are ephemeral (lost on server restart)
- No persistent storage or clustering
- Single-server deployment only (no horizontal scaling)

## Next Steps

1. **Optional**: Add persistent database (PostgreSQL + sqlx)
2. **Optional**: Implement room persistence & recovery
3. **Optional**: Add WebSocket compression (permessage-deflate)
4. **Optional**: Wire metrics exporter to Prometheus/Grafana
5. **Optional**: Deploy to Kubernetes with health checks
