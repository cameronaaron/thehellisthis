# CLAUDE.md — infinite-chat (thehellisthis.com)

> **Read `ENGINEERING-STANDARDS.md` before any concurrency, memory, protocol,
> or performance work.** It is the authoritative rulebook. It opens with **§0,
> the first-principles doctrine** — reason from ground truth (a measurement, a
> lock held, bytes on the wire), not from convention; treat "that's just how
> it's done" as an unexamined claim; apply the loop *in order* (question the
> requirement → delete → simplify → accelerate → automate). **§0.5 is not
> optional before trusting any performance number:** validate the instrument
> before the measurement. §1 is the complexity doctrine (per-message and
> per-event work is O(1); linear work runs rarely and never under contention),
> §2 the lock-discipline law, §3 the memory-ceiling law, §4 the protocol law,
> §5 the failure-blast-radius law, §6 the regression ratchet, §7 the engagement
> doctrine (why rooms die and history fades — those are the product, not
> bugs), §8 naming and organisation, §9 the shipped-artifact law, §10 the
> client law. Every rule there is backed by a test in `src/tests.rs` — when one
> fails, fix the source, not the test.

## What this is

An **ephemeral multi-room WebSocket chat server**. Visit a URL, get a room. Get
an animal name instead of an account. Rooms with nobody talking in them are
deleted; history in `main` fades. There is **no database** — all state is in
memory and all of it is meant to disappear.

Live at **<https://thehellisthis.com>**, served by a Rust binary running in a
Cloudflare Container behind a Worker.

## Commands

```bash
cargo run                       # dev server → http://localhost:3000/main
cargo test --all-features       # 556 tests; all must pass before committing
cargo fmt --all -- --check      # formatting is a gate, not a preference
cargo clippy --all-targets --all-features -- -D warnings   # warnings are failures
cargo build --release           # LTO'd binary for the container image
scripts/coverage.sh             # line-coverage floor (93%), ratchets up only
```

Those five are the gate, and they are exactly what `.github/workflows/ci.yml`
runs — a green local run means a green CI run.

**Mutation testing** is a manual sweep, not a gate (a full run takes far longer
than a commit should wait):

```bash
cargo mutants                   # everything
cargo mutants -f src/room.rs    # one file, while iterating
```

Coverage says a line ran; mutation testing says something would have noticed if
it were wrong. See ENGINEERING-STANDARDS.md §6.6, including the classification
of the six mutants that currently survive.

## Commits

Small, single-topic commits — one logical change each, readable from
`git log --oneline` without opening diffs. Subject: imperative, ≤72 chars, says
*what*; body says *why*. **A fix and the test that pins it are one commit** (the
ratchet rule); unrelated changes are separate commits. Every commit passes the
four gate commands above.

## Session-end ritual (ENGINEERING-STANDARDS.md §6)

Before ending a session that touched code or made a real decision:

1. **Every behaviour change ships with its test in the same commit.** No "add
   tests later."
2. **Non-obvious wisdom gets written down, not just enacted.** A measurement
   that contradicted the guess, a rejected approach and why it lost, a decision
   with a reopen condition — if a future session hitting the same wall would
   want to have read it first, it belongs in `ENGINEERING-STANDARDS.md` (a
   reusable law) or here (a project fact). Restating a rule already covered is
   itself a §0 violation: a claim with no new ground truth behind it.
3. **A manually-found defect becomes a sweep for its class, same commit** (§6).
   Fixing the instance is not enough — the test suite, not a future human
   audit, must find the next one. Prove the sweep can fail (inject a violation,
   watch it fire, revert) before trusting it.

## Stack

- **Rust 2024 edition** (MSRV 1.85), **Tokio** async runtime
- **Axum 0.8** + `axum-server` — HTTP routes and the WebSocket upgrade
- **comrak** (Markdown) → **ammonia** (HTML sanitisation) message pipeline
- **Vanilla JS client** — `index.html` + `client.js`, both compiled into the
  binary with `include_str!`; no build step, no framework, no bundler. The
  script is a separate file so the CSP can forbid inline script (constraint #13)
- **Cloudflare Workers + Containers** — a Worker proxies to the Rust container
- **No database.** No Redis. No persistence of any kind.

## Architecture

```text
Browser ── WS /ws/:room ──▶ Cloudflare Worker (cloudflare/src/index.ts)
                                     │ proxies to the container
                                     ▼
                        Axum server (src/main.rs)
                        ├─ routes.rs    HTTP: /, /main, /:room, /health, /metrics
                        ├─ session.rs   WS: admission → 4 tasks → teardown
                        ├─ state.rs     AppState: RwLock<HashMap<String, RoomState>>
                        ├─ room.rs      per-room users, history, broadcast channel
                        ├─ limits.rs    rate limits, memory, connection pool, IP bans
                        └─ cleanup.rs   housekeeping loop: fade, reclaim, delete
```

### Module map

Every module has one job and says so in its header comment. Nothing is named
`utils`, `helpers`, `common`, or `misc` (§8).

| Module | Owns |
| --- | --- |
| `config.rs` | **Every tunable.** No magic numbers anywhere else. |
| `animals.rs` | The animal-name roster (sorted, deduped, enforced by test). |
| `protocol.rs` | The wire format. Changing it is a client change too. |
| `error.rs` | `ChatError` → HTTP status mapping. |
| `identity.rs` | The two identity cookies and the extractor for them. |
| `limits.rs` | `RateLimiter`, `ResourceMonitor`, `MemoryTracker`, `ConnectionPool`, `SecurityManager`. |
| `room.rs` | `RoomState`, `UserData`, history trimming, name assignment. |
| `state.rs` | `AppState`, graceful shutdown, periodic resource cleanup. |
| `validation.rs` | Input validation and the Markdown → sanitised-HTML pipeline. |
| `routes.rs` | Plain HTTP handlers. |
| `session.rs` | The WebSocket lifecycle. |
| `cleanup.rs` | The room housekeeping pass. |
| `main.rs` | Entry point, router construction, housekeeping loops. |

`main.rs` is **only** wiring. If you are adding logic to it, it belongs in a
module. `build_router` is factored out so tests exercise the real route table
rather than a hand-assembled copy of it
(`router_mounts_every_public_route`).

### The four per-connection tasks

`session.rs::handle_websocket` races four tasks with `tokio::select!`:

| Task | Job |
| --- | --- |
| `forward_task` | room broadcast → this client |
| `receive_task` | this client → the room |
| `ping_task` | WebSocket-level `Ping` every 5s |
| `heartbeat_task` | app-level `Heartbeat` frame + idle eviction |

They are **raced, not joined**: a dead socket surfaces in exactly one of them,
and the first to notice tears the whole session down. Whichever finishes first,
`cleanup_user` runs exactly once after the `select!`.

## Critical constraints

### 1. Every reservation is released on every exit path

`ws_handler_inner` admits a connection in two phases, and the order is the
whole point:

```rust
// 1. Every check that can fail — mutating nothing.
security_manager.check_ip(ip)?;
connection_pool.can_accept(ip);
resource_monitor.can_accept_connection();
validate_input(&room, MAX_ROOM_NAME_LEN)?;

// 2. Only then, the reservations.
connection_pool.add_connection(ip).await?;
resource_monitor.total_connections.fetch_add(1, SeqCst);

// 3. The one fallible step that must come after: admission under the room
//    write lock. On refusal it calls release_connection_slot explicitly.
```

Room capacity is decided **once**, by `is_user_allowed` under the write lock.
There used to be a second capacity check before the reservations, which made the
release path unreachable — the redundant guard was hiding the code that existed
to handle its own failure.

**This was a real bug.** The reservations used to be taken *first*, with
several checks that could fail running after and returning without releasing.
Every rejected connection permanently consumed a slot, so a server that had
refused enough connections would refuse all of them while completely idle.
Pinned by `rejected_upgrade_releases_its_connection_slot`.

### 2. The client learns its identity from the `Welcome` frame

`Welcome { user_id, animal_name }` is the **first frame** on every connection,
sent before history. The client stores it and uses it to tell its own messages
from everyone else's.

Both identity cookies are `HttpOnly`. The client must **never** read
`document.cookie` — it can't, and a client that tries silently renders every
message the user sends as somebody else's. These are two halves of one
decision; `client_takes_identity_from_welcome_frame_not_cookies` keeps either
half from being reverted alone.

Before this, the client assumed the first `UserJoined` it saw was itself and
otherwise parsed `document.cookie` — which is exactly why the cookies could not
be `HttpOnly`, on a server whose whole job is rendering user-supplied Markdown
to HTML.

### 3. `main` is never deleted — it fades

Every other room is deleted after `EMPTY_ROOM_CLEANUP_DELAY` (10 min) with no
connected users. `main` is the entry point and must always exist, so it fades
instead: idle for `MAIN_ROOM_FADE_IDLE`, its history trims to
`MAIN_ROOM_FADE_KEEP` (50). `cleanup.rs` special-cases `MAIN_ROOM` and
`continue`s before the deletion branch. Do not "simplify" that away.

### 4. Room-name check order is user-facing

`routes.rs::room_handler` reports reserved → length → shape → capacity, in that
order, with those exact strings. Tests assert them, because reordering changes
what a visitor is told about the name they typed.

### 5. Never hold the room lock across an `await` on the socket

`state.rooms` is a single `RwLock<HashMap<..>>` — every user in every room
contends on it. Sending to a socket can block for as long as the slowest client
takes. Read what you need, `drop` the guard, then send. `handle_websocket`
clones `chat_history` under the lock and sends it after, for exactly this
reason.

### 6. Work under the lock is a latency budget, not a micro-optimisation

Per-message work must be O(1). Work proportional to history length
(`retain_newest`, `cleanup_messages`, `recompute_memory`) runs on the join path
or the housekeeping loop, never per message. `HISTORY_TRIM_SLACK` exists purely
so trimming cannot land on the message path.

### 7. `panic = "abort"` is deliberately not set

Tokio confines a panicking task to that task. Under `abort`, one panic in one
connection handler kills the process and disconnects **every user in every
room**. Unwinding here is a blast-radius decision. See the comment in
`Cargo.toml` and §5.

### 8. Sanitise after rendering, never before

`validation.rs::render_message_html` runs `markdown_to_html` **then**
`ammonia::clean`. Sanitising the Markdown first both mangles legitimate syntax
and leaves whatever the renderer subsequently produced unchecked. `ammonia`
runs on the generated HTML because that is the only representation a browser
ever sees.

### 9. The animal roster stays sorted, deduped, and made of animals

`animals.rs` is enforced by `animal_roster_is_sorted_unique_and_well_formed`.
Sorted so a duplicate is visible in review; deduped because two connected users
with the same name breaks the only identity the UI shows.

An earlier revision ran alphabetically off the end of a dictionary and shipped
`hadron`, `hagiology`, `halley`, `gyroscope`, `half-penny` and `hallway` as
assignable names, with `hallingers` and `hallway` present twice. The test now
prevents the whole class — and caught a sorting slip in the replacement list on
its first run.

### 10. `config.rs` holds every tunable

No magic numbers in logic modules. Each constant carries the reason it has its
value, because several are deliberate **game mechanics** rather than technical
limits — see §7 and constraint #3.

### 11. The client is compiled into the binary

`routes.rs` does `include_str!("../index.html")`. The container image then has
exactly one artifact that can be out of date with itself, and serving the page
costs no syscall. It also means **editing `index.html` requires a rebuild** — a
running `cargo run` will not pick it up.

### 12. Frontend and backend constants must agree

`index.html` tells users when rooms disappear; `config.rs` decides when they
actually do. A set of `EMBEDDED_HTML`-based tests asserts the user-facing text
matches the backend constant. If the UI says "ten minutes" and the constant
says sixty seconds, the UI is lying — and that has happened.

### 13. The client script is external so the CSP can forbid inline script

`index.html` loads `/app.js`; neither contains an inline `<script>` block or an
`onclick=` handler. That is what lets the policy be `script-src 'self'` with no
`'unsafe-inline'` — the capability an injected `<script>` needs, on a server
whose job is rendering user Markdown to HTML.

`/app.js` is served with a content-hash version (`?v=…`), `immutable` caching,
and an ETag; the page is rewritten once at startup to reference the stamped URL.
Adding an inline script or an inline handler breaks the policy, and
`client_script_is_external_so_csp_can_forbid_inline` fails.

### 14. Client identity is the `Welcome` frame; the client never counts

Two client rules that were each a real bug:

- **Never read `document.cookie`** (constraint #2). Identity comes from the
  `Welcome` frame.
- **Never keep a running count of rendered messages.** The old `messageCount`
  was incremented for system messages, which remove themselves after 8s without
  decrementing it. The count drifted above the real number of nodes and started
  deleting live chat messages. `pruneRenderedMessages()` counts the DOM instead.

### 15. The client owns the scroll position, and does it instantly

`#chat` is `scroll-behavior: auto` and `overflow-anchor: none`. Pinning to the
bottom is instant and coalesced into one animation frame; smooth scrolling is
opt-in per call (`scrollToMessage`). Two flags suppress false "the user scrolled
up" readings: `programmaticScroll` (the client's own writes) and
`isLoadingHistory` (the replay, which ends at the `ReconnectToken` frame, with a
5s failsafe).

This is what fixed the chat visibly shifting on reload. See
ENGINEERING-STANDARDS.md §10.2.

### 16. Anything that appears only sometimes is an overlay

`#chat:empty` must not restyle the container. It used to set
`justify-content: center`, so the first history message flipped the whole column
from centred to top-aligned — a reorientation on every load. The placeholder is
an absolutely-positioned `::before`/`::after` overlay (§10.3).

### 17. The real client IP is `CF-Connecting-IP`

Cloudflare sets it; the Worker forwards it and also fills `X-Forwarded-For`.
The server checks `CF-Connecting-IP` first. Reading only `X-Forwarded-For`, as
it used to, meant every visitor arrived as the same address — turning the
per-IP connection limit into a global limit of three.

## Protocol

### Client → Server

```json
{ "type": "Message", "text": "hello", "reply_to": { "message_id": "...", "author_name": "...", "preview_text": "..." } }
{ "type": "Typing", "is_typing": true }
{ "type": "ReadReceipt", "message_id": "<uuid>" }
```

### Server → Client

```json
{ "type": "Welcome", "user_id": "<uuid>", "animal_name": "otter" }
{ "type": "Message", "message": { "message_id": "<uuid>", "user_id": "...", "animal_name": "otter", "text": "<sanitised html>", "timestamp": "<ms>" } }
{ "type": "System", "event": { "UserJoined": { "user_id": "...", "animal_name": "otter" } } }
{ "type": "UserCount", "count": 5 }
{ "type": "Heartbeat" }
{ "type": "ReconnectToken", "token": "<uuid>" }
```

Frame order on connect is **guaranteed**: `Welcome`, then history, then
`ReconnectToken`, then live traffic. Constraint #2 depends on it.

## HTTP routes

| Route | Purpose |
| --- | --- |
| `GET /` | 308 → `/main` |
| `GET /main` | the client |
| `GET /:room` | the client, if the room name is valid and there is capacity |
| `GET /health` | liveness probe — the container runtime polls this |
| `GET /metrics` | Prometheus text exposition, seven gauges |
| `GET /robots.txt` | disallows `/ws/` |
| `WS /ws/:room` | the chat itself |

`/{room}` is registered **last**; every reserved path is in
`config::RESERVED_ROOM_NAMES`.

## Tests

`src/tests.rs` — 556 tests, one file, run with `cargo test --all-features`.
Notable classes:

| Class | Pins |
| --- | --- |
| Roster integrity | roster sorted/deduped/well-formed; names never collide |
| Router wiring | every public route is mounted (`build_router`) |
| Connection accounting | rejected upgrades leak no slots |
| Identity contract | `Welcome` frame + `HttpOnly` cookies + no `document.cookie` |
| Frontend/backend consistency | UI text matches `config.rs` constants |
| WebSocket integration | real server, real `tokio-tungstenite` client |
| Security | XSS through the Markdown pipeline, IP bans, rate limits |
| Memory | tracker accounting, pruning, underflow protection |

Run a subset: `cargo test --all-features roster`

## Deployment

**Pushing to `main` deploys**, via `.github/workflows/deploy.yml` — which
**waits for CI to pass first**. The Worker and container deploy together.

```bash
./deploy.sh                            # manual: clear Docker cache, npm ci, deploy
cd cloudflare && npx wrangler deploy   # what deploy.sh ultimately runs
```

The container image builds from `Dockerfile.cloudflare`, referenced from
`cloudflare/wrangler.jsonc` as `../Dockerfile.cloudflare` so the build context
is the repo root. `max_instances: 1` is a cost decision **and** a correctness
one — the server holds all state in memory, so a second instance would be a
second, separate set of rooms with the same URLs.

Cloudflare stops containers with **SIGINT**, which is why `main.rs` handles
`ctrl_c` and `AppState::shutdown` announces departures before exiting: a
routine scale-down becomes "user left" rather than a silent socket drop.

## Common pitfalls

- Editing `index.html` and expecting a running `cargo run` to serve it — it is
  `include_str!`'d, so rebuild (constraint #11).
- Holding the room write lock across a socket send (constraint #5).
- Adding a constant inline instead of to `config.rs` (constraint #10).
- Adding a `ChatError` variant nothing constructs — the taxonomy is for
  failures the server actually produces. Three such variants were deleted.
- Changing `protocol.rs` without changing `index.html` in the same commit.
- Assuming per-IP limits are real protection — `X-Forwarded-For` is
  attacker-controlled when the container is reached directly. The
  per-connection and global ceilings are the real bound.
