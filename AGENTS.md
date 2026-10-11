# Working on infinite-chat (thehellisthis.com)

Read `ENGINEERING-STANDARDS.md` before any concurrency, memory, protocol or
performance work; it is the rulebook. §0 is the doctrine (reason from a
measurement, a lock held or bytes on the wire, not convention; §0.5:
validate the instrument before trusting a number). Then: §1 complexity
(per-message work is O(1)), §2 locks, §3 memory ceilings, §4 protocol, §5
blast radius (§5.6 privacy, §5.7 no third-party assets), §6 the regression
ratchet, §7 engagement (rooms dying and history fading are the product),
§8 naming, §9 the shipped artifact, §10 the client. Each rule is enforced by
a test in `src/tests/`; when one fails, fix the source, not the test.

## What this is

An ephemeral multi-room WebSocket chat. Visit a URL, get a room and an animal
name instead of an account. Rooms nobody talks in are deleted; history in
`main` fades. There is no database: all state is in memory and meant to
disappear.

Live at <https://thehellisthis.com>: a Rust binary in a Cloudflare Container
behind a Worker.

## Commands

```bash
cargo run                       # dev server at http://localhost:3000/main
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features --locked
cargo build --release --locked
cargo test --release --all-features --locked nova_rln   # STARK round trip, release only
scripts/coverage.sh             # line-coverage floor (84%; the script says what's left)
scripts/smoke-container.sh      # the release binary in its container image
scripts/gitleaks.sh             # secret scan over all of history
```

Those are what `.github/workflows/ci.yml` runs, so a green local run is a
green CI run. `scripts/install-git-hooks.sh` opts into a pre-commit
`gitleaks protect --staged`.

CI deliberately skips three that depend on real elapsed time, where a loaded
shared runner turns a test into a flake. Run them before pushing anything
touching the heartbeat, housekeeping, admission, identity or teardown:

```bash
scripts/smoke.sh                # drive the release binary, then read its log
scripts/slow-tests.sh           # the #[ignore]d time-dependent tests
scripts/coverage-full.sh        # coverage including them
```

`smoke.sh` starts the binary, behaves like a few browsers, and reads the log
as an operator would, failing on identity churn, silent refusals, panics,
leaked slots and unmatched arrivals. It once passed while matching nothing,
so it now fails when its checks find no lines (§6.4).

`scripts/client-inventory.sh` diffs what the shipped client is made of against
`scripts/client-inventory.toml`, so deleting a piece of it fails the gate
(§6.10).

Mutation testing is a manual sweep, not a gate:

```bash
cargo mutants                       # everything (about two hours)
cargo mutants -f src/room/mod.rs    # one file, minutes
```

Coverage says a line ran; mutation testing says something would notice if it
were wrong (§6.6). `MUTATION-SURVIVORS.md` is the work queue.

## Commits

Small, single-topic commits with an imperative subject of at most 72
characters saying what; the body says why. A fix and the test that pins it
are one commit. Every commit passes the gate.

Before ending a session that changed code or made a decision:

1. Every behaviour change ships with its test in the same commit.
2. Write down what a future session would want to have read first (a
   measurement that contradicted the guess, a rejected approach, a decision
   and its reopen condition): reusable laws in `ENGINEERING-STANDARDS.md`,
   project facts here. Restating a covered rule adds nothing (§0).
3. A defect found by hand becomes a sweep for its class in the same commit,
   and the sweep is shown to fail (inject a violation, watch it fire,
   revert) before it is trusted (§6).

## Stack

- Rust 2024 (MSRV 1.88), Tokio, Axum 0.8 with `axum-server`
- comrak (Markdown) then ammonia (sanitising) for messages
- Vanilla JS client: `index.html` and `client.js`, compiled into the binary
  with `include_str!`; no build step or framework. The script is a separate
  file so the CSP can forbid inline script (constraint #13).
- Cloudflare Workers and Containers; no database, Redis or persistence.
- The `nova` room adds novachannel's post-quantum channel, RLN anonymous
  posting and an MPC/FROST demo (`src/session/nova*.rs`, `nova-wasm/`,
  `nova-operator/`). Their trust model is in those modules' docs: the channel
  is to this server, not end to end between browsers.

## Architecture

```text
Browser -- WS /ws/:room --> Cloudflare Worker (cloudflare/src/index.ts)
                                    | proxies to the container
                                    v
                       Axum server (src/startup.rs)
                       |- routes/      HTTP: /, /main, /:room, /health, /metrics, /admin
                       |- session/     WS: admission, four tasks, teardown, nova
                       |- state.rs     AppState: RwLock<HashMap<String, RoomState>>
                       |- room/        users, history, names, reactions
                       |- limits/      rate limits, memory, connections, bans
                       `- cleanup.rs   housekeeping: fade, reclaim, delete
```

### Module map

Every module has one job, stated in its header comment; nothing is named
`utils`, `helpers`, `common` or `misc` (§8).

| Module | Owns |
| --- | --- |
| `config.rs` | Every tunable. No magic numbers elsewhere. |
| `animals.rs` | The animal-name roster (sorted, deduped, tested). |
| `emoji.rs` | The reaction emoji roster: the closed set a reaction may be. |
| `protocol.rs` | The wire format. Changing it changes the client too. |
| `error.rs` | `ChatError` to HTTP status. |
| `identity.rs` | The two identity cookies and their extractor. |
| `limits/` | `RateLimiter`, `ResourceMonitor`, `MemoryTracker`, `ConnectionPool`, `SecurityManager`. |
| `room/` | `RoomState`, `UserData`, history trimming, name assignment, presence, reactions. |
| `state.rs` | `AppState`, graceful shutdown, periodic resource cleanup. |
| `validation/` | Input validation, client addresses, attachments, Markdown to sanitised HTML. |
| `routes/` | Plain HTTP handlers. |
| `security/` | Response headers, the CSP, the WebSocket origin check, admin and metrics tokens. |
| `session/` | The WebSocket lifecycle, and the nova room. |
| `cleanup.rs` | The room housekeeping pass. |
| `startup.rs` | Router, housekeeping loops, bind, shutdown sequence. |
| `main.rs` | The entry point and nothing else. |

`main.rs` only declares modules and calls `startup::main_inner`, which is
what makes its coverage exemption honest (§6.1c). `build_router` is separate
so tests exercise the real route table (`router_mounts_every_public_route`).

### The four per-connection tasks

`session/lifecycle.rs` races four tasks with `tokio::select!`:

| Task | Job |
| --- | --- |
| `forward_task` | room broadcast to this client |
| `receive_task` | this client to the room |
| `ping_task` | WebSocket `Ping` every 5s |
| `heartbeat_task` | app-level `Heartbeat` and idle eviction |

They are raced, not joined: a dead socket shows up in one of them, and the
first to notice ends the session. `handle_websocket` calls `run_session` (the
tasks and every early exit) and then `cleanup_user` unconditionally. That is
the only teardown call site, so no exit, including one added later, can skip
releasing the connection; it used to be each `return`'s job and one forgot
(§5.11).

## Critical constraints

### 1. Every reservation is released on every exit path

`ws_handler_inner` admits in phases, and the order is the point:

```rust
// 1. Every check that can fail, mutating nothing.
security_manager.check_ip(ip)?;
connection_pool.can_accept(ip);
resource_monitor.can_accept_connection();
validate_input(&room, MAX_ROOM_NAME_LEN)?;

// 2. Only then, the reservations.
connection_pool.add_connection(ip).await?;
resource_monitor.total_connections.fetch_add(1, SeqCst);

// 3. Admission under the room write lock, which on refusal calls
//    release_connection_slot itself.
```

Room capacity is decided once, by `is_user_allowed` under the write lock. A
second capacity check before the reservations used to make the release path
unreachable.

This was a real bug: reservations were taken first and several later checks
returned without releasing, so a server that had refused enough connections
refused all of them while idle. Pinned by
`rejected_upgrade_releases_its_connection_slot`.

### 2. The client learns its identity from the `Welcome` frame

`Welcome { user_id, animal_name }` is the first frame on every connection,
before history; the client uses it to tell its own messages apart. Both
identity cookies are `HttpOnly`, so the client must never read
`document.cookie` (it would silently render every own message as someone
else's). `client_takes_identity_from_welcome_frame_not_cookies` keeps either
half from being reverted alone. The old client parsed `document.cookie`,
which is why the cookies couldn't be `HttpOnly` on a server that renders user
Markdown to HTML.

### 3. `main` is never deleted; it fades

Other rooms are deleted after `EMPTY_ROOM_CLEANUP_DELAY` (5 minutes) with
nobody connected: a spawned room is a spark, not a hearth (§7.4). `main` must
always exist, so after `MAIN_ROOM_FADE_IDLE` (30 minutes) idle its history
trims to `MAIN_ROOM_FADE_KEEP` (75). `cleanup.rs` special-cases `MAIN_ROOM`
before the deletion branch; don't simplify that away.

### 4. Room-name check order is user-facing

`routes/page.rs::room_handler` reports reserved, then length, then shape,
then capacity, with those exact strings. Tests assert them, because the order
decides what a visitor is told about the name they typed.

### 5. Never hold the room lock across an `await` on the socket

`state.rooms` is one `RwLock<HashMap<..>>` that every user contends on, and a
socket send can block as long as the slowest client. Read what you need, drop
the guard, then send.

### 6. Work under the lock is a latency budget

Per-message work is O(1). Work proportional to history (`retain_newest`,
`cleanup_messages`, `recompute_memory`) runs on the join path or in
housekeeping, never per message; that placement is the guarantee (§1.5).

### 7. `panic = "abort"` is deliberately not set

Tokio confines a panic to its task. Under `abort`, one panic in one handler
would disconnect every user in every room (§5; see `Cargo.toml`).

### 8. Sanitise after rendering, never before

`validation/text.rs::render_message_html` runs `markdown_to_html` and then
`ammonia::clean`. Sanitising Markdown first mangles legitimate syntax and
leaves the renderer's output unchecked; the generated HTML is what a browser
sees.

### 9. The animal roster stays sorted, deduped and made of animals

Enforced by `animal_roster_is_sorted_unique_and_well_formed`. An earlier list
ran off the end of a dictionary and shipped `hadron`, `gyroscope` and
`hallway` (twice) as names. Two connected users with one name break the only
identity the UI shows.

### 10. `config.rs` holds every tunable

Each constant carries its reason, because several are game mechanics rather
than technical limits (§7, constraint #3).

### 11. The client is compiled into the binary

`routes/page.rs` embeds `index.html` and `client.js` with `include_str!`: one
artifact that can't be out of date with itself, served without a syscall.
Editing either needs a rebuild; a running `cargo run` won't pick it up.

### 12. Frontend and backend constants must agree

`index.html` tells users when rooms disappear; `config.rs` decides it. Tests
over the embedded page assert the text matches the constants
(`frontend_parity.rs::PARITY`). A UI saying "ten minutes" over a sixty-second
constant has happened.

### 13. The client script is external so the CSP can forbid inline script

Neither `index.html` nor `/app.js` has an inline `<script>` or `on*=`
handler, so the policy is `script-src 'self'` without `'unsafe-inline'`.
`/app.js` is served under a content hash (`?v=`) with `immutable` caching and
an ETag. `client_script_is_external_so_csp_can_forbid_inline` fails on an
inline script.

### 14. Identity comes from `Welcome`; the client never counts

Never read `document.cookie` (constraint #2), and never keep a running count
of rendered messages: the old `messageCount` counted system messages that
remove themselves, drifted high and deleted live messages.
`pruneRenderedMessages()` counts the DOM.

### 14a. The roster is asked for, not broadcast

Adding names to the `UserCount` broadcast would cost O(users) per recipient
on every arrival and departure, for a panel almost nobody has open.
`RequestRoster` is answered to the asker alone, and an open panel stays
current from `UserJoined`/`UserLeft`. Names only; a client has no use for
others' ids (§5.6, §10.10).

### 15a. A message is a row, not a bubble

Only the bubble is coloured and tailed; name, avatar, tapbacks and time sit
outside it, so `renderMessage` builds a row containing a stack. Runs group by
speaker and message time, and only a run's last bubble gets a tail and
avatar. Grouping by render time merged two people talking at once, and the
whole history replay, into one run (§10.6-§10.8).

### 15. The client owns the scroll position, instantly

`#chat` is `scroll-behavior: auto` and `overflow-anchor: none`. Pinning to the
bottom is instant and coalesced into one frame; smooth scrolling is opt-in
(`scrollToMessage`). `programmaticScroll` and `isLoadingHistory` (cleared at
`ReconnectToken`, with a 5s failsafe) suppress false "user scrolled up"
readings. This fixed the chat shifting on reload (§10.2).

### 16. What appears only sometimes is an overlay; what appears on hover survives being aimed at

`#chat:empty` must not restyle the container: it once set
`justify-content: center` and flipped the column on the first message. The
placeholder is an absolutely positioned `::before`/`::after` (§10.3).

The reaction bar is `position: fixed` and a sibling of `#chat`, so moving
towards it leaves the chat. The hide is delayed, arriving on the bar cancels
it, and the bar isn't rebuilt while showing (that replaced the button between
`pointerdown` and `pointerup`) (§10.7).

### 17. Client addresses are hashed, never stored

`ws_handler` hashes the address as soon as it has it. The limiter, pool and
ban list only compare for equality. The hash is keyed per process (std
`RandomState`); unkeyed would be theatre, since IPv4 is enumerable (§5.6).

### 18. The client fetches nothing from anyone else

No webfonts, CDN or analytics: icons are an inline SVG sprite and text uses
the system stack, so the CSP has `font-src 'none'` and no `https://` origin.
A third-party asset discloses every visitor to it;
`the_client_makes_no_third_party_requests` fails if one appears (§5.7).

### 19. The real client IP is `CF-Connecting-IP`

Cloudflare sets it and the Worker forwards it (and fills `X-Forwarded-For`).
Reading only `X-Forwarded-For`, as the server once did, made every visitor the
same address and turned the per-IP limit of three into a global one.

### 20. An identity cookie is a claim; the roster is a closed set

`HttpOnly` stops page script, not the person sending any `Cookie` header they
like. `admit_user` once took `animal_name` verbatim: a chosen name, a megabyte
long, or someone else's. Now the id must parse as a UUID and the name must be
on `ANIMAL_NAMES` and free in the room (`RoomState::claim_animal`), or the
visitor gets a fresh one (§5.9).

### 20a. The identity cookies must survive being stored

They are `HttpOnly; SameSite=Strict; Secure`, and browsers won't store a
`Secure` cookie over plain http (Safari refuses even on localhost). With
nothing stored every reconnect minted a new identity, announced as a
departure and an arrival ("skink left, stinks joined"). `create_user_cookies`
drops `Secure` only for a loopback `Host`. Pinned by
`identity_cookies_are_secure_everywhere_except_loopback` and
`reconnecting_with_the_handshake_cookies_keeps_the_same_identity`.

### 21. Every room's history is trimmed by the timer, not only on join

A busy room with no new joiners used to grow without limit, and since the
byte ceiling is process-wide it could make every room drop messages.
`cleanup_rooms` bounds every room to `MAX_MESSAGES_PER_ROOM`; idle `main`
still fades harder (§1.1).

### 22. The message budget is spent by messages

`can_send_message` was charged for every client event, and typing on/off made
each message cost three units: the real limit was ten a minute, not the
advertised thirty. Typing and read receipts have their own O(1) throttles.
The reaction path stamped its throttle before validating, so a refused event
spent the next one's budget. Validate first, then spend (as in constraint #1).

### 23. An image lives in the room's memory and fades

No object store, and no remote links (the CSP names no external origin, and
fetching one would disclose every visitor, §5.7), so an attachment is bytes in
`chat_history`, allowed by `img-src 'self' data:`. Every attachment constant
is a memory decision. The client downscales and re-encodes in a canvas (which
drops EXIF location) to fit `MAX_ATTACHMENT_BYTES`. Past
`MAX_ROOM_ATTACHMENT_BYTES` a room's oldest payloads are dropped and their
messages marked `faded` (§1.1). `sanitize_attachment` checks the declared
type against the magic bytes and stores the sniffed type. `image/svg+xml`
must stay out of `ALLOWED_ATTACHMENT_MIMES`: SVG can carry script.

### 24. A reaction is a closed set, stored beside the history

The emoji is checked against `emoji.rs`, not sanitised (§5.9). Reactions live
in `RoomState::reactions` keyed by message id, not inside the message, so
applying one is O(1) (§1.1). Every path that removes a message releases its
reactions (§3.5; `reactions_never_outlive_the_messages_they_belong_to`).
`reacted` is resolved per viewer, so one user's reactor list never reaches
another.

## Protocol

### Client to server

```json
{ "type": "Message", "text": "hello",
  "reply_to": { "message_id": "...", "author_name": "...", "preview_text": "..." },
  "attachment": { "mime": "image/webp", "data": "<base64>", "width": 1280, "height": 853 } }
{ "type": "Typing", "is_typing": true }
{ "type": "ReadReceipt", "message_id": "<uuid>" }
{ "type": "React", "message_id": "<uuid>", "emoji": "🔥" }
{ "type": "RequestRoster" }
```

`text` and `attachment` are each optional, but not both absent.

### Server to client

```json
{ "type": "Welcome", "user_id": "<uuid>", "animal_name": "otter" }
{ "type": "Message", "message": { "message_id": "<uuid>", "user_id": "...", "animal_name": "otter", "text": "<sanitised html>", "timestamp": "<ms>",
    "attachment": { "mime": "image/webp", "data": "<base64>", "width": 1280, "height": 853, "faded": false },
    "reactions": [ { "emoji": "🔥", "count": 3, "reacted": true } ] } }
{ "type": "System", "event": { "UserJoined": { "user_id": "...", "animal_name": "otter" } } }
{ "type": "System", "event": { "Reaction": { "message_id": "<uuid>", "user_id": "...", "emoji": "🔥", "active": true, "count": 3 } } }
{ "type": "UserCount", "count": 5 }
{ "type": "Roster", "users": ["badger", "otter"] }
{ "type": "Heartbeat" }
{ "type": "ReconnectToken", "token": "<uuid>" }
```

Frame order on connect is guaranteed: `Welcome`, history, `ReconnectToken`,
then live traffic (constraint #2 depends on it). The nova room's sealed
frames are in `protocol.rs`.

## HTTP routes

| Route | Purpose |
| --- | --- |
| `GET /` | 308 to `/main` |
| `GET /main` | the client |
| `GET /:room` | the client, if the name is valid and there is capacity |
| `GET /health` | liveness probe for the container runtime |
| `GET /metrics` | Prometheus text, seven gauges, gated on `METRICS_TOKEN` (§5.13) |
| `GET /admin` | every open room, gated on `ADMIN_TOKEN` via HTTP Basic; 401, not 404, when unset (§5.14) |
| `GET /robots.txt` | disallows `/ws/` |
| `WS /ws/:room` | the chat |

`/{room}` is registered last; every reserved path is in
`config::RESERVED_ROOM_NAMES`.

## Tests

`src/tests/` has one module per concern, none much over 1,100 lines:
`session_*` mirrors `src/session/`, `rooms_*` covers `room/` and
`cleanup.rs`, `limits_*` covers `limits/`, and `client_ui_*` covers the
shipped page and script. `contracts.rs` holds only the meta-contracts that
keep the other tests honest (documented sweeps, resolvable citations, no dead
exports). Run a subset with `cargo test --all-features roster`.

## Deployment

Pushing to `main` deploys through Cloudflare's Git integration; CI holds no
deploy credential (`ci_holds_no_deploy_credential_and_does_not_deploy`). The
Worker and container deploy together. `./deploy.sh` runs the whole gate and
then `wrangler deploy`, for rollbacks and out-of-band deploys.

The image builds from `Dockerfile.cloudflare`, referenced from
`cloudflare/wrangler.jsonc` so the build context is the repo root.
`max_instances: 1` is a cost decision and a correctness one: state is in
memory, so a second instance would be a second, separate set of rooms behind
the same URLs.

Cloudflare stops containers with SIGINT, so `main.rs` handles `ctrl_c` and
`AppState::shutdown` announces departures before exiting.

## Common pitfalls

- Expecting a running `cargo run` to serve an edited `index.html` (#11).
- Holding the room write lock across a socket send (#5).
- Putting a constant inline instead of in `config.rs` (#10).
- Adding a `ChatError` variant nothing constructs; three were deleted.
- Changing `protocol.rs` without the client in the same commit.
- Trusting per-IP limits as protection: `X-Forwarded-For` is
  attacker-controlled when the container is reached directly. The
  per-connection and global ceilings are the real bound.
