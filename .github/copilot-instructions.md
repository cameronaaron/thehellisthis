# Copilot Instructions

## Architecture Overview
Dual-platform real-time WebSocket chat: native Rust server (Axum 0.7) + Cloudflare Workers+Containers edge proxy. Core is single-server ephemeral chat with multi-room support, no persistence.

### Core Components
- **[src/main.rs](src/main.rs)** (2010 lines): Axum server, WS handlers, room state, rate limiting, memory tracking, IP bans. Contains all server logic.
- **[src/tests.rs](src/tests.rs)**: 387 integration tests covering room validation, rate limits, memory tracking, security. Run with `cargo test`.
- **[index.html](index.html)**: Zero-dependency client. WebSocket connects to `ws(s)://host/ws/{room}`, renders Markdown, handles typing indicators, read receipts. Keep plain JS—no build step.
- **[cloudflare/src/index.ts](cloudflare/src/index.ts)**: Durable Object proxy. Routes all traffic to Rust container at port 3000. Single `main` instance handles all rooms (rooms managed by Rust internally).

## Data Flow
```
Client (index.html) → WS /ws/{room} → Axum Handler → RoomState (broadcast channel) → All room clients
                                  ↓
                            UserData + RateLimiter + MemoryTracker
```

### State Architecture
- **`AppState`**: Global `RwLock<HashMap<String, RoomState>>` + `ResourceMonitor`, `MemoryTracker`, `ConnectionPool`, `SecurityManager`
- **`RoomState`**: Per-room `broadcast::Sender` (capacity 1000), `chat_history: Vec<OutgoingMessage>`, `users: HashMap<Uuid, UserData>`, `available_animals: Vec<String>`, memory counters, timestamps
- **`UserData`**: Tracks `user_id`, `animal_name`, `ConnectionState`, `RateLimiter`, typing state, last read message, last sanitized text

### Message Pipeline
1. Client sends JSON `{type:"Message", text:"..."}`
2. `validate_message` checks length/non-empty
3. Comrak renders Markdown → HTML
4. Ammonia sanitizes HTML (XSS protection)
5. Duplicate check (`SANITIZE_TIMEOUT` 50ms)
6. `OutgoingEvent::Message` broadcast to room
7. History appended (max `MAX_MESSAGES_PER_ROOM` 500)

## Critical Constants (src/main.rs lines 36-71)
- `MAX_MESSAGES_PER_ROOM: 500` — history limit per room
- `MAX_TOTAL_ROOMS_MEMORY: 400MB` — global cap, enforced by `MemoryTracker`
- `MAX_CONCURRENT_CONNECTIONS_PER_IP: 3` — IP-based throttling
- `MAX_MESSAGE_LEN: 8000` — reject longer messages
- `MESSAGE_RATE_LIMIT: 500ms` — min time between messages
- `MAX_MESSAGES_PER_WINDOW: 30` — per 60s per user
- `HEARTBEAT_INTERVAL: 5s`, `HEARTBEAT_TIMEOUT: 6s`
- `ROOM_CLEANUP_INTERVAL: 60s` — check for inactive rooms every minute
- `EMPTY_ROOM_CLEANUP_DELAY: 60s` — delete empty rooms after 1 minute of inactivity
- **When adding features, use existing helpers (`RateLimiter::can_send_message`, `MemoryTracker::can_add_message`) instead of custom checks**

## HTTP/WebSocket Routes
- `GET /` → redirect `/main`
- `GET /main` → bundled HTML (main room)
- `GET /:room` → validate name (3-50 chars, regex `^[a-zA-Z0-9][a-zA-Z0-9-_]*[a-zA-Z0-9]$`, reject reserved: `robots.txt`, `main`, `admin`, `api`, `ws`, `health`, `metrics`)
- `WS /ws/:room` → upgrade after validation
- `GET /robots.txt` → dynamic handler (update both [src/main.rs](src/main.rs) and [robots.txt](robots.txt) if changing)

## Security & Safety Patterns
- **Cookies**: `user_id` + `animal_name` via `create_user_cookies`; HttpOnly, SameSite=Strict, Secure. Extracted via `UserCookie` in handlers.
- **IP Bans**: `SecurityManager` auto-bans after 10 suspicious activities. Check with `security_manager.is_banned(&ip)` before allowing new connections.
- **Connection Pooling**: `ConnectionPool` enforces `MAX_CONCURRENT_CONNECTIONS_PER_IP`. Always call `connection_pool.add_connection(&ip)` on connect, `remove_connection(&ip)` on disconnect.
- **Message Sanitization**: All user text goes through `validate_message` → Comrak → Ammonia. **Never bypass this pipeline**—client receives pre-sanitized HTML.
- **Room Name Validation**: `validate_input` enforces length and regex. Rejects reserved paths.

## Event Schema (JSON over WebSocket)
**Client → Server:**
```json
{"type":"Message", "text":"Hello!"}
{"type":"Typing", "is_typing":true}
{"type":"ReadReceipt", "message_id":"<uuid>"}
```

**Server → Client:**
```json
{"type":"Message", "message":{"message_id":"<uuid>","animal_name":"Lion","text":"<sanitized html>","timestamp":"1234567890"}}
{"type":"System", "event":{"type":"UserJoined","animal_name":"Tiger"}}
{"type":"UserCount", "count":5}
{"type":"Heartbeat"}
{"type":"ReconnectToken", "token":"<reconnect-id>"}
```
**Keep enum names stable**—client depends on exact string values.

## Background Tasks
- **Heartbeat sender**: Every 5s sends `OutgoingEvent::Heartbeat` to all clients
- **Client pinger**: Every 5s sends ping frames, disconnects if no pong within 6s
- **Cleanup**: Every 60s `cleanup_rooms` removes empty rooms (1 minute threshold), prunes old messages, disconnects stale users
- **Memory GC**: Every 60s via `MemoryTracker::cleanup_if_needed`
- **Shutdown handler**: Ctrl-C triggers `graceful_shutdown` → broadcasts `OutgoingEvent::System(ServerShutdown)` to all rooms

## Developer Workflows

### Build & Test
```bash
cargo run                # Debug build, PORT=3000
cargo build --release    # Optimized binary (~7.5 MB)
cargo test               # Run all 397 tests
cargo fmt --all          # Format code
cargo clippy --all-targets --all-features -- -D warnings  # Lint
```

### Pre-Deployment Checklist
- Code passes `cargo fmt --all -- --check`
- Code passes `cargo clippy --all-targets --all-features -- -D warnings`
- All tests pass: `cargo test --all-features`
- Release build succeeds: `cargo build --release`
- Binary size is ~7.5 MB
- GitHub Actions CI/CD configured

### Deployment Targets

**Heroku** (simple, classic PaaS)
```bash
git push heroku main
```
Uses [heroku.yml](heroku.yml) Docker build, runs `infinite-chat` binary from [Procfile](Procfile). Binary name must match Cargo.toml `[package] name`. If changing, update both [Procfile](Procfile) and [heroku.yml](heroku.yml).

**Cloudflare Containers** (edge, WebSocket-native, scales to zero)
```bash
cd cloudflare && npm install && npm run deploy
```
Architecture:
- **Worker**: [cloudflare/src/index.ts](cloudflare/src/index.ts) proxies all traffic to container at port 3000
- **Config**: [cloudflare/wrangler.jsonc](cloudflare/wrangler.jsonc) — sets `max_instances: 5`
- **Container**: [Dockerfile.cloudflare](Dockerfile.cloudflare) builds Rust server image
- **Environment**: Worker passes `PORT=3000` and `RUST_LOG=info` to container
- **Dev mode**: `npm run dev` (builds locally & proxies through Worker)
- **Production**: `npm run deploy` (builds, pushes to registry, deploys Worker+Container)
- **Logs**: `npx wrangler tail` streams real-time logs from production

**Cloudflare Advantages**:
- 300+ edge locations worldwide
- Native WebSocket support
- Scales to zero (pay only for usage)
- Cold starts ~2-5 seconds
- No minimum instances required

### Environment Variables
- `PORT` — Server port (default: `3000`)
- `RUST_LOG` — Tracing level (optional, e.g., `info`, `debug`)

## Extension Patterns

### Adding Features
1. **Reuse state helpers**: `RoomState::assign_animal`, `RoomState::broadcast_user_count`, `RoomState::broadcast_system_event`
2. **Avoid long-held locks**: Never hold `RwLock` write lock across `.await` (follow existing patterns—lock, clone data, drop lock, then await)
3. **Add tests**: Mirror behavior in [src/tests.rs](src/tests.rs) (see `test_room_validation`, `test_rate_limiting`)
4. **Update events**: If adding new `ClientEvent`/`OutgoingEvent` variants, update both server enums and client JS in [index.html](index.html)
5. **Keep timing in sync**: If changing any timing constants (cleanup delays, heartbeat intervals, etc.), run `cargo test test_frontend` to verify frontend HTML still matches. The tests embed index.html and parse it to detect drift.

### Logging
- **Framework**: `tracing` (configured via `tracing_subscriber::fmt::init()`)
- **Use macros**: `info!`, `warn!`, `error!`, `debug!` (already consistent throughout)
- **Metrics**: `metrics` crate present but not wired. To enable: wire `metrics-exporter-prometheus` in `main()` (see [DEPLOYMENT.md](DEPLOYMENT.md#monitoring--observability))

## Known Constraints
- **Ephemeral**: Rooms/messages lost on restart (no DB)
- **Single-server**: No clustering/horizontal scaling
- **Memory-bound**: 400MB cap enforced; exceeding triggers cleanup
- **Max 100 rooms**, **500 messages/room**, **400 concurrent users**

## Docs for Ops/Troubleshooting
- **[DEPLOYMENT.md](DEPLOYMENT.md)**: Pre-deploy checklist, env vars, resource limits, monitoring setup, performance tuning
- **[CLOUDFLARE_DEPLOYMENT.md](CLOUDFLARE_DEPLOYMENT.md)**: Cloudflare-specific setup, scaling behavior, cold start times, cost comparison
- **[README.md](README.md)**: Quick start, features, API reference, architecture diagram

## Resource Limits & Scaling

All limits enforced in [src/main.rs](src/main.rs):
- **Max rooms**: 100 active concurrent
- **Messages per room**: 500 (oldest auto-pruned)
- **Global memory**: 400 MB cap (enforced by `MemoryTracker`)
- **Connections per IP**: 3 (prevents abuse)
- **Message length**: 8,000 bytes max (after sanitization)
- **Rate limit**: 30 messages per 60s per user, min 500ms between messages
- **Heartbeat**: 5s interval, 6s timeout before disconnect
- **Cleanup**: Every 60s for stale connections, every 60s for rooms (1-minute inactivity threshold)

## Monitoring & Observability

**Logs**: Structured via `tracing` crate to stdout
- Macros: `info!`, `warn!`, `error!`, `debug!`
- All major events logged (connections, disconnects, rate limit hits, IP bans, memory events)

**Metrics**: `metrics` crate is integrated but not wired (optional enhancement)
- To enable Prometheus export:
  ```rust
  let prometheus_builder = metrics_exporter_prometheus::PrometheusBuilder::new();
  prometheus_builder.install_recorder().unwrap();
  ```

**Health Checks**: No built-in endpoint—use WebSocket connectivity tests or ping container on `PORT`

**Security Monitoring**:
- Auto-ban IPs after 10 suspicious activities (checked in `SecurityManager`)
- Connection pooling tracks concurrent connections per IP
- Message validation rejects oversized/invalid messages with error events

## Production Security Posture

- **Message Sanitization**: User text → Markdown (Comrak) → HTML → Ammonia sanitization → safe HTML in broadcast
- **Cookie Security**: `user_id` + `animal_name` cookies are HttpOnly, SameSite=Strict, Secure
- **IP Bans**: Automatic after 10 violations; checked before accepting new connections
- **Input Validation**: Room names (regex + length), message sizes (8 KB), character sets
- **Connection Pooling**: Per-IP rate limiting (max 3 concurrent connections)
- **No Persistence**: Ephemeral design prevents data leakage on compromise

## Performance Tuning

### Memory Management
- **Cleanup cycle**: Every 60s triggers `cleanup_rooms()` → removes empty rooms, prunes old messages
- **Global cap**: 400 MB hard limit; exceeding triggers aggressive pruning
- **Per-room**: Max 500 messages; oldest auto-deleted when limit reached
- **User cleanup**: Disconnected users removed from room state

### WebSocket Efficiency
- **Broadcast capacity**: 1000 messages buffered per room (sender channel)
- **Heartbeat**: 5s interval + 6s timeout prevents zombie connections
- **Message parsing**: JSON validation + sanitization on hot path
- **Connection state**: RwLock minimizes contention; locks dropped after snapshot

## Known Constraints

- **Ephemeral**: All rooms/messages lost on restart (no database)
- **Single-server**: No clustering or horizontal scaling
- **Max 100 concurrent rooms**, **500 messages/room**, **~400 concurrent users** (memory-bound)
- **Cold start**: Cloudflare container cold starts ~2-5 seconds

## Optional Future Enhancements

1. Add PostgreSQL for persistence + room recovery on restart
2. Implement cluster support (Redis pub/sub for room coordination)
3. Add WebSocket compression (permessage-deflate)
4. Wire Prometheus metrics exporter + Grafana dashboards
5. Deploy to Kubernetes with health checks & auto-scaling
6. Add room moderation: pin/unpin messages, user mutes
7. Implement rich media (image previews, link embeds)

## Deployment Troubleshooting

### Heroku Issues
- Binary size > 500 MB? Causes slug compilation timeout. Use `cargo build --release` locally, commit binary.
- Port binding fails? Verify `PORT` env var is read in [src/main.rs](src/main.rs) — default is 3000.
- Dyno memory limit? 512 MB dyos can run single room with ~50 users. Use larger dyos for scale.

### Cloudflare Issues
- Container doesn't start? Verify `docker build -f Dockerfile.cloudflare .` succeeds locally.
- WebSocket connection fails? Check Worker is proxying upgrade requests ([cloudflare/src/index.ts](cloudflare/src/index.ts)).
- Cold starts slow? Rust binary takes ~500ms to initialize. Keep at least 1 instance warm with periodic health checks.
- Deploy fails? Run `npm run deploy` from `cloudflare/` directory; ensure `wrangler login` is authenticated.

### General
- High memory usage? Reduce `MAX_TOTAL_ROOMS_MEMORY` in [src/main.rs](src/main.rs) — currently 400 MB.
- Rate limit too strict? Adjust `MESSAGE_RATE_LIMIT` and `MAX_MESSAGES_PER_WINDOW` constants.
- Tests fail post-deploy? Rebuild binary: `cargo build --release`. Some tests embed [index.html](index.html) and validate timing.

## Tone & UX
- **Branding**: "Social art experiment" framing in [index.html](index.html). Keep copy consistent with collaborative/playful tone.
- **Animal names**: Random assignment from `ANIMAL_NAMES` in [src/main.rs](src/main.rs). Client displays these, not user IDs.
