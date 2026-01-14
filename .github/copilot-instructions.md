# Copilot Instructions

## Architecture Overview
Dual-platform real-time WebSocket chat: native Rust server (Axum 0.7) + Cloudflare Workers+Containers edge proxy. Core is single-server ephemeral chat with multi-room support, no persistence.

### Core Components
- **[src/main.rs](src/main.rs)** (2010 lines): Axum server, WS handlers, room state, rate limiting, memory tracking, IP bans. Contains all server logic.
- **[src/tests.rs](src/tests.rs)**: 21 integration tests covering room validation, rate limits, memory tracking, security. Run with `cargo test`.
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
- `ROOM_CLEANUP_INTERVAL: 3600s` — prune inactive rooms
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
- **Cleanup**: Hourly `cleanup_rooms` removes inactive rooms (2hr threshold), prunes old messages (30d), disconnects stale users
- **Memory GC**: Every 60s via `MemoryTracker::cleanup_if_needed`
- **Shutdown handler**: Ctrl-C triggers `graceful_shutdown` → broadcasts `OutgoingEvent::System(ServerShutdown)` to all rooms

## Developer Workflows

### Build & Test
```bash
cargo run                # Debug build, PORT=3000
cargo build --release    # Optimized binary (~7.5 MB)
cargo test               # Run all 21 tests
cargo fmt --all          # Format code
cargo clippy --all-targets --all-features -- -D warnings  # Lint
```

### Deployment
- **Heroku**: `git push heroku main` (uses [heroku.yml](heroku.yml) Docker build, runs `infinite-chat` binary from [Procfile](Procfile))
- **Cloudflare**: `cd cloudflare && npm install && npm run deploy` (builds [Dockerfile.cloudflare](Dockerfile.cloudflare), deploys Worker+Container)
- **Binary name**: `infinite-chat` (from Cargo.toml). If changing, update [Procfile](Procfile) and [heroku.yml](heroku.yml).

### Cloudflare-Specific
- **Worker file**: [cloudflare/src/index.ts](cloudflare/src/index.ts) — proxies to container on port 3000
- **Config**: [cloudflare/wrangler.jsonc](cloudflare/wrangler.jsonc) — `max_instances: 5`, `sleepAfter: 30m`
- **Commands**: `npm run dev` (local), `npm run deploy` (production), `npx wrangler tail` (logs)
- **Container env**: `PORT=3000`, `RUST_LOG=info` set in [index.ts](cloudflare/src/index.ts)

## Extension Patterns

### Adding Features
1. **Reuse state helpers**: `RoomState::assign_animal`, `RoomState::broadcast_user_count`, `RoomState::broadcast_system_event`
2. **Avoid long-held locks**: Never hold `RwLock` write lock across `.await` (follow existing patterns—lock, clone data, drop lock, then await)
3. **Add tests**: Mirror behavior in [src/tests.rs](src/tests.rs) (see `test_room_validation`, `test_rate_limiting`)
4. **Update events**: If adding new `ClientEvent`/`OutgoingEvent` variants, update both server enums and client JS in [index.html](index.html)

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

## Tone & UX
- **Branding**: "Social art experiment" framing in [index.html](index.html). Keep copy consistent with collaborative/playful tone.
- **Animal names**: Random assignment from `ANIMAL_NAMES` in [src/main.rs](src/main.rs). Client displays these, not user IDs.
