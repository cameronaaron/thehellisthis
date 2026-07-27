# Engineering Standards — infinite-chat (thehellisthis.com)

This document is the permanent record of the correctness and quality bar for
this codebase. **Every rule here is enforced by a test in `src/tests.rs`.** If a
rule matters and has no test, the first task is to write the test — a standard
that isn't executable is a suggestion.

> **Prime directive: when a test fails, fix the SOURCE, not the test.**
> The tests encode decisions made deliberately, usually after finding a real
> defect. Weakening a test to make code pass inverts the entire system.

**This codebase is held to the standard of software that runs unattended.** Not
because a chat room is life-safety equipment, but because that discipline is the
concrete, executable version of "obsessive engineering quality." Concretely:

- **A defect, once found, gets a permanent test in the same commit that fixes
  it.** The ratchet in §6 only tightens. A bug that recurs after being fixed
  once is a process failure, not bad luck.
- **A defect, once found, becomes a sweep for its whole class.** Fixing the
  instance is the smaller half of the job. §6.3.
- **Nothing ships on "should work."** Every claim in this document is backed by
  a measurement, a reproduction, or a test.
- **The blast radius of a failure is a design input, not an outcome.** This is
  a server: one process holds every room, and a mistake does not inconvenience
  one user, it disconnects all of them. §5.
- **"What happens when the client is hostile, slow, or gone" is the first
  question, not an edge case asked afterward.** Every input on the socket is
  attacker-controlled, every client can vanish mid-frame, and the room lock is
  shared by everyone. A feature that works when all four clients are healthy on
  localhost is not finished.

## Enforcing tests

| Law | Enforced by |
| --- | --- |
| §1 Complexity | `assign_animal_never_repeats_a_connected_name`, memory-pruning tests |
| §2 Lock discipline | WebSocket integration tests (a held lock deadlocks them) |
| §3 Memory ceiling | `test_memory_tracker_*`, pruning and trim tests |
| §4 Protocol | `first_frame_is_welcome_with_this_users_identity`, `client_takes_identity_from_welcome_frame_not_cookies` |
| §5 Blast radius | `rejected_upgrade_releases_its_connection_slot` |
| §6 Ratchet | `cargo fmt`/`clippy -D warnings`/`cargo test` in CI |
| §7 Engagement | frontend/backend consistency tests (`EMBEDDED_HTML`) |
| §8 Naming | `animal_roster_is_sorted_unique_and_well_formed` |
| §9 Shipped artifact | `router_mounts_every_public_route` |

---

## 0. The first-principles doctrine — how every other rule is derived

Everything below §0 is a *conclusion*. This section is the *method* that
produced them and the method every future change must use. The rules in §1–§9
are not sacred because they are written down; they are correct because they were
derived from ground truth, and the moment a better derivation appears, backed by
evidence, the rule changes and the ratchet locks the new floor.

**Reason from the mechanics, never from the convention.** The default
engineering move is analogy: "this is how chat servers / Rust services /
everyone does it." Analogy copies other people's constraints along with their
solution. First principles instead asks: what is *actually* true here — how long
this lock is held, how many allocations this message costs, what the container's
memory limit is, what the client can actually observe — and what is the best
thing those facts permit? A "best practice" is a cached answer to someone else's
question; verify it still holds for *our* question or discard it.

### 0.1 "Impossible" is a measurement you haven't taken yet

Treat *impossible* as a bug report against your own understanding. Almost every
"can't be done" is one of three things in disguise:

1. **Unmeasured** — nobody profiled it; the wall is assumed.
2. **Un-questioned requirement** — the expensive thing shouldn't exist (0.2
   step 1).
3. **Expensive, not impossible** — it costs something, and the real decision is
   whether the win is worth the price, stated honestly.

The genuine walls are physical: network round-trip time, the container's memory
ceiling, the fact that one process cannot hold two independent copies of the
same room. You must *prove* you've hit one, with a number, before accepting it.

### 0.2 The algorithm (apply strictly in order)

The ordering is the whole point — most engineering waste is optimising, and
even automating, something that should have been deleted two steps earlier.

1. **Question the requirement.** Every requirement carries the name of a person,
   never a department or a convention, so it can be challenged. "The error type
   needs a variant for every failure category," "rooms need persistence,"
   "per-IP limits protect us" are requirements to interrogate, not givens.
   Requirements from senior sources included — those get questioned least and
   are therefore the most dangerous.
2. **Delete the part or the process.** The best code is no code. If you are not
   later forced to add back ~10% of what you removed, you did not delete enough.
   Deleted code has no bugs, no tests to maintain, and no cost.
   *Applied here:* four dependencies (`metrics`, `metrics-exporter-prometheus`,
   `async-trait`, `hyper`) declared and never referenced; three `ChatError`
   variants nothing constructed; one unreachable validation branch in
   `room_handler`; `lazy_static` replaced by `std::sync::LazyLock`. All deleted.
3. **Simplify or optimise — only what survived step 2.** Optimising something
   that should not exist is the most common form of wasted work.
4. **Accelerate.** Only once it is simple and correct.
5. **Automate.** Last. Automating a broken process makes it break faster. A test
   that pins a wrong behaviour is worse than no test.

### 0.3 Novelty requires evidence, not enthusiasm

A new approach ships only when it demonstrably beats the incumbent on a stated
measure. An approach that loses gets **deleted and recorded** — the record is
what stops the next session from re-litigating it. §9.4 is the registry.

### 0.4 Say what is true

Report what happened, not what was hoped. If a change is unverified, say so. If
a test was skipped, say which. The README claimed `HttpOnly` cookies for months
while the code set no such flag, and claimed a "~7.5 MB optimized release
binary" while no release profile existed. Both claims were plausible, load-
bearing for a reader's trust, and false. Documentation that describes intent
rather than reality is a defect with the same severity as a code defect, because
it is what the next engineer reasons from.

### 0.5 Validate the instrument before trusting the measurement

**Before theorising about the system, audit the ruler.** A number that surprises
you is more often a broken instrument than a broken system.

Concretely, for this codebase:

- **A `cargo test` pass proves less than it looks** if the test asserts a
  tautology. `assert!(result || !result)` sat in the suite asserting nothing
  about `should_gc`; it was found by `clippy::overly_complex_bool_expr`, not by
  a human reading it, and replaced with a real assertion about the rate-limiting
  property.
- **A timing test that depends on wall-clock is measuring the CI runner**, not
  the code. Prefer asserting the *property* (`should_gc` refuses a second sweep)
  over the *duration*.
- **Load numbers from localhost are not load numbers.** Every client shares one
  loopback with no latency, no packet loss, and no TLS. Real clients arrive
  through a Worker, over the internet, on hostile networks.
- **`cargo build` timing is not runtime performance.** The release profile added
  LTO and `codegen-units = 1`, which made builds slower and the binary smaller
  (5.4 MB). Neither number says anything about how fast a message is delivered.

If a local number disagrees sharply with production, audit the instrument before
theorising about the server.

---

## 1. The complexity doctrine

**Per-message and per-event work is O(1). Work proportional to history or user
count runs rarely, and never on the message path.**

A chat server's hot path is one message arriving and fanning out. Everything on
that path is paid for by every user, on every message, while holding a lock the
whole room contends on (§2). Everything off it is paid once.

### 1.1 The classification

| Path | Frequency | Budget |
| --- | --- | --- |
| Message receive → validate → render → broadcast | every message | O(1) |
| Typing / read-receipt events | up to 5/s per user | O(1) |
| Join / admission | once per connection | O(history) acceptable |
| Housekeeping sweep | once per 60s | O(rooms × history) acceptable |

### 1.2 Never shift from the front of a `Vec`

`prune_old_messages` used to call `Vec::remove(0)` in a loop. Each call shifts
every remaining element, so pruning *k* messages from a history of *n* costs
O(k·n) — on the message path, under the room write lock.

```rust
// ❌ was: O(k·n), re-shifting the whole history on every iteration
while removed_size < needed_space && !self.chat_history.is_empty() {
    if let Some(msg) = self.chat_history.first() {
        removed_size += msg.estimate_size();
        self.chat_history.remove(0);
    }
}

// ✅ now: count first, then one drain — O(n)
let mut drop_count = 0;
for msg in &self.chat_history {
    if removed_size >= needed_space { break; }
    removed_size += msg.estimate_size();
    drop_count += 1;
}
self.chat_history.drain(..drop_count);
```

### 1.3 Never allocate a copy of a collection to delete from it

`cleanup_messages` used to build a second `Vec` and clone every *surviving*
message into it — allocating a duplicate of the entire history in order to
remove a handful of entries. `retain` does it in place:

```rust
// ✅ in-place, no allocation
self.chat_history.retain(|msg| {
    if removed >= CLEANUP_BATCH_SIZE { return true; }
    // ...
});
```

### 1.4 Build the lookup set once, not per candidate

`assign_animal` scanned every user in the room for every candidate name:
O(pool × users). With a full room and a full roster that is ~25,000 string
comparisons, under the write lock. Build the set once:

```rust
// ✅ O(users) to build, O(1) per lookup, O(pool) to scan
let taken: HashSet<&str> = self.users.values()
    .filter(|u| u.is_connected())
    .map(|u| u.animal_name.as_str())
    .collect();
```

Pinned by `assign_animal_never_repeats_a_connected_name`.

### 1.5 Slack keeps linear work off the hot path

`HISTORY_TRIM_SLACK` (100) exists so history is trimmed only once it has drifted
well past the cap. Trimming exactly at `MAX_MESSAGES_PER_ROOM` would move the
whole history on *every single message* once a room is busy — the cap turning
into an O(n)-per-message cost precisely when the room is most active.

### 1.6 Prefer the const over the allocation

`ANIMAL_NAMES` is a `&'static [&'static str]` living in the binary, not a
`Vec<String>` rebuilt per room. The roster is immutable and shared; only the
per-room shuffled *order* is state.

---

## 2. The lock-discipline law

`AppState::rooms` is a single `RwLock<HashMap<String, RoomState>>`. Every user
in every room contends on it. **Lock duration, not CPU, is this server's
scaling limit.**

### 2.1 Never hold the room lock across an await on the socket

A socket send completes when the *client* is ready. A slow phone on a train can
take seconds. Holding the room lock across that stalls every user in every room.

```rust
// ✅ clone what is needed under the lock, release, then send
let (mut receiver, chat_history) = {
    let mut rooms = state.rooms.write().await;
    // ...
    (receiver, room_state.chat_history.clone())
};   // <- guard dropped here
for message in &chat_history { /* send */ }
```

The `chat_history.clone()` is a deliberate allocation bought to shorten a
critical section. That is the correct trade and the reason it is not "optimised"
away.

### 2.2 Take the lock once, decide everything, release

`cleanup_rooms` takes the write lock once and does its whole pass inside it.
Splitting into a read pass and a write pass would mean re-deriving every
decision after the map may have changed underneath — more code, a race, and no
less contention for a lock held tens of microseconds either way.

### 2.3 The liveness probe must never take the write lock

`/health` is polled by the container runtime to decide whether to kill the
instance. If it can block behind a write lock, a busy server looks dead and gets
restarted — turning load into an outage.

### 2.4 Check cheaply before locking

`AppState::cleanup` calls `should_gc()` (two atomics) and returns before taking
the write lock when there is no work. The lock is taken only once something is
known to need doing.

---

## 3. The memory-ceiling law

There is no database. Every message lives in RAM until something removes it, in
a container with a hard memory limit. **Exceeding that limit is an OOM kill,
which disconnects every user in every room.**

### 3.1 Refuse rather than grow

`MemoryTracker::add_bytes` returns `false` and reserves nothing when the message
would breach the ceiling. Dropping one message is strictly better than losing
every session.

### 3.2 Over-estimate, never under-estimate

`ESTIMATED_MESSAGE_SIZE` (1 KB) is added to every message's accounted size, on
top of its actual string lengths, to cover allocator and `Vec` overhead the
lengths cannot see. Under-counting means the process believes it is within
budget while the kernel disagrees — and the kernel wins.

### 3.3 Every counter is saturating

`remove_bytes` uses `saturating_sub`. An accounting slip that underflows a
`usize` produces `usize::MAX`, and the server then believes it is permanently
full while completely idle. Same reasoning in `should_gc`, where a backwards
clock step (NTP correction, container migration) would otherwise panic in debug
and wedge GC in release.

### 3.4 Recompute where it is already linear; subtract where it is not

`retain_newest` recomputes the room's byte total from its history rather than
subtracting. It is already doing O(history) work, and recomputing is
**self-healing**: an accounting slip anywhere else is corrected at the next
trim, instead of accumulating until the room wrongly reports itself full. On the
message path, where recomputation would be O(n)-per-message, `prune_old_messages`
subtracts instead. The rule is not "always recompute" — it is "self-heal
wherever it is free."

### 3.5 Every unbounded map needs an eviction path

`ConnectionPool::ip_counters` is keyed by client IP — attacker-influenced and
otherwise unbounded. `cleanup_stale` drops entries older than
`IP_COUNTER_RETENTION`, so the map is bounded by *recent* clients, not by every
client ever seen. Any new map keyed by client-supplied data needs the same.

---

## 4. The protocol law

`protocol.rs` is a contract with a program that is already running in someone's
browser. **A protocol change is a client change, in the same commit.**

### 4.1 Tell the client what it needs; never make it guess

The client used to infer its own identity two ways, both wrong:

1. It assumed the first `UserJoined` it received was itself. True on a quiet
   server; wrong when two people connect at once.
2. Failing that, it parsed `document.cookie` — which is why the identity cookies
   could not be `HttpOnly`, on a server whose entire job is turning
   user-supplied Markdown into HTML.

The fix is one frame: `Welcome { user_id, animal_name }`, sent first on every
connection. It deleted both heuristics and unblocked `HttpOnly`. **A guess in a
client is usually a missing field in a protocol.**

### 4.2 Frame ordering is part of the contract

`Welcome` → history → `ReconnectToken` → live traffic. The client's ability to
distinguish its own messages depends on `Welcome` preceding history. Ordering
that something depends on is a contract, and it is tested
(`first_frame_is_welcome_with_this_users_identity`).

### 4.3 Two halves of one decision get one test

`HttpOnly` cookies and the `Welcome` frame are a single change. Either half
reverted alone leaves the client silently rendering the user's own messages as
somebody else's — no error, no crash, just wrong.
`client_takes_identity_from_welcome_frame_not_cookies` asserts both halves
together, and asserts the *absence* of `document.cookie` parsing in the shipped
client.

### 4.4 Sanitise the representation the browser receives

`markdown_to_html` **then** `ammonia::clean`. Sanitising Markdown first mangles
legitimate syntax and leaves the renderer's output unchecked. The only
representation that reaches a browser is the generated HTML, so that is what
gets sanitised.

### 4.5 The error taxonomy describes failures that happen

`ChatError` carried `RateLimited`, `ConnectionError` and `RoomError` — variants
nothing but their own tests ever constructed. A taxonomy of failures the server
cannot produce is speculative generality: it makes the type harder to read,
implies capability that does not exist, and its tests report coverage of
unreachable code. Deleted. Add a variant when a real call site returns it.

---

## 5. The failure-blast-radius law

One process holds every room. Design for what a failure takes down with it.

### 5.1 Unwind, do not abort

`panic = "abort"` is deliberately **not** set in the release profile. Tokio
confines a panicking task to that task; the connection dies and everyone else
continues. Under `abort`, one panic in one connection handler kills the process
and disconnects every user in every room. The smaller binary and marginally
faster code are not worth trading a per-connection failure for a total one.

### 5.2 Every reservation is released on every exit path

The admission path used to increment the per-IP pool counter and the global
connection counter, and *then* run checks that could fail — returning without
releasing either. Every rejected connection permanently consumed a slot. A
server that had refused enough connections would refuse all of them **while
completely idle**, with no error to point at.

The structural fix is ordering, not cleanup code:

1. Every check that can fail, mutating nothing.
2. Only then, the reservations.
3. The one fallible step that must come after releases explicitly.

Pinned by `rejected_upgrade_releases_its_connection_slot`, which also asserts
the server still accepts a good connection afterwards — the property that
actually matters to a user.

### 5.3 Shut down loudly

Cloudflare stops containers with SIGINT. `AppState::shutdown` announces
`UserLeft` for every connected user before exiting, so a routine scale-down
reads as people leaving rather than as the app breaking. Best-effort by design:
the grace period is short, and a clean signal to the people watching is worth
more than a guaranteed one from a process that is exiting anyway.

### 5.4 Teardown must be idempotent under reconnection

`cleanup_user` compares `connection_id` before acting. A user who reconnected
before the old session's teardown ran already has a *newer* live connection;
marking them disconnected would evict the session that is currently working.
Any teardown that can race a re-establish needs this guard.

### 5.5 Trust boundaries are stated, not assumed

`X-Forwarded-For` is the only place the real client IP appears, because
Cloudflare terminates in front of the container. It is also entirely
attacker-controlled if the container is reached directly. Per-IP limits are
therefore a **courtesy bound**; the per-connection and global ceilings are the
real protection. Documented at `extract_client_ip` so nobody mistakes one for
the other.

---

## 6. The regression ratchet — how standards stay upheld

### 6.1 The gate

Four commands, run by `.github/workflows/ci.yml` and expected to pass locally
before every commit:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

**Warnings are failures.** This is not aesthetic: `clippy` found the tautological
assertion in §0.5 that a human reading the file had missed.

The clippy gate was red for some time before this was noticed, because nothing
downstream depended on it. `deploy.yml` now **waits for CI**, so a red gate
cannot reach production.

### 6.2 A fix and its test are one commit

Non-negotiable. A fix without a test is a fix with an expiry date. The commit
that fixes the connection leak contains the test that detects it; the commit
that curates the animal roster contains the sweep that keeps it curated.

### 6.3 A found defect becomes a sweep for its class

Fixing the instance is the smaller half. The commit must also add the check that
finds the *next* instance, and the sweep must be proven able to fail — inject a
violation, watch it fire, revert — before it is trusted.

Worked example from this codebase: the animal roster contained non-animals
(`hadron`, `gyroscope`, `half-penny`, `hallway`) and duplicates. Fixing it means
curating the list. The *sweep* is
`animal_roster_is_sorted_unique_and_well_formed`, which asserts sortedness,
uniqueness, and character shape. It proved itself immediately by failing on a
sorting slip in the replacement list (`pika, pike, pigeon`) — a defect
introduced by the very commit that fixed the original one.

### 6.4 Assert properties, not tautologies

A test that cannot fail is worse than no test: it reports coverage it does not
provide. `assert!(x || !x)` and `assert!(result.is_ok() || result.is_err())` are
the recognisable forms. Assert the property you actually care about — for
`should_gc`, that a second immediate call returns `false`.

### 6.5 Tests declare their own imports

`tests.rs` imports each module explicitly rather than through one `use crate::*`
that inherits whatever `main.rs` happened to import. When a test fails, the
import list says which part of the server it belongs to; and the server's
internal organisation can change without the test file silently depending on it.

### 6.6 Build artifacts are never committed

`target/` (including a 7.5 MB compiled binary) and 1.6 MB of tarpaulin coverage
reports were tracked in git. They bloat every clone, produce meaningless diffs,
and go stale the moment anyone builds. `Cargo.lock` **is** tracked — this is a
binary crate, and the deployed container must build from the exact dependency
set CI tested. The old `.gitignore` had these exactly backwards.

---

## 7. The engagement doctrine

Several behaviours that look like resource management are **the product**.
Deleting them as "unnecessary cleanup" would delete the thing that makes the
site work.

### 7.1 Scarcity is the feature

| Mechanic | Constant | Why |
| --- | --- | --- |
| Empty rooms are deleted | `EMPTY_ROOM_CLEANUP_DELAY` (10 min) | A room that outlives its conversation is a ghost town. Deleting it means every room you find has someone in it. |
| Idle users are disconnected | `USER_IDLE_MESSAGE_TIMEOUT` (10 min) | So the user count reads "people actually here," not "tabs left open." |
| `main` history fades | `MAIN_ROOM_FADE_IDLE` → `MAIN_ROOM_FADE_KEEP` | `main` can't be deleted, so it forgets instead. |

These are **product decisions with technical implementations**, which is why
they live in `config.rs` with their reasoning attached rather than as bare
numbers in a cleanup loop.

### 7.2 `main` is special and stays special

`cleanup.rs` special-cases `MAIN_ROOM` and `continue`s before the deletion
branch. `main` is the entry point; deleting it would 404 the front door. Any
refactor that "unifies" the room-cleanup path must preserve this.

### 7.3 The UI must not lie about the mechanics

`index.html` tells users when rooms disappear. `config.rs` decides when they
actually do. If those disagree, the UI is lying — and it has: the fade warning
once fired at the wrong threshold, and the frontend once described a timeout the
backend had changed. Tests over `EMBEDDED_HTML` assert the user-facing text
matches the backend constant. **A user-visible number is a constant with two
call sites, and both must be checked.**

---

## 8. Naming and organisation law

### 8.1 A module has one job and states it

Every module opens with a header comment saying what it owns and, where
non-obvious, what it deliberately does not. No module is named `utils`,
`helpers`, `common`, or `misc` — those names describe where code was put, not
what it does, and they grow without bound because nothing can be said not to
belong.

`main.rs` is wiring only: `mod` declarations, the router, the housekeeping
loops, `main`. Logic in `main.rs` is logic in the wrong place. It went from 2,369
lines holding the entire server to ~180 lines of wiring across twelve modules.

### 8.2 A name states the thing, not the mechanism

`SANITIZE_TIMEOUT` was a 50 ms window in which an identical message from the
same user is treated as a double-send. It has nothing to do with sanitisation
timing out. Renamed `DUPLICATE_MESSAGE_WINDOW`. A name that describes the wrong
concept is worse than a vague one: it actively teaches the next reader something
false.

### 8.3 Constants carry their reasoning

Every constant in `config.rs` has a comment saying *why* it has its value — the
mechanism it protects, or the product decision it encodes. `3600` appearing in
four places meant four different things (cookie lifetime, ban duration,
disconnected-user retention, IP-counter retention); they are now four named
constants that can move independently.

### 8.4 Factor for the test, when the test is the point

`build_router` exists as a separate function so tests exercise the **real**
route table rather than a hand-assembled copy that drifts from `main`. Extracting
a function purely so a test can see the real thing is good design, not test
pollution. §9.

---

## 9. The shipped-artifact law — what the user receives is the only truth

### 9.1 Verify against the artifact, not the source that should have produced it

The client is `include_str!`'d into the binary. What a visitor runs is what was
compiled, not what is in the working tree — a rebuild is the only thing that
makes an `index.html` edit real. Tests that assert on the client assert over
`EMBEDDED_HTML`, the same constant the server serves, for exactly this reason.

### 9.2 Test the route table that ships

`router_mounts_every_public_route` builds the router with `build_router` — the
function `main` calls — and asserts every public path responds. A test that
constructs its own `Router::new().route(...)` proves that *axum* works, not that
*this server* is reachable.

### 9.3 Documentation is part of the artifact

`README.md`, `CLAUDE.md` and this file are read by the next engineer and
believed. The README claimed `HttpOnly` cookies the code did not set, a "~7.5 MB
optimized release binary" with no release profile configured, "397 tests" and
"21 tests" in two different sections, and Axum 0.7 while `Cargo.toml` pinned
0.8. Each was individually harmless and collectively corrosive: a document that
is wrong about the things you can check is not trusted about the things you
cannot. §0.4.

### 9.4 The watched-levers registry

Every measured rejection, deferred optimisation, or upstream block records its
reopen condition here, so parked decisions resurface instead of fossilising into
lore.

| Lever | Status | Reopen when |
| --- | --- | --- |
| `panic = "abort"` in release | **Rejected** (§5.1) — trades a per-connection failure for a total one | Never, while one process holds all rooms. Reopens if rooms are ever sharded across processes. |
| Multiple container instances | **Blocked** — all state is in-process memory, so a second instance is a second, separate set of rooms behind the same URLs | Reopens only with shared state (Durable Objects, or rooms pinned to instances by name). `max_instances: 1` is correctness, not just cost. |
| `chat_history` as `VecDeque` | **Deferred** — would make front-removal O(1) natively, but `drain(..k)` already made pruning O(n) once per prune, and `Vec` keeps slicing/indexing that ~40 tests use | Reopens if pruning ever moves onto the per-message path, where the constant factor would matter. |
| Per-IP rate limiting as real protection | **Rejected as a security boundary** (§5.5) — `X-Forwarded-For` is attacker-controlled | Reopens if the container stops being reachable except through the Worker, verifiable at the network layer. |
| Sliding-window rate limiter | **Rejected** — needs a timestamp per event; the fixed window's edge behaviour is imperceptible in chat | Reopens if burst abuse is observed crossing window boundaries in production. |
| `cargo-audit` / `cargo-deny` in CI | **Not yet added** — CI currently gates fmt, clippy, tests, build | Add when a dependency CVE is missed by manual review, or before the dependency count grows materially. |
| Removing the double `ammonia::clean` | **Deferred** — `validate_message` sanitises to detect markup-only messages, then `render_message_html` sanitises the rendered HTML. Two passes over ≤8 KB, microseconds | Reopens if message throughput is ever measured as CPU-bound. |
| Load testing | **Never done** — every performance claim here is structural (complexity, lock duration), not empirical throughput | Before raising `MAX_CONCURRENT_USERS` (400) or `MAX_USERS_PER_ROOM` (100). Those numbers are currently unvalidated assumptions, and §0.5 says so out loud. |

---

## Appendix: what changed, and why it is written down

This document was rewritten wholesale in July 2026. The file it replaced
described a Next.js portfolio site — React render paths, Lighthouse budgets,
`content-visibility`, mobile tap targets — none of which exists in this
repository. It had been copied in and left, which meant the project's
"authoritative rulebook" was authoritative about a different project.

That is itself the §0.4 failure this document warns about, at the largest
possible scale: a standards file nobody could follow, cited by a `CLAUDE.md`
that described a component tree this repository does not have. The laws above
are derived from *this* server — its lock, its memory ceiling, its protocol, and
the specific defects found while writing them down.
