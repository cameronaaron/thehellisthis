# infinite-chat

A **high-performance WebSocket chat server** written in Rust using Axum 0.7. Multi-room support, real-time presence, and built-in memory/rate-limiting for safe concurrent use.

## Quick Start

```bash
# Local development
cargo run

# Release build
cargo build --release
PORT=3000 ./target/release/infinite-chat
```

Open [http://localhost:3000/main](http://localhost:3000/main) to chat.

## Features

✨ **Core**
- Real-time multi-room WebSocket chat
- Markdown rendering with HTML sanitization
- Persistent per-user cookies (UUID + animal name)
- Room discovery via dynamic routing

🛡️ **Reliability**
- Built-in rate limiting (30 msgs/min per user)
- Global memory cap (400 MB) with auto-pruning
- IP-based connection pooling & bans
- Automatic inactive user/room cleanup

🔒 **Security**
- HttpOnly, SameSite=Strict, Secure cookies
- HTML sanitization via Ammonia
- Message size limits (8 KB max)
- Per-IP connection throttling

⚡ **Performance**
- Axum + Tokio async runtime
- Broadcast channel for efficient room fanout
- Zero-copy message pipelines
- ~7.5 MB optimized release binary

## Project Structure

```
src/
  main.rs         — Server: WS handlers, room state, rate limiting
  tests.rs        — Integration tests (21 tests, all passing)
index.html        — Client: plain JS, WebSocket, read receipts, typing indicators
Procfile          — Heroku dyno command
heroku.yml        — Heroku container build config
.github/workflows/ci.yml  — GitHub Actions (fmt/clippy/test/build)
```

## API

### WebSocket Events

**Client → Server**
```json
{ "type": "Message", "text": "Hello!" }
{ "type": "Typing", "is_typing": true }
{ "type": "ReadReceipt", "message_id": "<uuid>" }
```

**Server → Client**
```json
{ "type": "Message", "message": { "message_id": "<uuid>", "animal_name": "Lion", "text": "<sanitized html>", "timestamp": "1234567890" } }
{ "type": "System", "event": { "type": "UserJoined", "animal_name": "Tiger" } }
{ "type": "UserCount", "count": 5 }
{ "type": "Heartbeat" }
{ "type": "ReconnectToken", "token": "<reconnect-id>" }
```

### HTTP Routes

- `GET /` → Redirect to `/main`
- `GET /main` → Main chat room HTML
- `GET /:room` → Dynamic room HTML (validates room name)
- `WS /ws/:room` → WebSocket upgrade

## Configuration

Edit [src/main.rs](src/main.rs) constants:

| Setting | Value | Purpose |
|---------|-------|---------|
| `MAX_MESSAGES_PER_ROOM` | 500 | Chat history limit per room |
| `MAX_TOTAL_ROOMS_MEMORY` | 400 MB | Global memory cap |
| `MAX_CONCURRENT_CONNECTIONS_PER_IP` | 3 | Per-IP connection limit |
| `HEARTBEAT_INTERVAL` | 5s | Keep-alive ping frequency |
| `MAX_MESSAGE_LEN` | 8 KB | Max message size |
| `MESSAGE_RATE_LIMIT` | 500ms | Min time between messages |

## Testing

```bash
cargo test              # Run all 21 tests
cargo test -- --nocapture  # With logging
```

Tests cover:
- Room creation & validation
- Rate limiting & windows
- Memory tracking & concurrency
- Security (IP bans, connection limits)
- Message sanitization
- Cookie handling
- Cleanup logic

## Deployment

### Heroku

```bash
git push heroku main
```

Heroku automatically builds and runs via [heroku.yml](heroku.yml).

### Docker / Self-hosted

```bash
cargo build --release
PORT=3000 ./target/release/infinite-chat
```

See [DEPLOYMENT.md](DEPLOYMENT.md) for detailed setup, monitoring, and performance tuning.

## Architecture

```
┌─────────────────────┐
│   index.html (JS)   │  Client: WebSocket, rendering, read receipts
└──────────┬──────────┘
           │ WS /ws/:room
           ▼
┌─────────────────────────────────────────┐
│ Axum Server (Tokio async)               │
├─────────────────────────────────────────┤
│ HTTP routes: /, /main, /:room, /robots  │
│ WS handler: validate → user/room setup  │
├─────────────────────────────────────────┤
│ AppState                                 │
│  ├─ RwLock<HashMap<RoomState>>          │
│  ├─ MemoryTracker (global cap)          │
│  ├─ ConnectionPool (per-IP limits)      │
│  └─ SecurityManager (IP bans)           │
├─────────────────────────────────────────┤
│ RoomState (per room)                    │
│  ├─ broadcast::Sender<OutgoingEvent>    │
│  ├─ HashMap<UserData>                   │
│  ├─ Vec<OutgoingMessage> (history)      │
│  └─ Resource tracking (memory, users)   │
├─────────────────────────────────────────┤
│ Background Tasks (async)                 │
│  ├─ Room cleanup (hourly)               │
│  ├─ Memory GC (60s intervals)           │
│  └─ Ctrl-C graceful shutdown            │
└─────────────────────────────────────────┘
```

## Hardening Checklist

- [x] Unused code removed, clippy clean
- [x] Memory tracking wired & enforced
- [x] Security activity tracking enabled
- [x] All tests passing
- [x] Release build optimized
- [x] CI/CD pipeline configured
- [x] Heroku deployment aligned
- [x] Cookie security hardened
- [x] Rate limiting enforced
- [x] Message sanitization strict

## License

MIT (or your choice)

## Support

See [DEPLOYMENT.md](DEPLOYMENT.md) for ops & monitoring guidance.
