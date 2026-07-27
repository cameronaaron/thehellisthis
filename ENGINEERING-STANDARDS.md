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
| §3 Memory ceiling | `test_memory_tracker_*`, pruning and trim tests, `every_room_history_is_bounded_by_housekeeping_not_only_by_joins`, `rendered_html_is_bounded_by_the_input_cap` |
| §3.3 Saturating counters | `releasing_more_than_was_reserved_cannot_wrap_a_counter` |
| §3.5 Map eviction | `every_client_keyed_map_is_swept_by_housekeeping`, `a_live_connection_counter_survives_the_stale_sweep` |
| §4 Protocol | `first_frame_is_welcome_with_this_users_identity`, `client_takes_identity_from_welcome_frame_not_cookies` |
| §5 Blast radius | `rejected_upgrade_releases_its_connection_slot`, `a_full_room_refuses_and_releases_its_reservations`, `a_session_whose_room_disappeared_still_releases_its_slots` |
| §5.5 CSP | `every_response_carries_security_headers`, `client_script_is_external_so_csp_can_forbid_inline` |
| §5.6 Privacy | `client_addresses_are_hashed_not_stored_in_the_clear`, `the_upgrade_path_hashes_the_address_at_the_boundary` |
| §5.7 No third parties | `the_client_makes_no_third_party_requests`, `the_policy_permits_no_third_party_origins` |
| §5.8 Trust boundary | `client_address_prefers_the_header_cloudflare_sets`, `worker_forwards_the_client_address` |
| §5.9 Identity is a claim | `a_forged_identity_cookie_cannot_choose_its_own_name_or_id`, `a_cookie_name_never_duplicates_a_name_already_in_the_room`, `a_returning_visitor_keeps_a_roster_name_that_is_free` |
| §5.10 Saturating admission counters | `releasing_more_than_was_reserved_cannot_wrap_a_counter` |
| §5.11 Release in a wrapper | `a_session_whose_room_disappeared_still_releases_its_slots` |
| §6 Ratchet | `cargo fmt`/`clippy -D warnings`/`cargo test`/`scripts/coverage.sh` in CI |
| §4.5 Error taxonomy | `every_error_tells_the_client_a_usable_category`, `a_valid_id_does_not_let_a_cookie_invent_its_own_name` |
| §6.8 Boundaries | `the_memory_ceiling_admits_exactly_the_limit`, `an_ip_is_banned_only_past_the_suspicion_threshold` |
| §7 Engagement | frontend/backend consistency tests (`SHIPPED_CLIENT`) |
| §8 Naming & organisation | `every_module_is_named_for_its_job_and_says_what_it_is`, `nothing_is_public_only_for_its_own_test` |
| §9.3 Docs are part of the artifact | `every_section_reference_resolves` |
| §8 Naming | `animal_roster_is_sorted_unique_and_well_formed`, `the_roster_is_larger_than_a_room_can_ever_be`, `reaction_roster_is_sorted_unique_and_actually_emoji` |
| §9 Shipped artifact | `router_mounts_every_public_route`, `page_references_the_versioned_script_url`, `every_element_the_client_looks_up_exists_in_the_page`, `the_client_declares_every_screaming_case_constant_it_uses` |
| §6.10 Removal is a decision | `the_shipped_client_still_contains_everything_it_did`, `the_servers_public_surface_still_exists` |
| §10.9 Show implies hide | `every_conditional_display_rule_has_a_base_that_hides_it` |
| §10.8 Rendered classes are styled | `every_class_the_client_renders_is_styled` |
| §10.6 One rule per selector | `no_css_selector_is_defined_twice`, `the_conversation_is_a_readable_centred_column` |
| §10.7 Hover targets are reachable | `the_reaction_bar_can_actually_be_clicked` |
| §10 Client | `empty_chat_placeholder_does_not_alter_container_layout`, `rendered_history_is_trimmed_from_the_dom_not_a_counter`, `the_page_does_not_block_pinch_zoom`, `images_reserve_their_space_before_they_load` |
| Attachments | `an_attachment_must_be_the_image_type_it_claims_to_be`, `svg_is_not_an_allowed_attachment_type`, `a_rooms_oldest_images_fade_once_it_is_over_its_attachment_budget`, `images_are_re_encoded_rather_than_sent_as_picked` |
| Reactions | `reacting_twice_with_the_same_emoji_removes_the_reaction`, `reactions_never_outlive_the_messages_they_belong_to`, `a_reaction_must_be_on_the_roster` |
| §6.1 The gate answers "will this deploy" | `ci_node_version_satisfies_the_toolchain`, `the_workflows_install_with_the_lockfile_that_exists`, `ci_holds_no_deploy_credential_and_does_not_deploy` |
| Startup lifecycle | `run_returns_cleanly_when_its_shutdown_fires`, `the_process_exit_code_reports_a_clean_stop`, `the_process_exit_code_reports_a_failed_bind`, `the_shutdown_sequence_waits_for_its_signal`, `a_server_error_is_reported_and_a_clean_stop_is_not_an_error`, `binding_a_port_already_in_use_is_an_error_not_a_panic`, `run_serves_until_it_is_shut_down`, `shutting_down_announces_departures_and_clears_the_rooms` |
| Constraint #12 Frontend/backend agreement | `client_attachment_ceiling_matches_the_server`, `client_reaction_roster_matches_the_server`, `the_node_types_match_the_node_ci_installs` |
| Operational log is a contract | `housekeeping_reports_a_trim_only_when_it_trims`, `housekeeping_reports_the_rooms_it_deletes` |
| Housekeeping boundaries | `the_housekeeping_boundaries_are_exact`, `the_empty_room_grace_period_is_exact`, `deleting_a_room_returns_its_bytes_to_the_process_budget`, `deleting_an_empty_room_reclaims_nothing` |
| Meta: coverage exemptions | `coverage_exemptions_are_justified_and_current` |
| Meta: standards are enforced | `every_test_the_standards_name_exists`, `every_parked_decision_records_how_to_reopen_it`, `every_contract_test_is_documented` |
| Meta: tests can fail | `no_assertion_in_this_suite_is_a_tautology` |
| Dead weight | `every_declared_dependency_is_used` |
| Testability | `shutting_down_announces_departures_and_clears_the_rooms`, `binding_a_port_already_in_use_is_an_error_not_a_panic`, `run_serves_until_it_is_shut_down` |
| §7 Rooms fade | `a_room_nobody_is_in_is_deleted_and_main_never_is`, `the_client_does_not_reconnect_after_an_idle_eviction`, `client_and_server_agree_on_the_idle_close_code`, `an_idle_user_is_evicted_with_the_close_code_the_client_expects` |
| Attachment plumbing | `the_base64_prefix_decoder_handles_its_whole_alphabet`, `image_sniffing_recognises_each_allowed_format`, `an_empty_attachment_is_refused`, `a_jpeg_attachment_is_accepted` |
| Reaction integrity | `a_reaction_for_a_message_that_does_not_exist_is_refused`, `the_message_id_index_never_drifts_from_the_history`, `history_resolves_reactions_for_the_viewer_receiving_it` |
| Housekeeping runs | `the_housekeeping_loops_run_on_their_own`, `fading_with_nothing_left_to_fade_changes_nothing` |
| Ratchets | `the_memory_budget_still_closes`, `the_join_path_shares_history_rather_than_copying_it`, `adding_a_message_costs_the_same_whatever_the_history_holds`, `reacting_never_touches_the_history` |
| Dead webfonts | `the_page_names_no_font_it_does_not_ship_with`, `no_css_content_string_is_an_icon_ligature` |

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

The two "acceptable" rows are a budget, not a dumping ground. A bound that runs
**only** on the join path is not a bound: nothing obliges anybody to join.
History trimming lived there — `admit_user` trimmed, and `cleanup_rooms` faded
`main` — so a room that was busy but had no new joiners grew without limit, and
because the byte ceiling is process-wide, one such room could fill it and make
every room on the server start dropping messages. **Anything that must always
hold belongs on the timer**; the join path may repeat it to pay the cost early,
but must not be the only thing doing it.

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

### 1.4a Share what is immutable; copy only what differs

A stored message never changes, so there is no reason for every join to get its
own copy of one. `chat_history` holds `Arc<OutgoingMessage>` and the join path
clones refcounts.

The number is what makes this worth writing down. Copying a full room's history
— 500 messages, ~2 MB of attachments — measured **119 µs**, and it happened
*under the room write lock*, so that cost was charged to every user in every
room rather than to the one joining. The same copy as `Arc` clones is **1.3 µs**.
It was the largest lock hold in the server and it was a type away from not
existing.

What could *not* be shared is the per-viewer part: whether **you** reacted has a
different answer for each recipient. That is why `reactions` is not a field of
`OutgoingMessage` — putting it there would have forced a full copy of the
message per viewer just to fill it in, which is the copy this removed. It is
carried alongside instead, and flattened onto the wire so the frame is
byte-identical to a live one.

**Split a structure along the line between what is shared and what differs**,
then share the first half. §3.4's "self-heal where it is free" and §1.4a's "do
not keep a second copy" are the same instinct applied to time and to state.

### 1.4b Derive the answer; do not keep a second copy of it

`available_animals` was a free list: names came out when assigned and callers
put them back when a user was reclaimed. The bookkeeping did not balance. A name
that never came out of *that room's* list — one carried in on a cookie, or a
`guest_N` fallback — was pushed back anyway, so the pool grew on every such
reclaim and filled with strings that were not on the roster. Two copies of "which
names are free" existed, and only one of them was right.

The fix deletes the second copy rather than repairing it (§0.2, step 2). Whether
a name is in use is *derived* from `users`, which `assign_animal` was already
doing to build its `taken` set — so the free list was never load-bearing, only
the rotation order was. The pool is now a rotation cursor that never changes
membership: every name drawn is pushed to the back whether or not it was handed
out, and the pool is always exactly the roster in this room's order.

This is the same shape as §10.1 on the client (`messageCount` drifting from the
real number of DOM nodes) and §3.4 on the server. **A derived quantity stored
separately is a quantity that will disagree with itself.** Keep the copy only
where recomputation is genuinely too expensive, and say so where you keep it.

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

**Checking expiry on read is not an eviction path.** `SecurityManager` held two
such maps and looked expired for neither: `check_ip` tested a ban's age on every
read but never removed it, and a suspicion record was never removed at all.
Reading the entry and deciding it no longer *means* anything bounds the
entry's effect, not its existence — the maps still grew by one entry per address
ever seen. The two are easy to confuse because the expiry constant appears in
the code and looks like it is doing the work.

Every such map is swept from one place, `AppState::cleanup`, so adding a map
without adding a sweep is visible at a single line. Pinned by
`every_client_keyed_map_is_swept_by_housekeeping`.

**And eviction must not evict something live.** `ip_counters` entries were
stamped when a connection was *added*, so a session outlasting the retention
window — which any user who keeps talking does — had its counter swept out from
under it, silently lifting the per-IP limit for that address. A counter that is
still counting is not stale at any age; age only decides for entries at zero.

### 3.6 Bound what you store, not just what you accept

`MAX_MESSAGE_LEN` bounds the Markdown a user sends. What the server *stores in
history and broadcasts to every socket in the room* is the rendered HTML, and
rendering amplifies: measured at 7.2x for `[a](b)` repeated to the input cap —
8 KB in, 57 KB out. The input cap therefore bounded nothing that costs memory or
bandwidth, while looking exactly like it did.

Wherever input is transformed before being retained, the ceiling belongs on the
*output* of the transformation. `MAX_RENDERED_MESSAGE_LEN` is the ceiling, and
its value is derived rather than chosen: 5x, because escaping expands by at most
5x (`&` → `&amp;`, measured 8 000 → 40 000), so the escaped-plain-text fallback
is guaranteed to fit under it. A fallback that can itself breach the limit is
not a fallback.

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

### 5.5 The browser gets a policy, not just sanitised output

The message pipeline turns user input into HTML. Sanitising is the first line of
defence; the Content-Security-Policy is the second, and it is the one that still
holds if the first is bypassed.

`script-src 'self'` is the directive that matters, and it is the reason the
client's JavaScript lives in `/app.js` rather than in an inline `<script>`
block. While the script was inline, any policy that let the page work had to
include `'unsafe-inline'` — which grants exactly the capability an injected
`<script>` needs. **A CSP that permits inline script on a page that renders user
HTML is decoration.** Moving one script tag is what turned it into a control.

The server sent no security headers at all before this. Cloudflare adds none of
its own to a Worker-proxied origin, so "the CDN handles it" was an assumption
nobody had checked — §0.5 applied to security rather than to performance.

### 5.6 Store the comparison, not the identity

The server held every visitor's real IP address in memory — as keys in the
connection pool and the ban list — and logged it on the upgrade path. None of
that storage ever needed the actual address: the rate limiter, connection pool
and ban list only compare addresses for **equality**.

So the address is hashed at the boundary and the raw value never outlives the
expression that produced it. It is in no map, no log line, and no memory dump.

The hash must be **keyed** (std's `RandomState`, seeded per process) or it is
theatre: the IPv4 space is small enough to enumerate exhaustively against an
unkeyed hash, so an attacker with the digests would simply recover the
addresses. Keyed and per-process also means digests cannot be correlated
across restarts.

The general rule: when the only operation on a piece of personal data is
comparison, store something that compares equal and reveals nothing else.

### 5.7 Every third-party asset is a disclosure

The page loaded its typeface and icon font from `fonts.googleapis.com`. That is
not a styling decision, it is a data-sharing one: every visitor's browser
announced their IP address, and via the `Referer` the room they were opening,
to a third party — on a server that otherwise goes out of its way to know
nothing about anyone.

An asset you serve yourself is a dependency. An asset the browser fetches from
someone else is a **disclosure**, and it is made by every visitor, not by you.
Self-host it, inline it, or do without it.

The reward for having none is that the CSP stops being an allowlist and starts
being a denial: `font-src 'none'`, no `https://` anywhere in the policy. A
policy that names no external origin is enforcing something much stronger than
one that names two.

### 5.8 Trust boundaries are stated, not assumed

`X-Forwarded-For` is the only place the real client IP appears, because
Cloudflare terminates in front of the container. It is also entirely
attacker-controlled if the container is reached directly. Per-IP limits are
therefore a **courtesy bound**; the per-connection and global ceilings are the
real protection. Documented at `extract_client_ip` so nobody mistakes one for
the other.

### 5.9 `HttpOnly` is not an integrity control

Both identity cookies are `HttpOnly`, which is why the client cannot read them
(§4.1). It is easy to slide from there into treating what comes *back* in the
`Cookie` header as server-issued. It is not. `HttpOnly` keeps a page's script
away from the cookie jar; it does nothing about the person operating the
browser, who can send any header they like with `curl`.

`admit_user` took `animal_name` from the cookie verbatim — so a visitor could
choose their own display name, make it a megabyte long, or take the name of
somebody already in the room, and the server stored it and broadcast it to
everyone. The only reason it was not also an XSS vector is that the current
client happens to render names with `textContent` — a property of one client,
not of the server, which is the same argument already made about `sanitize_reply`
and just as thin.

**The strongest form of validation is a closed set.** An animal name is one of
`ANIMAL_NAMES` or it is not a name; a user id parses as a UUID or it is not an
id. Neither has an escaping bug, a length to bound, or an encoding to get
wrong — the questions do not arise. Reach for a closed set before reaching for a
sanitiser.

### 5.10 A counter that decides admission must never wrap

Every counter compared against a ceiling — connection slots, per-IP counts,
retained bytes — has the property that one unbalanced decrement does not merely
misreport. It wraps to `usize::MAX`, the comparison says "full", and the server
refuses everything for the rest of the process's life, with no traffic to point
at. That is a permanent outage from a single arithmetic slip.

`MemoryTracker::remove_bytes` had guarded against exactly this since it was
written, and its doc comment explained why. The connection counters had the
identical shape and none of the protection, because the reasoning lived in one
function's comment instead of in a helper every counter had to go through.
**When the same reasoning applies to more than one call site, make it a function,
not a comment** — `saturating_dec` in `limits.rs`. Pinned by
`releasing_more_than_was_reserved_cannot_wrap_a_counter`.

### 5.11 Release in a wrapper, not at every exit

The session's teardown used to be the duty of each `return` inside
`handle_websocket`, and one of them — "room vanished between admission and
upgrade" — did not do it, holding that connection's slots until the process
died. This is §5.2 recurring: the same defect class, reintroduced one `return`
at a time, in the function whose comment already described the class.

Ordering fixes admission because admission is straight-line. A session is not:
it has four raced tasks and several early exits, so "remember to release"
is a rule that must hold at every future exit anyone adds. Splitting the body
into an inner function and releasing in the wrapper makes it hold at exits that
do not exist yet:

```rust
pub async fn handle_websocket(...) {
    run_session(...).await;   // every early return lands here
    cleanup_user(...).await;  // exactly one teardown, unreachable to route around
}
```

**Prefer a structure where the invariant cannot be violated over a rule that
must be remembered.** A test can only cover the exits that exist today.

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

### 6.1a A green gate that cannot see the deploy is not a gate

The gate exists to answer one question: *will this deploy?* For most of a day
it answered "yes" and was wrong. `wrangler` requires Node >= 22; both workflows
pinned Node 20. Every push showed fmt, clippy, 600 tests, a release build and
the coverage floor all green — and then the **deploy step**, which runs after
all of it, failed on a version check. Nothing reached the live site, and every
signal a person actually looks at said the commit was fine.

Switching the Worker toolchain to pnpm made it worse before it made it better:
pnpm 11 needs `node:sqlite`, also Node 22, so the failure moved earlier — into
the gate — which is the only reason it was noticed at all.

**A requirement that only the last step enforces is a requirement nothing
checks.** The version now lives once, in `cloudflare/package.json` under
`engines`, `engineStrict` makes the install itself refuse a Node below it, and
`ci_node_version_satisfies_the_toolchain` asserts the workflows agree — so the
mismatch fails in the Rust suite, on a laptop, before any of it is pushed.

### 6.1b The claim that rules are enforced is itself a contract

This document opens by asserting that every rule is enforced by a test. That
claim can rot in two directions, both silently, and both are now checked:

- **Phantom enforcement.** The Enforcing-tests table names a test that has since
  been renamed or deleted. The row reads exactly the same either way, so a rule
  with a dead enforcer looks identical to an enforced one.
  `every_test_the_standards_name_exists`.
- **Undocumented enforcement.** A contract test exists but nothing in
  `CLAUDE.md` or this file mentions it, so what it guards and why lives only in
  the file — one refactor from being lore. `every_contract_test_is_documented`.

Plus the registry's own shape: a watched-lever row without a reopen condition is
the "decided, then forgotten" failure §9.4 exists to prevent, so the table is
machine-checked (`every_parked_decision_records_how_to_reopen_it`).

The pattern is ported from the contract suite on `cameronaaron.com`, which runs
the same three sweeps over a TypeScript project. What travels is not the code —
none of it — but the idea that **the enforcement mechanism needs enforcing too**,
and that an exemption must never outlive its reason. `every_declared_dependency_is_used`
carries the same self-cleaning property: its allow-list of indirectly-used
crates fails if an entry names a crate that is no longer in `Cargo.toml`.

### 6.1c An untestable line is usually a misplaced line

The coverage floor was 95% and the argument for it was that the rest is "the
process shell" — `main`, signal handling. That was true of the *file* and not of
the code. Splitting `startup.rs` out of `main.rs` and taking two things as
parameters instead of reaching for them moved 78 lines from unreachable to
covered:

- `main` is now one line, because a test can never call it — so every line left
  inside it is a line nothing can cover, and it is the only honest exclusion.
- `wait_then_announce` takes the shutdown signal instead of calling
  `ctrl_c()`. A test cannot send itself SIGINT without killing the runner; it
  can hand over a future that has already resolved. Both arms are now covered,
  including the one that handles a *broken* signal handler.
- `main_inner` returns an `ExitCode` instead of calling `std::process::exit`,
  which does not return and would take the test runner with it.
- `log_server_result` is a function rather than a `select!` arm, so the error
  path needs no live server failing mid-flight.

**Injection belongs at the edge, where the untestable thing actually is.** That
is the distinction from the injectable clock in §9.4, which is rejected: a clock
threads through every timing decision in the codebase, while a signal is one
parameter in one function at the boundary of the process.

What is left is genuinely racy — a socket dying between two frames — and is
exempted line by line with reasons, not rounded away.

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

### 6.6 Coverage says a line ran; mutation testing says it mattered

Two different questions, and the second is the one that catches bad tests.

`scripts/coverage.sh` enforces a **line-coverage floor** (currently 95%). The
floor only ratchets up. If coverage drops, the fix is a test, not a smaller
number.

`cargo mutants` edits the source — flips a comparison, replaces a return value —
and fails when the suite still passes. That is the check that catches the class
of test this repo has already shipped: `assert!(result || !result)` had 100%
coverage of the line it exercised and asserted nothing about it.

Run it scoped while iterating (`cargo mutants -f src/room.rs`) and unscoped
periodically; it is a manual sweep, not a commit gate, because a full run takes
far longer than a commit should wait.

**Surviving mutants get classified, not ignored.** The current run of the two
most logic-dense modules leaves six, and each has a reason:

| Survivor | Why it survives |
| --- | --- |
| `while total > peak` → `>=` | **Equivalent.** One redundant compare-and-swap writing the value already there; identical result. |
| `end > 0` → `end >= 0` in `truncate_on_char_boundary` | **Equivalent in practice.** Index 0 is always a char boundary, so the walk always terminates first; the guard is defensive. |
| Four `elapsed() < WINDOW` → `<=` comparisons | Differ only when a duration is *exactly* the threshold. Killing them needs an injected clock threaded through the limiters — a real abstraction bought for a one-nanosecond behavioural difference. Rejected on those terms, recorded in §9.4. |

### 6.6a Deleting a dependency leaves references that still parse

Removing the webfonts took out the `<link>` tags and the network requests, and
`the_client_makes_no_third_party_requests` confirmed it. Two CSS declarations
naming those fonts stayed behind, and both still parsed, still applied, and
still did something — just not the right thing.

`'Roboto Mono', monospace` was harmless; it had been falling back to the
generic for months. The other was not: `#chat:empty::before` set
`content: 'chat_bubble'` in `'Material Icons Round'`. That word is a
**ligature** — with the font it draws a picture, without it the browser renders
the text. Every visitor opening an empty room was shown the literal word
"chat_bubble" at 80px, and no test that checks for network requests could ever
see it, because there was no request to see.

**When you delete a dependency, sweep for what still names it**, not just for
what still fetches it. The absence of a request is not evidence that the
reference is gone. Pinned by `the_page_names_no_font_it_does_not_ship_with`,
which checks every `font-family` resolves to something the machine already has,
and `no_css_content_string_is_an_icon_ligature`.

### 6.6b Mutation testing finds the test you did not think to write

Coverage says a line ran. The full sweep found two gaps that 100%-covered lines
were hiding, and both were the same shape: **a test that reaches a function
without exercising the thing it decides.**

`is_animal_name` replaced with `true` survived the entire suite. The
forged-cookie test looked like it covered this — it asserts an invented name is
replaced by a roster one — but its `user_id` is not a UUID, so the identity was
discarded *before* the name was ever checked. The roster check was covered and
never actually consulted. It needed a cookie with a valid id and a bad name.

`public_message` replaced with `""` also survived: the error taxonomy was tested
for its status codes only, so the body a client is shown could have said
anything, or nothing.

The lesson is not "write more tests". It is that **covering a branch and
exercising its decision are different things**, and only mutation testing tells
them apart.

The sweep also has to be readable to be used: three mutants in `main`,
`shutdown_signal` and `init_tracing` were excluded by patterns like `^main$`,
which match nothing — cargo-mutants matches the whole description
("replace main with ()"). Three permanent unkillable survivors in every report
is how a report stops being read.

### 6.6c Ported from cameronaaron.com — what travels between two codebases

That repository is TypeScript against a static site; this one is a Rust
WebSocket server. **None of the code ports.** Six ideas do, and each became a
sweep here:

| Idea there | Here |
| --- | --- |
| The enforcement mechanism needs enforcing | `every_test_the_standards_name_exists`, `every_contract_test_is_documented`, `every_parked_decision_records_how_to_reopen_it` |
| An exemption must never outlive its reason | `every_declared_dependency_is_used`, `coverage_exemptions_are_justified_and_current`, and the test-only list in `nothing_is_public_only_for_its_own_test` |
| A test that cannot fail is worse than no test | `no_assertion_in_this_suite_is_a_tautology` — found three |
| A doc pointer that no longer resolves is a lie with a green gate | `every_section_reference_resolves`, extended to Rust comments, where most citations live |
| Dead exports hide behind their own tests | `nothing_is_public_only_for_its_own_test` |
| 100% coverage with a short, reasoned exclusion list | `scripts/coverage-exemptions.toml` and its contract |

The one that generalises furthest is the second. Every allow-list in this
repository fails when an entry stops being needed, rather than sitting there
being quietly wrong: the dependency list fails if it names a crate no longer in
`Cargo.toml`, the coverage registry fails if a path disappears or a line count
drifts, the test-only list fails if the function is gone. **An exemption that
cannot expire is a decision nobody will revisit.**

### 6.6d Three things mutation testing keeps finding

**A guard with nothing behind it.** `if freed > 0 { remove_bytes(freed) }` and
`if len > target { retain_newest(target) }` both mutated three ways and survived
— because `remove_bytes(0)` and `retain_newest` at the cap are already no-ops.
The mutants were *equivalent*: the code behaved identically either way. The fix
was not a test, it was deleting the guards (§0.2). A comparison that cannot
change what happens is not a check, it is a place for a bug to hide.

**The log nobody reads.** What is left of that trim guard now gates an `info!`,
and three mutants lived there because no test read the output. An operator reads
the log to understand a running server — which room faded, who was evicted — so
`capturing_logs` installs a scoped subscriber and the log became assertable.
§5.3 already said to shut down *loudly*; this is the same idea applied to
housekeeping.

**A boundary nobody stood on.** Every `>` that mutated to `>=` and survived was
a threshold the tests approached from both sides and never landed on. For
counters that is a test; for durations it needed `cleanup_rooms_at`, which takes
the instant as a parameter — the same edge-injection as the shutdown signal, and
still not the injectable clock §9.4 rejects.

### 6.10 Write down what exists, so removing it is a decision

Three regressions in one afternoon had one cause, and it was not any of the
three: a bulk mechanical edit removed rules it was not aimed at, and **nothing
knew they were supposed to be there.** A visitor reported one. The other two —
a hover-target bridge added an hour earlier, and the whole `.room-fading`
effect — were found by auditing afterwards, which is not a mechanism.

Every sweep written in response caught exactly one *kind* of loss, after it had
already happened once. That is a ratchet, and it is worth having, but it only
ever protects against what has already gone wrong.

`scripts/client-inventory.toml` and `config.rs.inventory` are the general form:
a written record of every element id, stylesheet rule, client method and server
tunable that exists. Adding is free. **Removal fails the gate**, and the failure
names the thing by name.

The point is not that removal is forbidden — it is that removal stops being
something that can happen *by accident*. When the contract fails it is asking a
question: was that deliberate? If yes, delete the line from the manifest in the
same commit, and the diff now says out loud what was removed. If no, you have
just been told about a bug before a visitor was.

A regex over a stylesheet does not know what a rule is for. Neither does a
reviewer reading a four-thousand-line diff. A list does.

### 6.7 Assert on structure, never on a substring a comment can contain

Four tests in this repo have failed because a **comment explaining why
something was removed** necessarily names the thing it removed:
`#chat:empty {`, `user-scalable=no`, `fonts.googleapis.com`, and — most
recently — `content: 'chat_bubble'`, where the sweep written to forbid icon-font
ligatures failed on the comment documenting the ligature it had just removed.
The fix there was `embedded_html_without_comments()`: scan the declarations,
not the prose about them.

A `source.contains("...")` assertion cannot tell an active declaration from
prose about one. Match on the structure that would actually take effect — the
selector at the start of a line, the attribute inside the right tag, the origin
inside a `src=`/`href=`/`url()` — not on the raw string anywhere in the file.

This matters more than it sounds. A test that a comment can break is a test
that gets weakened or deleted the next time someone documents a decision, and
the coverage goes with it.

### 6.8 A boundary is a place to test, not a place to assume

Every mutant killed in the last round was an off-by-one at a threshold: the
memory ceiling admitting exactly the limit, a sweep becoming due strictly after
its interval, an IP banned strictly past the allowance. None of them had a test,
and all of them are the kind of bug that only shows up under the load the limit
exists to handle. When a comparison guards a limit, test the value *at* the
limit and one past it.

### 6.9 Build artifacts are never committed

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
| Owned `OutgoingMessage` in `chat_history` | **Replaced by `Arc`** — measured: copying a full room's history (500 messages, ~2 MB of attachments) took 119 µs *under the room write lock*, against 1.3 µs for the same copy as refcount clones. Total join-path cost 119 µs → 19.6 µs; the remainder is per-viewer reaction resolution | Closed. Reopens only if messages ever become mutable after storage, which would make the sharing unsound |
| `chat_history` as `VecDeque` | **Deferred** — would make front-removal O(1) natively, but `drain(..k)` already made pruning O(n) once per prune, and `Vec` keeps slicing/indexing that ~40 tests use | Reopens if pruning ever moves onto the per-message path, where the constant factor would matter. |
| Per-IP rate limiting as real protection | **Rejected as a security boundary** (§5.5) — `X-Forwarded-For` is attacker-controlled | Reopens if the container stops being reachable except through the Worker, verifiable at the network layer. |
| Sliding-window rate limiter | **Rejected** — needs a timestamp per event; the fixed window's edge behaviour is imperceptible in chat | Reopens if burst abuse is observed crossing window boundaries in production. |
| `cargo-deny` in CI | **Not added** — `cargo audit` and `pnpm audit` now run in CI on every push and weekly, covering advisories on both dependency trees. `cargo-deny` would add licence and duplicate-version policy on top | Add when a licence obligation or a duplicate-version conflict actually bites. Advisories are already covered. |
| Removing the double `ammonia::clean` | **Deferred** — `validate_message` sanitises to detect markup-only messages, then `render_message_html` sanitises the rendered HTML. Two passes over ≤8 KB, microseconds | Reopens if message throughput is ever measured as CPU-bound. |
| AVIF for attachments | **Deferred, unmeasured** — AVIF is ~20-30% smaller than WebP at equal quality, but the constraint is *encoding*, not decoding. Browser `canvas` AVIF encode support is thin where WebP's is universal, and `toDataURL` silently returns PNG for a type it cannot encode — so an AVIF-first attempt would run a full lossless PNG encode of a 1600px image and discard it, on every browser that lacks support, for every attempt in the retry loop. Where AVIF encode does exist it is slow, and `toDataURL` is synchronous. Adding it also needs `image/avif` in `ALLOWED_ATTACHMENT_MIMES` and its magic bytes in `sniff_image_mime`. **No browser was available to measure any of this** | Reopens when the encode path can actually be measured in a browser. The shape it would need: a one-time cached probe on a 1×1 canvas (never a probe per attempt), and async `toBlob` rather than blocking `toDataURL`. Worth revisiting if attachment bytes ever become the binding constraint — they are not today |
| Synchronous `toDataURL` in the attachment encoder | **Known cost, unmeasured** — the encoder tries up to 15 (size, quality) combinations, each a canvas redraw plus a synchronous encode that blocks the main thread. A 128 KB *base64* ceiling is ~96 KB of image, which 1600px will usually miss, so the loop probably runs several times rather than once | Reopens with a browser to measure in. The fix is `toBlob` (async) plus starting the search from a size estimated off the source dimensions instead of always at 1600px. Both are unverifiable from here, and shipping an unmeasured rewrite of a path that currently works is the trade §0.5 warns about |
| Rust nightly / experimental features | **Rejected** — this is a server that holds every room in one process, and nightly has no stability guarantee: a feature can change or vanish between two nights, and the build breaking is the site going down. The features that would actually help are already stable and in use — let-chains (1.88, which the MSRV bump turned on and clippy applied 49 times), inline `const` blocks for the budget assertions, `LazyLock`, async fn in traits. Nothing on nightly addresses a constraint this project actually has: the bottleneck is lock duration and the memory ceiling, neither of which a language feature moves | Reopens for a *specific* feature with a measured win that stable cannot express, pinned to an exact nightly, and only if the container build can be reproduced from it. "It is newer" is not a reason (§0.3) |
| 100% mutation coverage | **Rejected as a target, pursued as a direction** — six mutants survive and are classified in §6.6. Four differ only when a duration is *exactly* its threshold and would need an injectable clock, which is separately rejected because the indirection costs more than the mutants are worth. Killing the last few would mean testing the clock rather than the behaviour | Reopens if a *reachable* mutant appears that is not one of the six classified, which is a real gap rather than an exclusion |
| Load testing | **Never done** — every performance claim here is structural (complexity, lock duration), not empirical throughput | Before raising `MAX_CONCURRENT_USERS` (400) or `MAX_USERS_PER_ROOM` (100). Those numbers are currently unvalidated assumptions, and §0.5 says so out loud. |
| Injectable clock for the limiters | **Rejected** (§6.6) — would kill four surviving mutants that differ only when a duration is *exactly* its threshold | Reopens if a timing bug is ever observed at a limit boundary in production, or if the limiters need testable time for another reason. |
| 100% line coverage | **Now the target, with a reasoned exemption list** — 98.18%, every module at 100% except `session.rs` (352/373). `main.rs` was reduced to an entry point and `startup.rs` split out of it so the exclusion is honest rather than a hiding place; the test file is excluded the way `*.test.ts` is on cameronaaron.com. The 21 remaining lines are listed individually in scripts/coverage-exemptions.toml | Reopens for any of those 21 that becomes reachable — the registry's line count is asserted, so it fails if the number moves either way. The old entry read: **Not the target** — the floor is 95% and ratchets. The remainder is the process shell (`main`, signal handling) and socket-failure paths inside the four connection tasks, which need a socket to fail at an exact instant | Reopens for any *reachable* branch: those are gaps, not exclusions. Chasing the rest would mean flaky timing tests, which §6.4 rules out as worse than none. |
| Edge-caching the room pages | **Rejected** — the page is per-room and sets identity cookies, so a shared cache would serve one visitor's `Set-Cookie` to another | Reopens only if identity moves entirely to the socket and the page becomes byte-identical for all visitors. |

---

## 10. The client law

The client is one HTML file and one JavaScript file with no framework, no build
step and no tests of its own beyond assertions over the shipped bytes. That
makes a small number of rules load-bearing.

### 10.1 Never derive state you can read

The client kept a running `messageCount` to decide when to trim rendered
history. System messages incremented it and then removed themselves from the
DOM eight seconds later without decrementing it, so on a busy room the counter
drifted far above the number of nodes actually present — and began deleting
**live chat messages** that were nowhere near the limit. Messages silently
vanishing from a conversation is close to the worst failure a chat client has,
and it was invisible because the page was right and the counter was wrong.

The fix is not a matching decrement. It is to stop keeping the number at all and
count `querySelectorAll('.message')` at the point of use. A derived value that
can drift from its source will.

### 10.2 Never fight the browser for the scroll position

Three separate mechanisms were moving the chat's scroll position at once:
`scroll-behavior: smooth` animating every programmatic jump, the client pinning
to the bottom on each message, and the browser's own scroll anchoring adjusting
for inserted content. During history replay — one frame per message — they
produced visible sliding, and the scroll events from the client's *own* writes
were read back as "the user scrolled up", switching auto-scroll off partway
through. That race is why the symptom was intermittent.

The rules that came out of it:

- Programmatic scrolling is **instant**; smooth is opt-in per call, for
  scrolling the user asked for (`scrollToMessage`).
- `overflow-anchor: none` where the client manages the scroll itself.
- Scroll events caused by the client's own writes are ignored
  (`programmaticScroll`), and so is everything during history replay
  (`isLoadingHistory`).
- The replay is bracketed by a protocol frame — `ReconnectToken` marks its end —
  with a timeout failsafe, because a missing frame must never leave the UI
  permanently degraded.

### 10.3 A state that only exists sometimes must not change the layout

`#chat:empty` used to set `justify-content: center` on the container. The moment
the first history message arrived the selector stopped matching and the whole
column snapped from centred to top-aligned: a visible reorientation on every
load of a room with any history.

An empty state, a loading state, or anything else that appears only sometimes is
an **overlay**, positioned out of flow. It must not be able to change the layout
of the content that replaces it.

### 10.4 Escape for the sink you are writing to

`textContent` does not interpret markup; passing it escaped text double-escapes,
so an animal name containing `&` displayed as `&amp;`. `innerHTML` does
interpret markup and must always receive escaped values. The client had
`escapeHtml` on both, which was simultaneously a display bug everywhere and no
extra safety anywhere. One escape helper, used only where markup is built as a
string.

### 10.5 Accessibility is not negotiable for convenience

`user-scalable=no` was blocking pinch-zoom, which fails WCAG 1.4.4. The reason
people set it — stopping iOS zooming when an input is focused — is solved
properly by a 16px input font, which this client already had. The workaround had
outlived the problem it was for.

---

### 10.6 A selector styled twice is a value undone somewhere else

Thirteen selectors were defined in two places at once — `#chat`, `.message`,
`.input-container`, `.reaction-bar` among them. CSS resolves that by letting the
later block win, silently, and both blocks read as though they apply. A padding
set deliberately in one place was undone thirteen hundred lines later by a rule
added during a different change.

That is what "weirdly proportioned" looks like from the inside: no single wrong
value, just two right ones disagreeing. Merged into one rule each, later
declarations winning exactly as the cascade had them, and
`no_css_selector_is_defined_twice` keeps it that way.

The related shape: **the conversation is a column, not the window.** `#chat`
spanned the full width with bubbles capped at 560px, so on a wide monitor one
message sat against the left edge and the next against the right with a metre of
nothing between. The composer is laid out against the same width, or it drifts
away from the messages it belongs to.

### 10.8 A class the renderer sets and the stylesheet never mentions

The client builds its DOM in JavaScript, so the two halves of every element
live in different files with nothing connecting them. A class that is set and
never styled renders as an unstyled box: no error, no warning, nothing in the
console — it simply looks wrong.

It happened during the iMessage rework. A sweep removing the `.message*` rules
matched `.message-image-faded` too, because `\b` after "message" matches at a
hyphen, and the placeholder shown for a picture that has aged out of the room's
budget silently lost its box. `every_class_the_client_renders_is_styled` reads
the classes out of `client.js` and checks each against the stylesheet.

The same edit is where §10.6 came from: the removal was necessary *because*
thirteen selectors had been defined twice. Bulk edits to a stylesheet are where
both of these failures come from, which is the argument for having them
machine-checked rather than reviewed.

### 10.9 A rule that shows something implies a rule that hides it

`.reply-preview.active { display: flex }` says "visible when active", which
means nothing unless the base `.reply-preview` is hidden. A bulk sweep removed
the base rule and nothing noticed: the class was still mentioned throughout the
stylesheet, the page still parsed, and the reply preview — with its placeholder
text baked into the markup — sat above the composer permanently, telling every
visitor "Replying to / Message text…". A reader reported it; no test could
have, because §10.8's sweep only asks whether a class is *mentioned*.

This is §10.3 stated the other way round. That one says a sometimes-present
thing must not change the layout; this says a sometimes-*visible* thing must
actually start invisible.

The wider lesson is about the edit rather than the rule. Three separate
regressions came out of one bulk stylesheet sweep — a duplicated selector, a
class matched by accident through `\b` at a hyphen, and this. **A regex over a
stylesheet does not know what a rule is for.** Each got its own contract
afterwards, which is the right ratchet, but the cheaper lesson is that
mechanical edits to CSS need mechanical verification in the same commit, not
after somebody notices.

### 10.7 A control that appears on hover must survive being aimed at

The reaction bar is `position: fixed` and a sibling of `#chat`, so moving the
pointer towards it *leaves* the chat. Hiding on that event made the buttons
appear and vanish the instant you aimed at them — visible, documented, and
completely unclickable with a mouse.

Three things are needed, and any one missing restores the bug: the hide is
delayed, arriving on the control cancels it, and leaving the container *onto*
the control is not leaving. A fourth almost as subtle: the bar must not be
rebuilt while it is already showing, or the button under the cursor is replaced
between `pointerdown` and `pointerup` and the click lands on nothing.

**Anything that appears on hover and must then be clicked has a gap to cross.**
Either bridge it or forgive it.

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
