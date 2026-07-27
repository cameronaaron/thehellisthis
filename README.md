# infinite-chat

An **ephemeral multi-room WebSocket chat server** in Rust, on Axum 0.8. Visit a
URL, get a room. Get an animal name instead of an account. Rooms with nobody
talking in them are deleted. There is no database — all state is in memory and
all of it is meant to disappear.

Live at **<https://thehellisthis.com>**.

## Quick start

```bash
cargo run                    # → http://localhost:3000/main

cargo build --release        # LTO'd, stripped: 5.4 MB
PORT=3000 ./target/release/infinite-chat
```

## Features

### Core

- Real-time multi-room WebSocket chat
- Markdown rendering with HTML sanitisation (comrak → ammonia)
- Anonymous identity: a UUID and an animal name, in `HttpOnly` cookies
- Rooms created by visiting them; reply threads, typing indicators, read receipts

### Game mechanics

Deliberate product decisions, not resource management.

- Ephemeral rooms vanish after 10 minutes with nobody connected
- Idle users are disconnected after 10 minutes without speaking, so the user
  count means "people actually here"
- `main` can't be deleted, so its history fades instead

### Reliability

- Global memory ceiling (400 MB) with pruning that refuses rather than grows
- Per-user rate limiting (30 messages/min, plus a short burst limit)
- Per-IP connection limits and temporary bans for repeated rejected attempts
- Graceful shutdown on SIGINT that announces departures before exiting

### Security

- `HttpOnly`, `SameSite=Strict`, `Secure` identity cookies; the client learns
  who it is from the `Welcome` frame, never from `document.cookie`
- HTML sanitisation applied to rendered output, not to input
- 8 KB message cap, 512 KB frame cap

## Project structure

```text
src/
  main.rs         Entry point, router, housekeeping loops — wiring only
  config.rs       Every tunable, with the reasoning attached
  animals.rs      The animal-name roster (sorted and deduped, enforced by test)
  protocol.rs     The WebSocket wire format
  error.rs        ChatError → HTTP status
  identity.rs     Identity cookies and their extractor
  limits.rs       Rate limits, memory tracking, connection pool, IP bans
  room.rs         RoomState, UserData, history trimming, name assignment
  state.rs        AppState, shutdown, periodic cleanup
  validation.rs   Input validation, Markdown → sanitised HTML
  routes.rs       HTTP handlers
  session.rs      WebSocket lifecycle: admission, four tasks, teardown
  cleanup.rs      Room housekeeping pass
  tests.rs        510 tests
index.html        The client: vanilla JS, compiled into the binary
cloudflare/       Worker + Container deployment
```

## API

### WebSocket

#### Client → server

```json
{ "type": "Message", "text": "hello" }
{ "type": "Typing", "is_typing": true }
{ "type": "ReadReceipt", "message_id": "<uuid>" }
```

#### Server → client

`Welcome` is always the first frame.

```json
{ "type": "Welcome", "user_id": "<uuid>", "animal_name": "otter" }
{ "type": "Message", "message": { "message_id": "<uuid>", "user_id": "...", "animal_name": "otter", "text": "<sanitised html>", "timestamp": "<ms>" } }
{ "type": "System", "event": { "UserJoined": { "user_id": "...", "animal_name": "otter" } } }
{ "type": "UserCount", "count": 5 }
{ "type": "Heartbeat" }
{ "type": "ReconnectToken", "token": "<uuid>" }
```

### HTTP

| Route | Purpose |
| --- | --- |
| `GET /` | 308 → `/main` |
| `GET /main` | main room |
| `GET /:room` | dynamic room |
| `GET /health` | liveness probe |
| `GET /metrics` | Prometheus text exposition |
| `WS /ws/:room` | WebSocket upgrade |

## Configuration

Every tunable lives in [src/config.rs](src/config.rs) with a comment explaining
its value. The ones worth knowing:

| Setting | Value |
| --- | --- |
| `MAX_ROOMS` | 100 |
| `MAX_USERS_PER_ROOM` | 100 |
| `MAX_CONCURRENT_USERS` | 400 |
| `MAX_MESSAGES_PER_ROOM` | 500 |
| `MAX_TOTAL_ROOMS_MEMORY` | 400 MB |
| `MAX_MESSAGE_LEN` | 8 KB |
| `MAX_CONCURRENT_CONNECTIONS_PER_IP` | 3 |
| `HEARTBEAT_INTERVAL` | 5s |
| `EMPTY_ROOM_CLEANUP_DELAY` | 10 min |
| `USER_IDLE_MESSAGE_TIMEOUT` | 10 min |

## Testing

```bash
cargo test --all-features        # 510 tests
cargo test --all-features roster # a subset
```

Covers room lifecycle, rate limiting, memory accounting and pruning, connection
accounting, IP bans, message sanitisation and XSS attempts, cookie handling,
the frontend/backend constant contract, and full WebSocket integration against a
real server with a real client.

## Development gate

These four commands are what CI runs. All must pass before committing.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

## Deployment

Pushing to `main` deploys, after CI passes
([.github/workflows/deploy.yml](.github/workflows/deploy.yml)). To deploy
manually:

```bash
./deploy.sh
```

The Rust server runs in a Cloudflare Container behind a Worker that proxies both
HTTP and WebSocket traffic. `max_instances` is 1: all state is in process
memory, so a second instance would be a second, separate set of rooms behind the
same URLs.

## Documentation

- [CLAUDE.md](CLAUDE.md) — architecture, critical constraints, protocol,
  pitfalls
- [ENGINEERING-STANDARDS.md](ENGINEERING-STANDARDS.md) — the rulebook: the
  complexity, lock-discipline, memory, protocol, and blast-radius laws, and why
  each exists

## License

MIT
