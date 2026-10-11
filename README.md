# infinite-chat

An ephemeral multi-room WebSocket chat server in Rust (Axum 0.8). Visit a URL,
get a room and an animal name instead of an account. Rooms nobody is in are
deleted, and there is no database: all state is in memory and meant to
disappear.

Live at <https://thehellisthis.com>.

## Quick start

```bash
cargo run                    # http://localhost:3000/main

cargo build --release --locked
PORT=3000 ./target/release/infinite-chat
```

## Features

- Multi-room chat over WebSockets; a room exists once someone visits it
- Markdown rendered and then sanitised (comrak, then ammonia)
- Anonymous identity: a UUID and an animal name in `HttpOnly` cookies
- Replies, reactions, image attachments, typing indicators, read receipts
- The `nova` room: a live demo of [novachannel](https://github.com/cameronaaron/novachannel)'s
  post-quantum channel, RLN anonymous rate-limited posting and an MPC/FROST
  quorum. The channel is between the browser and this server, not end to end
  between browsers, and the server still renders messages.

### Deliberate mechanics

- A spawned room is deleted 5 minutes after the last person leaves.
- Idle users are disconnected after 10 minutes without speaking, so the count
  means people actually here.
- `main` can't be deleted; after 30 minutes of silence its history fades to
  the newest 75 messages.

### Limits

- A 150 MB global memory ceiling; messages are refused rather than letting it
  grow
- 30 messages a minute per user, plus a short burst limit
- 3 connections per address, and temporary bans after repeated rejections
- 8,000-character messages, bounded rendered output and quoted replies
- Graceful shutdown on SIGINT that announces departures first

### Security

- `HttpOnly; SameSite=Strict; Secure` identity cookies; the client learns who
  it is from the `Welcome` frame, never `document.cookie`
- A Content-Security-Policy with `script-src 'self'` and no third-party
  origins; HSTS, `X-Frame-Options: DENY`, `nosniff`, Referrer-Policy and
  Permissions-Policy
- WebSocket `Origin` checking (upgrades aren't covered by the same-origin
  policy)
- Client addresses are kept only as per-process keyed hashes

## Layout

```text
src/
  main.rs           module declarations and main
  startup.rs        router, housekeeping loops, bind, shutdown
  config.rs         every tunable, with its reasoning
  protocol.rs       the WebSocket wire format
  session/          admission, the four connection tasks, teardown, nova
  room/             room state, history, names, presence, reactions
  limits/           rate limits, memory, connections, bans
  routes/           HTTP handlers
  security/         headers, CSP, origin policy, endpoint tokens
  validation/       text, attachments, client addresses
  tests/            unit, contract and WebSocket integration tests
index.html          the page, compiled into the binary
client.js           served as /app.js, so the CSP can forbid inline script
nova-wasm/          browser bindings for the nova room (built package committed)
nova-operator/      a quorum member for the nova room's MPC demo
cloudflare/         the Worker in front of the container
```

## API

### WebSocket client to server

```json
{ "type": "Message", "text": "hello" }
{ "type": "Typing", "is_typing": true }
{ "type": "ReadReceipt", "message_id": "<uuid>" }
{ "type": "React", "message_id": "<uuid>", "emoji": "🔥" }
{ "type": "RequestRoster" }
```

### WebSocket server to client

`Welcome` is always first, then history, then `ReconnectToken`.

```json
{ "type": "Welcome", "user_id": "<uuid>", "animal_name": "otter" }
{ "type": "Message", "message": { "message_id": "<uuid>", "user_id": "...", "animal_name": "otter", "text": "<sanitised html>", "timestamp": "<ms>" } }
{ "type": "System", "event": { "UserJoined": { "user_id": "...", "animal_name": "otter" } } }
{ "type": "UserCount", "count": 5 }
{ "type": "Heartbeat" }
{ "type": "ReconnectToken", "token": "<uuid>" }
```

[AGENTS.md](AGENTS.md#protocol) has the full set.

### HTTP

| Route | Purpose |
| --- | --- |
| `GET /` | 308 to `/main` |
| `GET /main` | the main room |
| `GET /:room` | any other room |
| `GET /health` | liveness probe |
| `GET /metrics` | Prometheus text; needs `Authorization: Bearer $METRICS_TOKEN`, 404 if unset |
| `GET /admin` | every open room; HTTP Basic against `$ADMIN_TOKEN`, 401 if unset |
| `WS /ws/:room` | WebSocket upgrade |

## Configuration

Every tunable is in [src/config.rs](src/config.rs) with its reasoning:

| Setting | Value |
| --- | --- |
| `MAX_ROOMS` | 50 |
| `MAX_USERS_PER_ROOM` | 100 |
| `MAX_CONCURRENT_USERS` | 400 |
| `MAX_MESSAGES_PER_ROOM` | 500 |
| `MAX_TOTAL_ROOMS_MEMORY` | 150 MB |
| `MAX_MESSAGE_LEN` | 8,000 characters |
| `MAX_CONCURRENT_CONNECTIONS_PER_IP` | 3 |
| `HEARTBEAT_INTERVAL` | 5s |
| `EMPTY_ROOM_CLEANUP_DELAY` | 5 min |
| `USER_IDLE_MESSAGE_TIMEOUT` | 10 min |
| `MAIN_ROOM_FADE_IDLE` | 30 min |

## Development

The gate, run by CI and expected to pass before each commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features --locked
cargo build --release --locked
cargo test --release --all-features --locked nova_rln   # real STARK proofs
scripts/coverage.sh
```

CI also runs the container smoke test, the Worker's typecheck, tests and
audit, `cargo audit`, and a full-history secret scan. Before a release, also
run `scripts/smoke.sh` and `scripts/slow-tests.sh`, which depend on real time
and stay out of CI. `cargo mutants` is a manual mutation-testing sweep.

## Deployment

Cloudflare's Git integration deploys every push to `main`, independently of
CI, so require CI through branch protection and merge by pull request.
`./deploy.sh` runs the whole gate and then deploys, for out-of-band releases.

The server runs in a Cloudflare Container behind a Worker that proxies HTTP
and WebSocket traffic. `max_instances` is 1: state is in memory, so a second
instance would be a separate set of rooms behind the same URLs.

`docker build -t infinite-chat .` uses the same non-root recipe as
production. A restart loses all history and identities, by design.

Configure the Worker's secrets from `cloudflare/`:

```sh
pnpm exec wrangler secret put ADMIN_TOKEN
pnpm exec wrangler secret put METRICS_TOKEN
pnpm exec wrangler secret put NOVA_OPERATOR_TOKEN   # only with a nova quorum
```

The Worker forwards them to the container; an unset secret closes its
endpoint. A standalone container takes them as environment variables.

## Documentation

- [AGENTS.md](AGENTS.md): architecture, critical constraints, protocol,
  pitfalls
- [ENGINEERING-STANDARDS.md](ENGINEERING-STANDARDS.md): the rules (complexity,
  locks, memory, protocol, blast radius) and why each exists

## License

[AGPL-3.0-or-later](LICENSE). If you run a modified version as a network
service, AGPL section 13 requires offering its users the source; the header's
source link is how this deployment does that.
