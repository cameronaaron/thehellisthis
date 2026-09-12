# infinite-chat

An **ephemeral multi-room WebSocket chat server** in Rust, on Axum 0.8. Visit a
URL, get a room. Get an animal name instead of an account. Rooms with nobody
talking in them are deleted. There is no database — all state is in memory and
all of it is meant to disappear.

Live at **<https://thehellisthis.com>**.

## Quick start

```bash
cargo run                    # → http://localhost:3000/main

cargo build --release --locked
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

- Ephemeral rooms vanish 5 minutes after the last person leaves them empty — a
  spark, not a hearth
- Idle users are disconnected after 10 minutes without speaking, so the user
  count means "people actually here"
- `main` can't be deleted, so its history fades instead — patiently, after 30
  minutes of room-wide silence

### Reliability

- Global memory ceiling (150 MB) with pruning that refuses rather than grows
- Per-user rate limiting (30 messages/min, plus a short burst limit)
- Per-IP connection limits and temporary bans for repeated rejected attempts
- Graceful shutdown on SIGINT that announces departures before exiting

### Security

- `HttpOnly`, `SameSite=Strict`, `Secure` identity cookies; the client learns
  who it is from the `Welcome` frame, never from `document.cookie`
- HTML sanitisation applied to rendered output, not to input
- Content-Security-Policy with `script-src 'self'` — no inline script anywhere
- HSTS, `X-Frame-Options: DENY`, `nosniff`, Referrer-Policy, Permissions-Policy
- WebSocket `Origin` checking (upgrades are not covered by the same-origin
  policy, so the server has to enforce it)
- 8 KB message cap, 512 KB frame cap, bounded and sanitised quoted replies

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
  emoji.rs        Reaction emoji roster (the closed set a reaction may be)
  security.rs     Security headers, CSP, WebSocket origin policy
  tests/          Unit, contract and WebSocket integration tests
index.html        The page, compiled into the binary
client.js         The client script, served from /app.js so the CSP can
                  forbid inline script entirely
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
| `GET /metrics` | Prometheus text exposition — requires `Authorization: Bearer $METRICS_TOKEN`, 404s if unset |
| `GET /admin` | dashboard listing every open room as a link — requires HTTP Basic auth against `$ADMIN_TOKEN`, 401s if unset |
| `WS /ws/:room` | WebSocket upgrade |

## Configuration

Every tunable lives in [src/config.rs](src/config.rs) with a comment explaining
its value. The ones worth knowing:

| Setting | Value |
| --- | --- |
| `MAX_ROOMS` | 50 |
| `MAX_USERS_PER_ROOM` | 100 |
| `MAX_CONCURRENT_USERS` | 400 |
| `MAX_MESSAGES_PER_ROOM` | 500 |
| `MAX_TOTAL_ROOMS_MEMORY` | 150 MB |
| `MAX_MESSAGE_LEN` | 8 KB |
| `MAX_CONCURRENT_CONNECTIONS_PER_IP` | 3 |
| `HEARTBEAT_INTERVAL` | 5s |
| `EMPTY_ROOM_CLEANUP_DELAY` | 5 min |
| `USER_IDLE_MESSAGE_TIMEOUT` | 10 min |
| `MAIN_ROOM_FADE_IDLE` | 30 min |

## Testing

```bash
cargo test --workspace --all-features --locked
cargo test --all-features roster # a subset
scripts/coverage.sh              # line-coverage floor (84%; remaining gaps in the script)
cargo mutants                    # mutation testing (manual sweep, slow)
```

Covers room lifecycle, rate limiting, memory accounting and pruning, connection
accounting, IP bans, message sanitisation and XSS attempts, cookie handling,
the frontend/backend constant contract, and full WebSocket integration against a
real server with a real client.

## Development gate

Run the local release checks before committing. CI also checks the Worker,
release-only RLN test, dependency audits and full-history secret scan.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --release --locked
scripts/coverage.sh
```

## Deployment

Cloudflare Git integration deploys pushes to `main` independently of CI.
Require all CI checks through branch protection before merging; a direct push
can deploy a failing commit. To deploy manually after the local gates:

```bash
./deploy.sh
```

The Rust server runs in a Cloudflare Container behind a Worker that proxies both
HTTP and WebSocket traffic. `max_instances` is 1: all state is in process
memory, so a second instance would be a second, separate set of rooms behind the
same URLs.

## Operational setup

`Dockerfile` links to `Dockerfile.cloudflare`, so `docker build -t infinite-chat .`
uses the same non-root production recipe. Run one instance: room state is not
shared, and a restart intentionally loses history and identities.

For the Cloudflare deployment, configure Worker secrets from `cloudflare/`:

```sh
pnpm exec wrangler secret put ADMIN_TOKEN
pnpm exec wrangler secret put METRICS_TOKEN
# Only if operating the separate Nova quorum:
pnpm exec wrangler secret put NOVA_OPERATOR_TOKEN
```

The Worker forwards these secrets to the container. With no configured secret,
the corresponding endpoint refuses access. For a standalone container, supply
them as environment variables. Never commit the values.

Before releasing, run `pnpm --dir cloudflare test`, the Worker typecheck and
audit, `scripts/smoke.sh`, and `scripts/slow-tests.sh`.
`scripts/smoke-container.sh` builds and starts the actual image, checks the
HTTP routes, embedded assets and admin/metrics authentication, and verifies
non-root execution and graceful shutdown. Check
branch protection and perform a Cloudflare smoke test after deployment; local
checks cannot establish those platform settings or live behavior.

## Documentation

- [CLAUDE.md](CLAUDE.md) — architecture, critical constraints, protocol,
  pitfalls
- [ENGINEERING-STANDARDS.md](ENGINEERING-STANDARDS.md) — the rulebook: the
  complexity, lock-discipline, memory, protocol, and blast-radius laws, and why
  each exists

## License

MIT
