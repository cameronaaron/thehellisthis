# Engineering Standards: infinite-chat (thehellisthis.com)

The correctness and quality bar for this codebase. **Every rule here is
enforced by a test in `src/tests/`.** A rule that matters and has no test gets
one first; a standard that isn't executable is a suggestion.

> **When a test fails, fix the source, not the test.** The tests encode
> deliberate decisions, usually made after a real defect. Weakening one to
> make code pass inverts the system.

The code is held to the standard of software that runs unattended:

- **A found defect gets a permanent test in the same commit that fixes it.**
  The ratchet (§6) only tightens; a bug that recurs is a process failure.
- **A found defect becomes a sweep for its whole class** (§6.3).
- **Nothing ships on "should work."** Every claim here is backed by a
  measurement, a reproduction or a test.
- **Blast radius is a design input** (§5). One process holds every room, so a
  mistake disconnects everyone, not one user.
- **"What if the client is hostile, slow or gone" is the first question.**
  Every socket input is attacker-controlled, any client can vanish mid-frame,
  and everyone shares the room lock. Working for four healthy clients on
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
| §5.5 CSP | `every_response_carries_security_headers`, `client_script_is_external_so_csp_can_forbid_inline`, `style_src_names_the_pages_inline_stylesheet_by_hash_not_by_unsafe_inline`, `the_page_has_exactly_one_style_block` |
| §5.6 Privacy | `client_addresses_are_hashed_not_stored_in_the_clear`, `the_upgrade_path_hashes_the_address_at_the_boundary` |
| §5.7 No third parties | `the_client_makes_no_third_party_requests`, `the_policy_permits_no_third_party_origins` |
| §0.4 Cover traffic is not called DP | `the_client_claims_no_differential_privacy` |
| AGPL-3.0 section 13 source offer | `the_page_links_to_its_source_code` |
| §5.8 Trust boundary | `client_address_prefers_the_header_cloudflare_sets`, `worker_forwards_the_client_address` |
| §5.9a Cookies must be storable | `identity_cookies_are_secure_everywhere_except_loopback`, `reconnecting_with_the_handshake_cookies_keeps_the_same_identity`, `a_reconnect_does_not_announce_a_departure_and_an_arrival`, `without_cookies_every_reconnect_is_a_different_person` |
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
| §9.1b Co-dependent assets can't cache independently | `nova_assets_revalidate_on_every_request_instead_of_going_stale`, `app_js_is_immutably_cacheable_and_revalidates` |
| nova crypto (X3DH, group ratchet, RLN, MPC/FROST — trust model in `session/nova*.rs`'s own module docs, not a numbered law here) | `two_clients_handshake_and_exchange_a_sealed_message`, `a_third_client_joining_mid_session_can_decrypt_a_later_broadcast`, `a_disconnected_member_is_cryptographically_excluded`, `nova_rejects_a_plaintext_message_sent_outside_the_sealed_wrapper`, `nova_rln_anonymous_post_and_double_post_slashing`, `nova_dummy_frames_are_discarded_without_a_trace`, `nova_mpc_demo_uses_a_real_distributed_operator_quorum`, `a_restarted_operator_fails_clearly_instead_of_hanging`, `complaint_is_valid_only_when_the_share_genuinely_fails_verification` |
| §6.10 Removal is a decision | `the_shipped_client_still_contains_everything_it_did`, `the_servers_public_surface_still_exists` |
| §10.10 The roster is asked for | `the_client_asks_for_the_roster_rather_than_being_sent_it`, `the_navigation_bar_keeps_what_the_client_drives`, `the_roster_names_everyone_connected_in_a_stable_order`, `requesting_the_roster_answers_with_the_current_names`, `roster_requests_are_throttled` |
| §10.9 Show implies hide | `every_conditional_display_rule_has_a_base_that_hides_it` |
| §10.8 Rendered classes are styled | `every_class_the_client_renders_is_styled` |
| §10.11 Ids identify, classes style | `stylesheet_ids_do_not_carry_layout`, `no_rule_styles_something_the_page_never_renders` |
| §10.6 One rule per selector | `no_css_selector_is_defined_twice`, `the_conversation_is_a_readable_centred_column` |
| §10.7 Hover targets are reachable | `the_reaction_bar_can_actually_be_clicked` |
| §10 Client | `empty_chat_placeholder_does_not_alter_container_layout`, `rendered_history_is_trimmed_from_the_dom_not_a_counter`, `the_page_does_not_block_pinch_zoom`, `images_reserve_their_space_before_they_load` |
| Attachments | `an_attachment_must_be_the_image_type_it_claims_to_be`, `svg_is_not_an_allowed_attachment_type`, `a_rooms_oldest_images_fade_once_it_is_over_its_attachment_budget`, `images_are_re_encoded_rather_than_sent_as_picked` |
| Reactions | `reacting_twice_with_the_same_emoji_removes_the_reaction`, `reactions_never_outlive_the_messages_they_belong_to`, `a_reaction_must_be_on_the_roster` |
| §6.1 The gate answers "will this deploy" | `ci_node_version_satisfies_the_toolchain`, `the_workflows_install_with_the_lockfile_that_exists`, `ci_holds_no_deploy_credential_and_does_not_deploy`, `these_images_track_latest_on_purpose` |
| Startup lifecycle | `run_returns_cleanly_when_its_shutdown_fires`, `the_process_exit_code_reports_a_clean_stop`, `the_process_exit_code_reports_a_failed_bind`, `the_shutdown_sequence_waits_for_its_signal`, `a_server_error_is_reported_and_a_clean_stop_is_not_an_error`, `binding_a_port_already_in_use_is_an_error_not_a_panic`, `run_serves_until_it_is_shut_down`, `shutting_down_announces_departures_and_clears_the_rooms` |
| Constraint #12 Frontend/backend agreement | `client_attachment_ceiling_matches_the_server`, `client_reaction_roster_matches_the_server`, `the_node_types_match_the_node_ci_installs`, `every_mirrored_constant_matches_its_source_of_truth` |
| §5.12 The log is an interface | `every_admission_says_what_became_of_the_identity`, `a_departure_is_logged_with_the_room_it_leaves`, `a_refused_connection_says_which_ceiling_refused_it`, `a_full_room_says_so_when_it_refuses`, `a_normal_session_reads_as_one_arrival_and_one_departure` |
| Operational log is a contract | `housekeeping_reports_a_trim_only_when_it_trims`, `housekeeping_reports_the_rooms_it_deletes` |
| §6.6 Boundaries a mutant can move | `the_script_version_is_the_fnv1a_hash_it_claims_to_be`, `a_room_name_at_either_length_boundary_is_accepted`, `an_existing_room_is_served_even_when_the_server_is_at_its_room_cap`, `housekeeping_leaves_rooms_alone_until_the_gc_interval_has_elapsed`, `the_process_memory_ceiling_trims_only_once_it_is_exceeded`, `a_quoted_reply_is_cut_at_the_limit_and_never_mid_character`, `an_attachment_exactly_at_its_ceilings_is_accepted`, `fading_pictures_stops_as_soon_as_the_room_is_back_under_its_budget`, `a_room_is_cleaned_only_once_it_is_past_the_soft_memory_limit`, `an_entry_exactly_at_its_retention_age_is_swept` |
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
| Container entry points | `docker_entry_points_share_the_production_recipe`, `scripts/smoke-container.sh` |
| Manual release checks | `manual_deploy_runs_release_checks_before_publishing`, `smoke_build_failure_never_signals_an_unstarted_server` |
| Dead webfonts | `the_page_names_no_font_it_does_not_ship_with`, `no_css_content_string_is_an_icon_ligature` |

---

---

## 0. The first-principles doctrine

Everything after §0 is a conclusion; this is the method that produced it and
that every change uses. The rules are right because they were derived from
ground truth, and when a better derivation arrives with evidence, the rule
changes and the ratchet locks the new floor.

**Reason from the mechanics, not the convention.** "How chat servers do it"
imports someone else's constraints with their solution. Ask what is true here
(how long this lock is held, what a message allocates, the container's memory
limit, what the client can observe) and what those facts permit. A best
practice is a cached answer to someone else's question.

### 0.1 "Impossible" is a measurement you haven't taken yet

Most "can't be done" is one of three things:

1. **Unmeasured**: nobody profiled it.
2. **An unquestioned requirement**: the expensive thing shouldn't exist
   (§0.2 step 1).
3. **Expensive, not impossible**: the decision is whether the win is worth
   the stated price.

The real walls are physical (round-trip time, the container's memory
ceiling, one process can't hold two independent copies of a room), and you
prove you've hit one with a number.

### 0.2 The algorithm, strictly in order

Most waste is optimising, or automating, something that should have been
deleted two steps earlier.

1. **Question the requirement.** "The error type needs a variant per
   category," "rooms need persistence," "per-IP limits protect us" are claims
   to interrogate, including ones from senior sources.
2. **Delete the part or the process.** If you never add back about 10% of
   what you removed, you didn't delete enough. Here: four dependencies
   declared and never used (`metrics`, `metrics-exporter-prometheus`,
   `async-trait`, `hyper`), three `ChatError` variants nothing constructed, an
   unreachable branch in `room_handler`, and `lazy_static` (now
   `std::sync::LazyLock`).
3. **Simplify or optimise** what survived step 2.
4. **Accelerate**, once it is simple and correct.
5. **Automate**, last. A test pinning wrong behaviour is worse than none.

### 0.3 Novelty requires evidence

A new approach ships when it beats the incumbent on a stated measure. One
that loses is deleted and recorded, so the next session doesn't relitigate
it; §9.4 is the registry.

### 0.4 Say what is true

Report what happened, not what was hoped; say what is unverified or skipped.
The README claimed `HttpOnly` cookies for months while the code set no such
flag, and a "~7.5 MB optimized release binary" with no release profile.
Documentation describing intent rather than reality is a defect as severe as
a code defect, because the next engineer reasons from it.

### 0.5 Validate the instrument before trusting the measurement

A surprising number is more often a broken ruler than a broken system.

- **A passing test proves less if it asserts a tautology.**
  `assert!(result || !result)` asserted nothing about `should_gc`; clippy's
  `overly_complex_bool_expr` found it, not a reviewer.
- **A wall-clock timing test measures the CI runner.** Assert the property
  (`should_gc` refuses a second sweep), not the duration.
- **Localhost load numbers aren't load numbers**: one loopback, no latency,
  loss or TLS, versus real clients arriving through a Worker.
- **Build time isn't runtime.** LTO and `codegen-units = 1` made builds
  slower and the binary smaller (5.4 MB); neither says how fast a message is
  delivered.

---

## 1. The complexity doctrine

**Per-message and per-event work is O(1). Work proportional to history or
user count runs rarely, and never on the message path.** Everything on that
path is paid by every user on every message, under a lock the whole room
contends on (§2).

### 1.1 The classification

| Path | Frequency | Budget |
| --- | --- | --- |
| Message receive, validate, render, broadcast | every message | O(1) |
| Typing / read-receipt events | up to 5/s per user | O(1) |
| Join / admission | once per connection | O(history) acceptable |
| Housekeeping sweep | once per 60s | O(rooms × history) acceptable |

A bound that runs only on the join path is not a bound: nothing obliges
anyone to join. History trimming lived there, so a busy room with no new
joiners grew without limit and, since the byte ceiling is process-wide, could
make every room drop messages. **Anything that must always hold belongs on
the timer**; the join path may repeat it to pay early.

### 1.2 Never shift from the front of a `Vec`

`prune_old_messages` called `Vec::remove(0)` in a loop: O(k·n) for k of n
messages, under the room write lock. Count first, then drain once:

```rust
let mut drop_count = 0;
for msg in &self.chat_history {
    if removed_size >= needed_space { break; }
    removed_size += msg.estimate_size();
    drop_count += 1;
}
self.chat_history.drain(..drop_count);
```

### 1.3 Never copy a collection to delete from it

`cleanup_messages` cloned every surviving message into a new `Vec` to remove
a handful. `retain` works in place.

### 1.4 Build the lookup set once, not per candidate

`assign_animal` scanned every user for every candidate name, about 25,000
string comparisons under the write lock in a full room. It now builds a
`HashSet` of taken names once (O(users)), then scans the pool with O(1)
lookups. Pinned by `assign_animal_never_repeats_a_connected_name`.

### 1.4a Share what is immutable; copy only what differs

A stored message never changes, so `chat_history` holds
`Arc<OutgoingMessage>` and the join path clones refcounts. Copying a full
room's history (500 messages, about 2 MB of attachments) measured **119 µs**
under the room write lock, charged to every user in every room; as `Arc`
clones it is **1.3 µs**. It was the largest lock hold in the server.

The per-viewer part can't be shared: whether *you* reacted differs per
recipient. That's why `reactions` isn't a field of `OutgoingMessage` (filling
it would force a per-viewer copy); it travels alongside and is flattened
onto the wire so the frame matches a live one. **Split a structure between
what is shared and what differs, and share the first half.**

### 1.4b Derive the answer; don't keep a second copy of it

`available_animals` was a free list that didn't balance: names that never
came out of that room's list (from a cookie, or a `guest_N` fallback) were
pushed back on reclaim, so the pool grew and filled with non-roster strings.
Whether a name is in use was already derived from `users` for the `taken` set,
so the free list was never load-bearing; only the rotation order was. The
pool is now a rotation cursor with fixed membership: every drawn name goes to
the back, and the pool is always exactly the roster.

Same shape as §10.1 (`messageCount` drifting from the DOM) and §3.4. **A
derived quantity stored separately will disagree with itself.** Keep a copy
only where recomputing is genuinely too expensive, and say so.

### 1.4c Encode a broadcast once; every receiver shares the bytes

§1.4a's shape was still on the broadcast path, found by checking whether the
O(1) claim held everywhere (§0.5). `RoomState::sender` carried the unencoded
`OutgoingEvent`, and `broadcast::Receiver::recv` clones its value per
receiver, so every connection's `forward_broadcasts` re-cloned the payload
and re-ran `serde_json::to_string` on identical output. In a full room
(`MAX_USERS_PER_ROOM` = 100) sending a 128 KiB image, that was up to a
hundred clones and serialisations of the same payload, sequentially, on the
server's one thread (`main.rs` runs `current_thread`).

`sender` now carries `Arc<str>`, the frame encoded once by
`encode_broadcast`; `recv()` bumps a refcount and `forward_broadcasts` writes
the shared bytes. Verified live: three connections, one message, three
identical frames. **The expensive part of "the same data to everyone"
happens once.**

### 1.4d Derive "empty" from what is stored, not a parallel check

`validate_message` ran `ammonia::clean` over the raw text just to judge
emptiness and length, discarded the result, and `render_message_html`
sanitised the rendered HTML again. The two could disagree: `<div></div>` or
`<img src=x onerror=alert(1)>` survived the raw check (ammonia keeps some
tags, minus dangerous attributes), but comrak, which runs first in the real
pipeline, omits block-opening raw HTML entirely. The message was stored and
broadcast as a blank bubble, with no error and no record.

`validate_and_render_message` renders and sanitises once and derives "empty"
from that output, the value that is stored. Same instinct as §1.4b, except
these copies disagreed on their first run.

### 1.5 Linear work belongs where it is paid once

Trimming history to `MAX_MESSAGES_PER_ROOM` moves the whole `Vec`, so it runs
on the join path and in housekeeping, not per message. A
`HISTORY_TRIM_SLACK` of 100 once gated the join-path trim; it did nothing,
since the next line trimmed unconditionally. Six surviving mutants in that
condition said so (§6.6d). **A threshold whose two sides can't be told apart
protects nothing.**

### 1.6 Prefer the const over the allocation

`ANIMAL_NAMES` is a `&'static [&'static str]` in the binary, not a
`Vec<String>` per room; only each room's order is state.

`routes::page::room_handler` returned `impl IntoResponse`, which must be one
concrete type, so the happy path matched the rejection branches'
`Html<String>` by cloning the ~70 KiB `CLIENT_PAGE` on every request.
Returning `Response` and converting each branch with `.into_response()`
serves it by reference again. Small and per request, but the same mistake: a
type constraint forcing an allocation the value didn't need.

---

## 2. The lock-discipline law

`AppState::rooms` is one `RwLock<HashMap<String, RoomState>>` that every
user in every room contends on. **Lock duration, not CPU, is this server's
scaling limit.**

### 2.1 Never hold the room lock across an await on the socket

A send completes when the client is ready, which on a phone on a train can be
seconds. Clone what you need under the lock, drop the guard, then send. The
history clone is an allocation bought to shorten a critical section, and
stays.

### 2.1a A synchronous call is a lock the scheduler can't see

`main.rs` runs Tokio's `current_thread` runtime: every room, connection,
housekeeping loop, heartbeat and ping share one OS thread, which only
switches at an `.await`. `apply_client_event_at` rendered messages (comrak
then ammonia over up to `MAX_RENDERED_MESSAGE_LEN` chars) inline, so a slow
render delayed every connection's heartbeat and ping: §2.1's failure, caused
by CPU instead of a slow client.

`render_off_thread` moves rendering to Tokio's blocking pool, which the OS
schedules preemptively, so the async thread stays free. It does **not**
release the room write lock during the wait. `current_thread` was chosen for
a container with 1/16 vCPU (`cloudflare/wrangler.jsonc`), where a
multi-threaded runtime would mostly contend for CPU it doesn't have, and
`state.rooms` already serialised other rooms' events. What this buys back is
the work that doesn't need that lock (pings, other receive loops).

It is generic over the closure, like the connection tasks (§6.1c), so the
failure path (a panicking task, or shutdown mid-render) is a value
`render_off_thread_turns_a_panic_into_none` produces directly, and
`resolve_render` folds it into the rejection an invalid message already gets.

### 2.2 Take the lock once, decide everything, release

`cleanup_rooms` does its whole pass under one write lock. Splitting into read
and write passes would mean re-deriving each decision after the map may have
changed: more code, a race, and no less contention for a lock held tens of
microseconds.

### 2.3 The liveness probe never takes the write lock

The container runtime polls `/health` to decide whether to kill the instance.
If it could block behind the write lock, a busy server would look dead and be
restarted: load turned into an outage.

### 2.4 Check cheaply before locking

`AppState::cleanup` calls `should_gc()` (two atomics) and returns before
taking the write lock when there is nothing to do.

---

## 3. The memory-ceiling law

There is no database: every message lives in RAM in a container with a hard
memory limit. **Exceeding it is an OOM kill, which disconnects every user in
every room.**

### 3.1 Refuse rather than grow

`MemoryTracker::add_bytes` returns `false` and reserves nothing when a
message would breach the ceiling. Dropping one message beats losing every
session.

### 3.2 Over-estimate, never under-estimate

`ESTIMATED_MESSAGE_SIZE` (1 KB) is added to each message's accounted size on
top of its string lengths, for allocator and `Vec` overhead the lengths can't
see. Under-counting means the process thinks it is within budget while the
kernel disagrees, and the kernel wins.

### 3.3 Every counter is saturating

`remove_bytes` uses `saturating_sub`: an underflow would read as `usize::MAX`,
permanently full while idle. Likewise `should_gc`, where a backwards clock
step (NTP, container migration) would otherwise panic in debug and wedge GC
in release.

### 3.4 Recompute where it is already linear; subtract where it isn't

`retain_newest` recomputes the room's byte total from its history instead of
subtracting. It is already O(history), and recomputing self-heals any
accounting slip elsewhere at the next trim. On the message path, where that
would be O(n) per message, `prune_old_messages` subtracts. The rule is
"self-heal wherever it is free."

### 3.5 Every unbounded map needs an eviction path

`ConnectionPool::ip_counters` is keyed by client address, which an attacker
influences. `cleanup_stale` drops entries older than `IP_COUNTER_RETENTION`,
so the map is bounded by recent clients. Any map keyed by client-supplied
data needs the same.

**Checking expiry on read is not eviction.** `SecurityManager`'s `check_ip`
tested a ban's age on every read but never removed it, and suspicion records
were never removed at all: the entries' effect was bounded, not their
existence. The expiry constant in the code looked like it was doing the work.

Every such map is swept from one place, `AppState::cleanup`, so a map without
a sweep is visible at one line. Pinned by
`every_client_keyed_map_is_swept_by_housekeeping`.

**Eviction must not evict something live.** `ip_counters` entries were
stamped when a connection was added, so any session outlasting the retention
window had its counter swept away, silently lifting that address's limit. A
counter still counting is not stale at any age; age only decides for entries
at zero.

### 3.6 Bound what you store, not just what you accept

`MAX_MESSAGE_LEN` bounds the Markdown a user sends, but history stores and
broadcasts the rendered HTML, and rendering amplifies: 7.2x for `[a](b)`
repeated to the cap (8 KB in, 57 KB out). The input cap bounded nothing that
costs memory or bandwidth.

When input is transformed before being kept, the ceiling goes on the output.
`MAX_RENDERED_MESSAGE_LEN` is 5x the input cap because escaping expands by at
most 5x (`&` to `&amp;`; measured 8,000 to 40,000), so the escaped-plain-text
fallback always fits. A fallback that can breach the limit isn't one.

---

## 4. The protocol law

`protocol.rs` is a contract with a program already running in someone's
browser. **A protocol change is a client change, in the same commit.**

### 4.1 Tell the client what it needs; never make it guess

The client once inferred its identity by assuming the first `UserJoined` was
itself (wrong when two people connect at once) or by parsing
`document.cookie` (which kept the cookies from being `HttpOnly` on a server
that turns user Markdown into HTML). One frame, `Welcome { user_id,
animal_name }` sent first, deleted both heuristics. **A guess in a client is
usually a missing field in a protocol.**

### 4.2 Frame ordering is part of the contract

`Welcome`, history, `ReconnectToken`, live traffic. Telling one's own
messages apart depends on `Welcome` preceding history, so the order is
tested (`first_frame_is_welcome_with_this_users_identity`).

### 4.3 Two halves of one decision get one test

`HttpOnly` cookies and the `Welcome` frame are one change; reverting either
alone makes the client silently render the user's messages as someone
else's. `client_takes_identity_from_welcome_frame_not_cookies` asserts both,
including the absence of `document.cookie` parsing in the shipped client.

### 4.4 Sanitise the representation the browser receives

`markdown_to_html`, then `ammonia::clean`. The generated HTML is the only
representation a browser sees, so that is what gets sanitised.

### 4.5 The error taxonomy describes failures that happen

`ChatError` had `RateLimited`, `ConnectionError` and `RoomError`, which only
their own tests constructed. A taxonomy of impossible failures implies
capability that doesn't exist and reports coverage of unreachable code.
Deleted; add a variant when a real call site returns it.

---

## 5. The failure-blast-radius law

One process holds every room. Design for what a failure takes with it.

### 5.1 Unwind, do not abort

`panic = "abort"` is deliberately not set. Tokio confines a panic to its
task, so one connection dies and everyone else continues; under `abort` the
whole process would. A smaller, marginally faster binary isn't worth turning
a per-connection failure into a total one.

### 5.2 Every reservation is released on every exit path

Admission once incremented the per-IP and global counters and then ran
checks that could fail and return without releasing them, so every rejection
leaked a slot and a server that had refused enough connections refused all
of them while idle.

The structural fix is ordering: every fallible check first, mutating nothing;
then the reservations; and the one fallible step that must come after
releases explicitly. Pinned by
`rejected_upgrade_releases_its_connection_slot`, which also checks a good
connection is still accepted afterwards.

### 5.3 Shut down loudly

Cloudflare stops containers with SIGINT, and `AppState::shutdown` announces
`UserLeft` for everyone first, so a scale-down reads as people leaving, not
the app breaking. Best effort: the grace period is short.

### 5.4 Teardown must be idempotent under reconnection

`cleanup_user` compares `connection_id` first. A user who reconnected before
the old session's teardown ran already has a newer live connection, which
must not be marked disconnected.

### 5.5 The browser gets a policy, not just sanitised output

Sanitising is the first defence for user HTML; the CSP is the one that holds
if it is bypassed. `script-src 'self'` is the directive that matters, and it
is why the client's script is `/app.js`: while it was inline, a working
policy had to allow `'unsafe-inline'`, exactly what an injected `<script>`
needs. **A CSP that permits inline script on a page rendering user HTML is
decoration.**

The server previously sent no security headers. Cloudflare adds none to a
Worker-proxied origin, so "the CDN handles it" was an unchecked assumption
(§0.5).

### 5.6 Store the comparison, not the identity

The server kept every visitor's IP in the connection pool and ban list and
logged it on upgrade, though all of them only compare for equality. The
address is now hashed at the boundary and the raw value never outlives that
expression: it's in no map, log or memory dump.

The hash is keyed (std `RandomState`, per process), because IPv4 is small
enough to enumerate against an unkeyed hash; per-process keys also stop
correlation across restarts. **When the only operation on personal data is
comparison, store something that compares equal and reveals nothing else.**

### 5.7 Every third-party asset is a disclosure

The page loaded its fonts from `fonts.googleapis.com`, so every visitor's
browser told a third party its IP and, via `Referer`, the room it was
opening, on a server that otherwise avoids knowing anything about anyone. An
asset you serve is a dependency; one the browser fetches from someone else is
a disclosure made by every visitor. Self-host it, inline it or drop it.

With none, the CSP becomes a denial (`font-src 'none'`, no `https://`
anywhere), which enforces far more than an allowlist of two origins.

### 5.8 Trust boundaries are stated, not assumed

Cloudflare terminates in front of the container and sets `CF-Connecting-IP`
(and the Worker fills `X-Forwarded-For`), so those headers are the only place
the real client address appears. Both are attacker-controlled if the
container is reached directly. Per-IP limits are therefore a courtesy bound;
the per-connection and global ceilings are the real protection. Documented
at `extract_client_ip`.

### 5.9 `HttpOnly` is not an integrity control

`HttpOnly` keeps page script out of the cookie jar; it does nothing about the
person sending any `Cookie` header with `curl`. `admit_user` took
`animal_name` from the cookie verbatim, so a visitor could choose their name,
make it a megabyte long or take someone else's, and the server broadcast it.
It wasn't also XSS only because this client renders names with
`textContent`: a property of one client, not the server.

**The strongest validation is a closed set.** A name is on `ANIMAL_NAMES` or
it isn't; an id parses as a UUID or it doesn't. No escaping bug, length or
encoding to get wrong. Reach for a closed set before a sanitiser.

### 5.9a A cookie the browser refuses to store is a cookie you don't have

`HttpOnly; SameSite=Strict; Secure` is right in production, but browsers won't
store a `Secure` cookie over plain http (Safari refuses even on localhost).
The symptom was a room repeating "skink left, stinks joined": with nothing
stored, `admit_user` minted a new identity on every reconnect, and nothing
logged an error. `Secure` is now set everywhere but loopback. **A security
attribute has a storage consequence**; check it against where the cookie must
survive.

### 5.9b A valid cookie's id must survive a room it hasn't seen

The cookie is `Path=/`: one identity for every room. `admit_user`'s outcomes
were `reclaimed` (this room knows the id), `recognised` (good cookie, new
room) and `fresh` (no usable cookie), and `recognised` minted a new
`Uuid::new_v4()`, relying on `claim_animal` to restore the same display name.

The client knows a message is its own only by comparing its `user_id` with
the `Welcome` frame's (constraint #2). So after moving to a room, or
returning once their entry had aged out, a visitor's older messages there
stopped matching and rendered as someone else's under their own name. Every
step was correct alone; the bug was `recognised` behaving like `fresh` when
the cookie's scope said it was nearly `reclaimed`. `recognised` now keeps
`c.user_id` and only draws the name through `claim_animal`; `fresh` is the
only branch that mints an id. Pinned by
`a_returning_visitor_keeps_a_roster_name_that_is_free` and
`every_admission_says_what_became_of_the_identity`.

### 5.9c Reclaiming an identity must re-check the name it restores

`names_in_use` (and so `claim_animal`/`assign_animal`) counts a name as taken
only while its holder is connected; a disconnected user's entry lingers for
`DISCONNECTED_USER_RETENTION` so they can reclaim the identity, but the name
is free for others at once. `reclaimed` restored the stored name
unconditionally, so if someone had drawn it meanwhile, two connected users
shared one name (constraint #9). Each piece was right; reclaiming just never
asked whether the name was still safe. It now checks
`RoomState::name_taken_by_another_connected_user` and draws a new name via
`claim_animal(None)` if needed. A changed name on a stale reconnect is
cosmetic; two people with one name is someone mistaken for you. Pinned by
`reclaiming_never_hands_back_a_name_someone_else_now_holds`.

### 5.9d Reclaiming an identity must close the connection it reclaims from

When a second connection arrives under the same identity (a second tab),
`admit_user` moves `connection_id` to it, but told the first connection
nothing. Its four tasks kept running: `forward_broadcasts` delivered room
events, `send_pings` kept it alive, and `touch_and_check_idle` refreshed
`last_heartbeat` by `user_id`, unable to see a newer connection owned it. A
socket the room no longer counted stayed open indefinitely, receiving
everything. Found by reproducing it (two real connections, one left alone,
watched): `OPEN` forever with nothing in the log (§0.5).

Fixed like idle eviction: a distinct `SUPERSEDED_CLOSE_CODE`, so the client
doesn't treat it as a dropped connection and steal the identity back
(flapping the two tabs forever), and `is_superseded`, checked at the top of
every heartbeat tick before anything else. Pinned by
`a_superseded_connection_is_closed_with_the_close_frame` and its `PARITY`
entry beside `IDLE_CLOSE_CODE`'s.

### 5.10 A counter that decides admission must never wrap

Every counter compared against a ceiling (connection slots, per-IP counts,
retained bytes) turns one unbalanced decrement into `usize::MAX`: the
comparison says "full" and the server refuses everything for the rest of the
process's life. `MemoryTracker::remove_bytes` was guarded from the start and
said why in a comment; the connection counters had the same shape and no
guard, because the reasoning lived in one comment instead of a helper. **When
one reasoning applies to several call sites, make it a function**:
`saturating_dec` in `limits/mod.rs`. Pinned by
`releasing_more_than_was_reserved_cannot_wrap_a_counter`.

### 5.11 Release in a wrapper, not at every exit

Teardown was each `return`'s duty in `handle_websocket`, and one ("room
vanished between admission and upgrade") skipped it, holding the slots until
the process died: §5.2 recurring one `return` at a time. Admission is
straight-line, so ordering fixes it; a session has four raced tasks and
several exits, so the release lives in a wrapper that holds even for exits
not yet written:

```rust
pub async fn handle_websocket(...) {
    run_session(...).await;   // every early return lands here
    cleanup_user(...).await;  // the one teardown
}
```

**Prefer a structure that can't violate the invariant over a rule that must
be remembered**; a test covers only today's exits.

### 5.12 The log is an interface, and the only one an operator has

There is no dashboard, tracing backend or debugger on a container serving
strangers, and no behavioural test covers the log: doing the right thing
silently looks like doing the wrong thing quietly. "skink left / stinks
joined" lasted as long as it did because nothing said why a visitor got a new
name, and every refused connection was silent.

- **Log the decision, not just the outcome.** `admit_user` says `reclaimed`,
  `recognised` or `fresh`; a room repeating `fresh` for one person is the
  flapping bug at a glance.
- **A refusal names the ceiling and its value**, because the question is
  whether the limit is right, and that needs the number.

Asserted like any behaviour, with `capturing_logs` installing a scoped
subscriber (and §6.6d's trim that reported itself).

### 5.13 Operational disclosure is still disclosure

`/metrics` names no room, message or IP, which was long treated as "fine to
leave public." It isn't: counts and memory watched over time tell an attacker
when the server nears a ceiling, the moment `MAX_CONCURRENT_USERS` or
`MAX_ROOMS` is worth testing.

It is gated on a bearer token from `METRICS_TOKEN`
(`security::is_authorized_for_metrics`), checked first and failing closed:
with no token configured the endpoint behaves as if it doesn't exist. The
comparison is constant-time, since `==` leaks a token byte by byte to anyone
measuring latency. A wrong and an unset token both read as 404, not 401, so
the route isn't confirmed to a wrong guess (as `is_allowed_origin` doesn't
distinguish a wrong origin from none, §4).

Cloudflare Web Analytics, a beacon injected by a zone setting rather than by
this code, is the other side: §5.7 applies to Cloudflare's own analytics too.
It's not fixable here, but leaving it on means the zone contradicts the CSP's
"no third-party origins."

### 5.14 A human-facing admin page may answer differently from a scraped one

`/admin` (`routes::admin_dashboard_handler`) lists every open room as a link:
a larger disclosure than `/metrics`, so it has its own token, `ADMIN_TOKEN`
(`security::is_authorized_for_admin`), and one leaked credential doesn't open
both.

It breaks §5.13's 404 rule on purpose. That rule suits an endpoint a scraper
polls; `/admin` is opened by a person clicking a bookmark, and only a real
401 with `WWW-Authenticate: Basic` makes the browser prompt (a link can't
carry an `Authorization` header, but a browser remembers Basic credentials).
The cost, an unauthenticated prober learning the route exists, is accepted
for this one endpoint.

Room names pass through `ammonia::clean_text` before formatting. Every name
in `state.rooms` already passed `matches_room_name_shape`, so this closes no
current gap; it keeps the guarantee if what a room name may contain ever
changes.

### 5.15 A reply owed to one connection must not go through a shared channel

`apply_client_event_at`'s `RequestRoster` arm said "answered to the asker
alone: the room's broadcast channel would send it to everybody," then sent
it through `room_state.sender`, the room's broadcast channel. Every client
received a `Roster` for a panel it never opened: the quadratic §1.1 avoids by
making the roster request-based, back through another door. It looked right
because every other `room_state.sender.send(...)` in the file is correctly a
broadcast; it matched the pattern in the one place the pattern didn't apply
(§0.5, aimed at a line of code).

`apply_client_event_at` now returns `Option<OutgoingEvent>`: `Some` only when
the caller owes a reply the shared channel can't deliver, which
`receive_task` (`session/lifecycle.rs`), holding this connection's own sink,
sends. Verified by tests (the throttle table; nothing arriving on the
broadcast channel; a failing write ending the session, possible because
`run_session` is generic over its sink, §6.1c) and on a live server with two
connections, where only the asker received the frame.

---

## 6. The regression ratchet

### 6.1 The gate

Run by `.github/workflows/ci.yml` and expected to pass locally before every
commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features --locked
cargo build --release --locked
cargo test --release --all-features --locked nova_rln
```

plus the coverage floor, the container smoke test and the Worker's
typecheck, tests and audit. **Warnings are failures**: clippy found §0.5's
tautology that a reader missed. The clippy gate was once red for a while
unnoticed, because nothing depended on it.

Deploys now come from Cloudflare's Git integration on every push to `main`,
in parallel with CI rather than after it, so a failing commit can ship.
Branch protection requiring CI on pull requests is what puts the gate back in
front of the deploy; it is a repository setting this repo can't enforce.

### 6.1a A green gate that can't see the deploy is not a gate

The gate answers "will this deploy?", and for most of a day it said yes
wrongly: `wrangler` needs Node >= 22, the workflows pinned Node 20, and every
push showed fmt, clippy, 600 tests, a release build and coverage green before
the deploy step failed a version check. Moving the Worker to pnpm (whose
version 11 needs `node:sqlite`, also Node 22) moved the failure into the
gate, which is the only reason anyone noticed.

**A requirement only the last step enforces is a requirement nothing
checks.** The version now lives once, in `cloudflare/package.json`
`engines`, `engineStrict` makes the install refuse an older Node, and
`ci_node_version_satisfies_the_toolchain` checks the workflows agree, on a
laptop, before pushing.

### 6.1b The claim that rules are enforced is itself a contract

The opening claim can rot two ways silently, and both are checked:

- **Phantom enforcement**: the table names a renamed or deleted test, and the
  row reads the same (`every_test_the_standards_name_exists`).
- **Undocumented enforcement**: a contract test that neither `AGENTS.md` nor
  this file mentions, so its purpose lives only in the file
  (`every_contract_test_is_documented`). It identifies tests by their test
  attribute; deciding by name once let a test ending in `_source` through as a
  "helper".

The registry's shape is checked too: a watched-lever row without a reopen
condition is the "decided, then forgotten" failure §9.4 prevents
(`every_parked_decision_records_how_to_reopen_it`).

Ported in idea, not code, from the contract suite on `cameronaaron.com`:
**the enforcement needs enforcing**, and an exemption must not outlive its
reason. `every_declared_dependency_is_used` likewise fails if its allow-list
names a crate no longer in `Cargo.toml`.

### 6.1c An untestable line is usually a misplaced line

The coverage floor was 95%, excusing "the process shell". That was true of
the file, not the code. Splitting `startup.rs` from `main.rs` and taking two
things as parameters moved 78 lines from unreachable to covered:

- `main` is one line, the only honest exclusion, since no test can call it.
- `wait_then_announce` takes the shutdown signal instead of calling
  `ctrl_c()`; a test can't SIGINT itself, but can pass a resolved future,
  including one for a broken signal handler.
- `main_inner` returns an `ExitCode` instead of calling
  `std::process::exit`, which would take the test runner with it.
- `log_server_result` is a function, not a `select!` arm, so the error path
  needs no server failing mid-flight.

**Inject at the edge, where the untestable thing is.** That distinguishes
this from the injectable clock §9.4 rejects: a clock threads through every
timing decision; a signal is one parameter at the process boundary. What
remains is genuinely racy (a socket dying between two frames) and is exempted
line by line with reasons.

### 6.1d Every "untestable" line here was a misplaced one

`session.rs` went from 21 exempted lines to 12, 3, then 0, each covered by
moving code, not by a cleverer test:

- The four tasks and `run_session` are generic over their sink, so "the
  client stopped accepting frames" is a fixture, not a race.
- `join_room`, `send_history`, `touch_and_check_idle`, `attach_cookies` and
  `encode_event` became functions instead of blocks inside a loop or handler.
- `loop`/`break` became `while running`, stating the exit condition once.

**When a line looks untestable, ask first whether it is in the right place**;
here the answer was no, twelve times out of twelve.

Two of the last three were the instrument disagreeing with the code.
`Message::Binary(_) => { … }` reported its arm uncovered while its body ran,
because tarpaulin attributes a brace-wrapped arm to the body; written as one
expression, like the `Pong` arm beside it, it reports honestly.

Likewise, adding `trace!`/`debug!`/`warn!` logging across `room/`, `limits/`,
`session/` and `state.rs`, several multi-line macro calls (three or more
fields, one per line) reported one interior line uncovered although a test
ran the branch at a captured level with every field pre-bound, so lazy
evaluation wasn't the cause. llvm-cov anchors a macro's coverage regions to
its argument tokens, and an interior line's region didn't connect. Rewriting
each call on one physical line restored 100%; they fit once fields repeating
a `config.rs` constant already implied by the message were dropped. **Suspect
a multi-line macro invocation before the branch it sits in**, and try a
one-line rewrite before `scripts/coverage-exemptions.toml`.

### 6.2 A fix and its test are one commit

A fix without a test has an expiry date. The commit fixing the connection leak
contains the test that detects it; the commit curating the roster contains the
sweep that keeps it curated.

### 6.3 A found defect becomes a sweep for its class

The commit also adds the check that finds the next instance, and the sweep is
proven able to fail (inject a violation, watch it fire, revert) before it is
trusted. The roster had non-animals and duplicates; curating it fixed the
instance, and `animal_roster_is_sorted_unique_and_well_formed` (sorted,
unique, well-formed) is the sweep. It failed at once on a sorting slip
(`pika, pike, pigeon`) introduced by the fixing commit.

### 6.4 Assert properties, not tautologies

A test that can't fail reports coverage it doesn't provide: `assert!(x ||
!x)`, `assert!(result.is_ok() || result.is_err())`. Assert the property; for
`should_gc`, that a second immediate call returns `false`.

### 6.5 Tests declare their own imports

`src/tests/` imports each module by name rather than through `use crate::*`
inheriting whatever `main.rs` imports, so a failing test's imports say which
part of the server it belongs to, and the server can reorganise without the
tests silently depending on it.

### 6.6 Coverage says a line ran; mutation testing says it mattered

`scripts/coverage.sh` enforces a line-coverage floor. It was 100% and is
temporarily 84%: measuring the whole workspace exposed nova code that only a
live MPC ceremony or a release-only STARK proof reaches (the script lists what
and how to reopen). The floor only ratchets up; a drop is fixed with a test.

`cargo mutants` edits the source (flips a comparison, replaces a return
value) and fails when the suite still passes, catching tests like `assert!(result
|| !result)`, which had 100% coverage and asserted nothing. Run it per file
while iterating and in full periodically; it is too slow for a commit gate.

**Survivors get classified, not ignored.** The two most logic-dense modules
leave six:

| Survivor | Why it survives |
| --- | --- |
| `while total > peak` to `>=` | **Equivalent**: one redundant compare-and-swap writing the same value. |
| `end > 0` to `end >= 0` in `truncate_on_char_boundary` | **Equivalent in practice**: index 0 is always a char boundary, so the walk stops first. |
| Four `elapsed() < WINDOW` to `<=` | Differ only at exactly the threshold; killing them needs an injected clock for a one-nanosecond difference. Rejected, recorded in §9.4. |

### 6.6a Deleting a dependency leaves references that still parse

Removing the webfonts took out the `<link>` tags and requests
(`the_client_makes_no_third_party_requests` confirmed it), but two CSS
declarations naming the fonts remained. `#chat:empty::before` set
`content: 'chat_bubble'` in `'Material Icons Round'`, a ligature: without the
font, every empty room showed the literal word "chat_bubble" at 80px, and no
request-based test could see it. **When you delete a dependency, sweep for
what still names it**, not only what fetches it. Pinned by
`the_page_names_no_font_it_does_not_ship_with` and
`no_css_content_string_is_an_icon_ligature`.

### 6.6b Mutation testing finds the test you didn't think to write

Two gaps hid behind 100%-covered lines, both **a test reaching a function
without exercising its decision**. `is_animal_name` replaced with `true`
survived: the forged-cookie test's `user_id` wasn't a UUID, so the identity
was discarded before the name was checked. `public_message` replaced with
`""` survived: errors were tested for status codes only, so their bodies
could say anything. Only mutation testing tells covering a branch from
exercising its decision.

The report also has to be readable: exclusions like `^main$` matched nothing
(cargo-mutants matches the whole description, "replace main with ()"), and
three permanent survivors is how a report stops being read.

### 6.6c Ported from cameronaaron.com: what travels between codebases

That repo is TypeScript for a static site; this is a Rust WebSocket server.
None of the code ports; six ideas did, each as a sweep:

| Idea there | Here |
| --- | --- |
| The enforcement mechanism needs enforcing | `every_test_the_standards_name_exists`, `every_contract_test_is_documented`, `every_parked_decision_records_how_to_reopen_it` |
| An exemption must never outlive its reason | `every_declared_dependency_is_used`, `coverage_exemptions_are_justified_and_current`, the test-only list in `nothing_is_public_only_for_its_own_test` |
| A test that can't fail is worse than none | `no_assertion_in_this_suite_is_a_tautology` (found three) |
| A doc pointer that no longer resolves is a lie with a green gate | `every_section_reference_resolves`, extended to Rust comments |
| Dead exports hide behind their own tests | `nothing_is_public_only_for_its_own_test` |
| Full coverage with a short, reasoned exclusion list | `scripts/coverage-exemptions.toml` and its contract |

The second generalises furthest: every allow-list here fails when an entry
stops being needed (a crate gone from `Cargo.toml`, a path gone or a line
count drifted, a function deleted). **An exemption that can't expire is a
decision nobody revisits.**

### 6.6d Three things mutation testing keeps finding

**A guard with nothing behind it.** `if freed > 0 { remove_bytes(freed) }` and
`if len > target { retain_newest(target) }` survived three mutations each,
because both calls are already no-ops at the boundary. The fix was deleting
the guards (§0.2): a comparison that can't change what happens is a place for
a bug to hide.

**The log nobody reads.** The remaining trim guard gates an `info!`, and three
mutants lived there because no test read the output. `capturing_logs` made
the log assertable (§5.3, §5.12).

**A boundary nobody stood on.** Every surviving `>` to `>=` was a threshold
tests approached from both sides and never landed on. Counters needed a test;
durations needed `cleanup_rooms_at`, which takes the instant as a parameter:
edge injection, not the clock §9.4 rejects.

### 6.6e A hash nothing checks is not a hash

`fnv1a` survived six mutations, including returning a constant: its only
assertion was that two scripts got different stamps, which returning the
input's length also satisfies. **Asserting a function isn't degenerate isn't
asserting it's correct.** Pin anything with a defined answer (a hash, an
encoding, a checksum) to a published test vector. The stamped URL is cached
`immutable` for a year, so a hash that fails to change serves a stale client
indefinitely.

### 6.6f An `elapsed()` inside a comparison is a boundary no test can reach

Nine survivors in the limits code were `last_seen.elapsed() < RETENTION`
mutating to `<=` or `==`. The boundary matters (a ban expiring, a counter
reclaimed), but the clock is read inside the comparison, so no test can land
on it. Passing the instant in fixes the class: `cleanup_stale_at(now)` and
`check_ip_at(ip, now)`, after `cleanup_rooms_at(state, now)`. Public methods
keep their signatures and call `Instant::now()` once, so no clock is
injected (§9.4). "Exactly at the retention age" then becomes a decision: **a
retention is how long something is remembered, so when it is reached, it is
over.**

`RateLimiter::roll_window` has the same shape and was left alone: a
one-nanosecond difference on a per-message hot path, observably equivalent.
Threading an instant through it would buy a killed mutant and nothing else.

### 6.7 Assert on structure, never on a substring a comment can contain

Four tests failed because a comment explaining a removal named what it
removed: `#chat:empty {`, `user-scalable=no`, `fonts.googleapis.com`, and
`content: 'chat_bubble'` (the ligature sweep failing on the comment about the
ligature it had just removed; hence `embedded_html_without_comments()`).
Match the structure that takes effect (a selector at the start of a line, an
attribute in the right tag, an origin inside `src=`/`href=`/`url()`), not a
raw substring. A test a comment can break gets weakened the next time someone
documents a decision.

### 6.8 A boundary is a place to test, not to assume

Every mutant killed in one round was an off-by-one at a threshold: the memory
ceiling admitting exactly the limit, a sweep due strictly after its interval,
a ban strictly past the allowance. When a comparison guards a limit, test the
value at the limit and one past it.

### 6.9 Build artifacts are never committed, with one deliberate exception

`target/` (including a 7.5 MB binary) and 1.6 MB of tarpaulin reports were
once tracked: bloated clones, meaningless diffs, stale on the next build.
`Cargo.lock` is tracked, because the container must build from exactly what
CI tested; the old `.gitignore` had both backwards.

The exception is `nova-wasm/pkg/`. The server embeds `nova_wasm_bg.wasm` with
`include_bytes!` (constraint #11's model), and the container build has no
wasm toolchain, so the built package is committed and must be rebuilt with
`scripts/build-nova-wasm.sh` whenever `nova-wasm/` or its novachannel pin
changes.

### 6.10 Write down what exists, so removing it is a decision

Three regressions in one afternoon had one cause: a bulk edit removed rules it
wasn't aimed at, and **nothing knew they were supposed to be there.** A
visitor reported one; auditing found the other two (a hover bridge added an
hour earlier, and the whole `.room-fading` effect). Each sweep written in
response catches one kind of loss after it has happened once.

`scripts/client-inventory.toml` and `config.rs.inventory` are the general
form: a record of every element id, stylesheet rule, client method and server
tunable. Adding is free; **removal fails the gate**, naming the thing. Removal
isn't forbidden, it just can't happen by accident: if deliberate, delete the
line in the same commit and the diff says so; if not, you've found a bug
before a visitor did. A regex over a stylesheet doesn't know what a rule is
for, and neither does a reviewer of a four-thousand-line diff. A list does.

---

## 7. The engagement doctrine

Several behaviours that look like resource management are **the product**;
deleting them as "unnecessary cleanup" deletes what makes the site work.

### 7.1 Scarcity is the feature

| Mechanic | Constant | Why |
| --- | --- | --- |
| Empty rooms are deleted | `EMPTY_ROOM_CLEANUP_DELAY` (5 min) | A spawned room is a spark, not a hearth: deleting rooms that outlive their conversation means every room you find has someone in it (§7.4). |
| Idle users are disconnected | `USER_IDLE_MESSAGE_TIMEOUT` (10 min) | The count reads "people here," not "tabs left open." |
| `main` history fades | `MAIN_ROOM_FADE_IDLE` (30 min) to `MAIN_ROOM_FADE_KEEP` (75) | `main` can't be deleted, so it forgets, patiently (§7.4). |

These are product decisions with technical implementations, so they live in
`config.rs` with their reasoning.

### 7.2 `main` is special and stays special

`cleanup.rs` special-cases `MAIN_ROOM` and `continue`s before the deletion
branch; deleting the entry point would 404 the front door. A refactor that
"unifies" room cleanup must keep this.

### 7.3 The UI must not lie about the mechanics

`index.html` says when rooms disappear; `config.rs` decides. They have
disagreed (a fade warning at the wrong threshold, a timeout described after
the backend changed it). Tests over `EMBEDDED_HTML` assert the text matches
the constant. **A user-visible number is a constant with two call sites, and
both are checked.**

### 7.3a The lie can outlive the fix, and the test can defend it

The fade indicator (the coloured dot, the "Room fading soon... say
something!" toast) was built believing rooms died after 60 idle seconds:
`warning` at 30s, `critical` at 45s, while `EMPTY_ROOM_CLEANUP_DELAY` was 600.
The banner's prose ("go silent for ten minutes") was right and checked; the
indicator was a second, unchecked implementation of the same fact, until a
user called the site janky.

Two tests defended the error. One asserted `30 < 600` and `600 - 45 >= 15`,
true however early the warning fired (§6.4). The other required the string
`"1 minute room timeout"` in the client, so it would fail the moment someone
fixed the number. A test enforcing a comment that matches a wrong belief is
the belief wearing a passing test.

The fix made every threshold a fraction (`* 0.7`, `* 0.9`) of one client
constant that a Rust test ties to the backend: then
`ROOM_GRACE_PERIOD_SECONDS`, since split in two (§7.4).

### 7.3b A prose scan needs comments stripped first

That fix added a doc comment with a possessive ("the room's real remaining
life"). `classes_the_page_can_render` (§10.11a) paired every `'` in
`EMBEDDED_JS` as string delimiters, so one apostrophe in a comment shifted
every pairing after it and `.message.sent`/`.message.received` briefly read
as dead CSS. A line-based `//` stripper fixed that, until an escaped `\'` in
a message string broke the pairing again, and the stripper itself had been
cutting `` `${protocol}//${host}` `` in half. `single_quoted_literals` now
scans the script as JavaScript (comments, three kinds of string, `${…}` and
escapes) instead. **Any sweep treating raw source as data has to read it the
way the language does**, not only the sweep written first.

### 7.4 A spawned room is a spark; `main` is the hearth; they don't share a clock

`EMPTY_ROOM_CLEANUP_DELAY` and `MAIN_ROOM_FADE_IDLE` were both 600 seconds by
coincidence, which told neither story: a spawned room should feel brief and
private, `main` always findable. A visitor can't tell "the same number today"
from "the same kind of thing," so the code has to mean the difference.

A spawned room is deleted five minutes after it goes **empty** (300s): long
enough to click back in after leaving by accident, short enough not to sit
like a locked door with the light on. `main` fades after thirty minutes of
**silence** (1800s; `MAIN_ROOM_FADE_KEEP` raised from 50 to 75), whether or
not anyone is connected (constraint #3), while a spawned room's clock can't
start until everyone leaves. Two preconditions are two mechanics, not one
with two numbers.

So the indicator needed two constants, `MAIN_ROOM_FADE_SECONDS` and
`IDLE_EVICTION_SECONDS` in `client.js`, chosen by `this.roomName ===
MAIN_ROOM_NAME`. A spawned room can't die while its viewer is in it, so
warning that it is about to disappear would be §7.3's lie relocated. What can
happen to a present, silent visitor is losing their own seat
(`USER_IDLE_MESSAGE_TIMEOUT`, 600s), and that is what it now warns about.

Three tests asserted a constant's value back at itself
(`EMPTY_ROOM_CLEANUP_DELAY.as_secs() == 600`), one named as if it tested that
`main` is never deleted. They broke on the retune for reasons unrelated to
their claims and were deleted; `the_empty_room_grace_period_is_exact` and
`the_housekeeping_boundaries_are_exact` already covered the behaviour by
reading the constants. A fourth,
`test_frontend_timeout_text_matches_backend_constant`, checked the banner's
fade sentence against `EMPTY_ROOM_CLEANUP_DELAY` and passed only while it
happened to equal `MAIN_ROOM_FADE_IDLE`, the constant the sentence describes.
Repointed.

### 7.5 A mirrored constant is a table entry, not a bespoke test

§7.3a and §7.4 were one root cause twice: a `client.js` number that went
wrong when the backend constant it mirrored moved. A bespoke test per
incident relies on someone remembering under a deadline.

`frontend_parity.rs::every_mirrored_constant_matches_its_source_of_truth`
walks `PARITY`, a table of every scalar the client may not invent (JS name,
backend value, why both exist), reading each `const NAME = value;` from the
shipped script. A new mirrored constant is one row. The specialised tests
(`client_attachment_ceiling_matches_the_server`,
`client_reaction_roster_matches_the_server`,
`client_and_server_agree_on_the_idle_close_code`) stay, because they cover
more than a scalar; the table is the floor under them.

The same pass found `test_message_rate_limit_constant() {}`, an empty body
passing on every commit. `no_assertion_in_this_suite_is_a_tautology` now also
fails on a test whose body is exactly `{}`.

---

## 8. Naming and organisation law

### 8.1 A module has one job and states it

Every module's header comment says what it owns and, where non-obvious, what
it doesn't. No module is named `utils`, `helpers`, `common` or `misc`: those
describe where code was put and grow without bound. `main.rs` went from 2,369
lines holding the whole server to module declarations and a one-line `main`;
the wiring lives in `startup.rs`.

### 8.2 A name states the thing, not the mechanism

`SANITIZE_TIMEOUT` was a 50 ms window treating an identical message from the
same user as a double send; nothing to do with sanitising. Renamed
`DUPLICATE_MESSAGE_WINDOW`. A name for the wrong concept teaches the next
reader something false.

### 8.3 Constants carry their reasoning

Every `config.rs` constant says why it has its value. `3600` appeared in four
places meaning four things (cookie lifetime, ban duration, disconnected-user
retention, IP-counter retention); they are now four named constants that move
independently.

### 8.4 Factor for the test, when the test is the point

`build_router` is separate so tests exercise the real route table rather than
a hand-built copy (§9).

---

## 9. The shipped-artifact law: what the user receives is the only truth

### 9.1 Verify against the artifact, not the source that should produce it

The client is compiled in with `include_str!`, so a rebuild is what makes an
`index.html` edit real, and client tests assert over `EMBEDDED_HTML`, the
constant the server serves.

### 9.1a A successful deploy isn't proof the running instance changed

The server is one long-lived Cloudflare Containers instance
(`max_instances: 1`) holding WebSockets open. A `wrangler deploy`, from the
Git integration or `./deploy.sh`, can build and push an image while the busy
instance keeps serving the old one. Two deploys seconds apart (a queued Git
build plus a manual redeploy) race, and whichever image is applied last
wins, regardless of which commit is newer. Deployment timestamps and exit
codes prove an image was uploaded, not which one is running.

Caught in production (August 2026): a wording fix (`6b68f25`) showed a
correct deployment entry while the site served the old string for minutes.
The proof is the live bytes, compared with the source (§9.1, one layer
further):

```bash
curl -s https://thehellisthis.com/app.js?v=<hash> | grep -c '<the changed string>'
shasum -a256 <(curl -s https://thehellisthis.com/nova_wasm_bg.wasm) nova-wasm/pkg/nova_wasm_bg.wasm
```

### 9.1b Two files a client fetches together must not cache independently

`/nova.js` loads its wasm with `new URL('nova_wasm_bg.wasm',
import.meta.url)`, and relative resolution drops the query string, so unlike
`/app.js` the pair can't share a `?v=` stamp. Both were served `max-age=3600`,
cached separately by the browser and the edge, so a deploy within the hour
could pair new `nova.js` with old wasm: `wasm.novaclient_myKeyPackage is not a
function`. Both now carry an ETag and `no-cache`: revalidated every request, a
cheap 304 when unchanged, the right pair as soon as either changes.

**Any two client artifacts that depend on each other at runtime share one
version stamp, or, when the reference can't carry one, both force
revalidation.** Independent TTLs let them drift exactly while a deploy is in
flight.

### 9.2 Test the route table that ships

`router_mounts_every_public_route` builds the router with `build_router`, the
function `main` uses, and checks every public path responds. A test building
its own `Router::new().route(...)` proves axum works, not that this server is
reachable.

### 9.3 Documentation is part of the artifact

`README.md`, `AGENTS.md` and this file are believed. The README once claimed
`HttpOnly` cookies the code didn't set, a "~7.5 MB optimized release binary"
with no release profile, "397 tests" and "21 tests" in two sections, and Axum
0.7 against a pinned 0.8. A document wrong about checkable things isn't
trusted about the rest (§0.4).

### 9.4 The watched-levers registry

Every measured rejection, deferred optimisation or upstream block records its
reopen condition here, so parked decisions resurface instead of becoming lore.

| Lever | Status | Reopen when |
| --- | --- | --- |
| `panic = "abort"` in release | **Rejected** (§5.1): trades a per-connection failure for a total one | Never while one process holds all rooms; reopens if rooms are sharded across processes. |
| Multiple container instances | **Blocked**: state is in-process, so a second instance is a second, separate set of rooms behind the same URLs | Only with shared state (Durable Objects, or rooms pinned to instances by name). `max_instances: 1` is correctness, not only cost. |
| Owned `OutgoingMessage` in `chat_history` | **Replaced by `Arc`** (§1.4a): 119 µs under the write lock against 1.3 µs; whole join path 119 µs to 19.6 µs, the rest being per-viewer reaction resolution | Closed; reopens only if stored messages become mutable, which would make sharing unsound. |
| `chat_history` as `VecDeque` | **Deferred**: O(1) front removal, but `drain(..k)` already makes pruning O(n) once per prune, and about 40 tests use `Vec` slicing | If pruning ever moves onto the per-message path. |
| Per-IP rate limiting as real protection | **Rejected as a security boundary** (§5.8): the client-address headers are attacker-controlled off the Worker path | If the container becomes unreachable except through the Worker, verifiable at the network layer. |
| Sliding-window rate limiter | **Rejected**: needs a timestamp per event, and fixed-window edge effects are imperceptible in chat | If burst abuse is seen crossing window boundaries in production. |
| Secret scanning (gitleaks) | **In CI and as an opt-in pre-commit hook** (2026-08-06). The audits cover vulnerable dependencies, not credentials committed here. `oss-guard.yml` runs gitleaks-action over full history on every push and pull request, and `scripts/gitleaks.sh` does the same locally (`--log-opts="--all"`), since a leak deleted next commit stays in history. Two confirmed-harmless matches (RFC 6455's example handshake key, used in tests, and the string "ChaCha20-Poly1305") are allowlisted in `.gitleaks.toml` with reasons | If a real secret is found: rotate it and rewrite history, not just delete it. Extend the allowlist only for a confirmed non-secret, with a reason. |
| `cargo-deny` in CI | **Not added, after checking the one duplicate**: `tokio-tungstenite` 0.29 (via axum's `ws` feature) and 0.30 (this project's dev-dependency). Dev-dependencies never reach the binary and axum's next bump resolves it; the audits cover advisories | When a duplicate has a real cost: a licence conflict, a security-relevant duplicate in the shipped binary, or a measurable build regression. |
| Removing the double `ammonia::clean` | **Done** (§1.4d): reopened not for throughput (never measured as CPU-bound) but because the two passes disagreed, letting `<img src=x onerror=alert(1)>` through as an empty message | Closed; reopens only if a message's raw form needs validating for a reason the rendered output can't answer. |
| AVIF for attachments | **Deferred, unmeasured**: 20-30% smaller than WebP, but canvas AVIF *encoding* is thin, `toDataURL` silently returns PNG for unsupported types (a wasted 1600px lossless encode per retry), and it is synchronous and slow where it exists. Also needs `image/avif` in `ALLOWED_ATTACHMENT_MIMES` and magic bytes in `sniff_image_mime`. No browser was available to measure | When the encode path can be measured in a browser: a cached one-time probe on a 1×1 canvas and async `toBlob`. Only if attachment bytes become the binding constraint; they aren't. |
| Synchronous `toDataURL` in the attachment encoder | **Known cost, unmeasured**: up to 15 (size, quality) attempts, each a redraw plus a blocking encode; a 128 KB base64 ceiling is about 96 KB of image, which 1600px usually misses | With a browser to measure: `toBlob`, starting from a size estimated from the source. Shipping an unmeasured rewrite of a working path is what §0.5 warns against. |
| Rust nightly / experimental features | **Rejected**: one process holds every room, and nightly can change between two nights. What helps is already stable and used (let-chains, applied 49 times at the 1.88 MSRV bump; inline `const` blocks; `LazyLock`; async fn in traits). The bottlenecks are lock duration and memory, which no language feature moves | For a specific feature with a measured win stable can't express, pinned to an exact nightly the container build can reproduce. "Newer" isn't a reason (§0.3). |
| 100% mutation coverage | **A direction, not a target**: six survivors are classified in §6.6; four need the rejected injectable clock | If a reachable survivor appears that isn't one of the six. |
| Load testing | **Never done**: every performance claim here is structural, not measured throughput | Before raising `MAX_CONCURRENT_USERS` (400) or `MAX_USERS_PER_ROOM` (100), which are unvalidated assumptions (§0.5). |
| Pinning `Dockerfile.cloudflare`'s base images | **Rejected, twice**: briefly pinned to `rust:1.97-bookworm`/`debian:bookworm-slim`, reverted to the standing policy of running at latest and catching problems with the weekly audits. A scan of `rust:latest` then showed 1 critical and 17 high findings in the builder stage, which is discarded after the build; such counts change with each pull, so no test asserts them (`these_images_track_latest_on_purpose` checks the tags move). The runtime stage is `gcr.io/distroless/cc-debian12:nonroot`: no shell or package manager, only what `ldd` on the binary names (`libc`, `libm`, `libgcc_s`, checked in a Linux build), and a built-in non-root user; `docker exec .../bin/sh` fails on the built image | If rebuilds become too rare for "latest" to mean anything (a running container isn't patched until rebuilt); a scheduled rebuild would fix that. For distroless, if a dependency needs a shell or package manager. |
| Injectable clock for the limiters | **Rejected** (§6.6): would only kill four mutants that differ at exactly the threshold | If a timing bug is seen at a limit boundary in production, or the limiters need testable time for another reason. |
| 100% line coverage | **Regressed to 84%** (2026-08-06) when `scripts/coverage.sh` gained `--workspace`. Without it, `nova-operator`'s own tests never counted: the instrument, not the code (§0.5). Measuring properly exposed nova code the instrumented run never reached: the DKG/FROST ceremony in `nova-operator/src/main.rs`'s `main()` (`run_steady_state` was made generic over its sink and tested), and the STARK accept/duplicate/slash paths in `session/nova_rln.rs`, which need a release-only proof. Same-day tests took it from 79.78% to over 85%. Before nova it was 100.00%, with or without the `#[ignore]`d tests, and two structural whole-file exemptions (the test suite and `main.rs`) | At 100% once the ceremony gets the generic-sink treatment and the STARK paths get a debug-safe route or a merged release coverage pass (itemised in `scripts/coverage.sh`). Raise the floor as each gap closes; never treat 84 as normal. |
| Edge-caching the room pages | **Rejected**: the page is per room and sets identity cookies, so a shared cache would serve one visitor's `Set-Cookie` to another | If identity moves entirely to the socket and the page becomes byte-identical for everyone. |
| `Cross-Origin-Embedder-Policy: require-corp` | **Rejected, unmeasured in production**: alongside the `Cross-Origin-Resource-Policy: same-origin` this server does send, it broke the WebSocket locally (`net::ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS`, Chrome's Private Network Access), and there was no safe way to test against the real origin | With a way to test against `wss://thehellisthis.com` before shipping; PNA classifies by address space, and localhost may trigger a check production wouldn't. |
| Cloudflare's Web Analytics beacon (`static.cloudflareinsights.com/beacon.min.js`) | **Observed in production, unresolved**: the CSP (`script-src 'self' 'wasm-unsafe-eval'`) blocks it, so nothing is disclosed, but Cloudflare's edge injects the `<script>` into every HTML response; nothing in this repository references it. Almost certainly the zone's Web Analytics/Browser Insights toggle | Closed once the zone setting is off (it isn't in `wrangler.jsonc`) and the console warning is gone on a fresh load. |

---

## 10. The client law

The client is one HTML file and one script, with no framework, build step or
tests of its own beyond assertions over the shipped bytes, which makes a few
rules load-bearing.

### 10.1 Never derive state you can read

A running `messageCount` decided when to trim rendered history. System
messages incremented it and removed themselves eight seconds later without
decrementing, so in a busy room it drifted high and deleted live messages far
from the limit, invisibly, because the page was right and the counter wrong.
The fix wasn't a matching decrement: the client counts
`querySelectorAll('.message')` where needed. A derived value that can drift
will.

### 10.2 Never fight the browser for the scroll position

`scroll-behavior: smooth`, per-message pinning to the bottom and the
browser's scroll anchoring all moved the chat at once. During history replay
they slid visibly, and the client's own scroll events read as "the user
scrolled up", switching auto-scroll off partway: hence the intermittency.

- Programmatic scrolling is instant; smooth is opt-in for scrolling the user
  asked for (`scrollToMessage`).
- `overflow-anchor: none` where the client manages scrolling.
- Scroll events from the client's own writes (`programmaticScroll`) and
  during replay (`isLoadingHistory`) are ignored.
- `ReconnectToken` marks the replay's end, with a timeout failsafe so a
  missing frame can't leave the UI degraded.

### 10.3 A state that only exists sometimes must not change the layout

`#chat:empty` set `justify-content: center`, so the first history message
snapped the column from centred to top-aligned on every load. Empty, loading
and other sometimes-states are overlays positioned out of flow.

### 10.4 Escape for the sink you are writing to

`textContent` doesn't interpret markup, so escaped text double-escapes (an
`&` showed as `&amp;`); `innerHTML` does, and needs escaped values. The client
escaped for both: a display bug everywhere and no extra safety anywhere. One
escape helper, used only where markup is built as a string.

### 10.5 Accessibility is not negotiable for convenience

`user-scalable=no` blocked pinch-zoom (WCAG 1.4.4). The reason people set it,
iOS zooming on input focus, is solved by a 16px input font, which this client
already had.

### 10.6 A selector styled twice is a value undone somewhere else

Thirteen selectors (`#chat`, `.message`, `.input-container`, `.reaction-bar`
among them) were defined twice. The later block silently wins while both
read as if they apply; a deliberate padding was undone thirteen hundred lines
later. That is "weirdly proportioned" from the inside: two right values
disagreeing. Merged, keeping the cascade's result, and pinned by
`no_css_selector_is_defined_twice`.

Related: **the conversation is a column, not the window.** Full-width `#chat`
with 560px bubbles put one message against the left edge and the next against
the right on a wide monitor. The composer is laid out against the same width.

### 10.7 A control that appears on hover must survive being aimed at

The reaction bar is `position: fixed` and a sibling of `#chat`, so moving
towards it leaves the chat, and hiding on that made it vanish as you aimed.
All of these are needed: the hide is delayed, arriving on the control cancels
it, leaving the container onto the control isn't leaving, and the bar isn't
rebuilt while showing (or the button is replaced between `pointerdown` and
`pointerup`). **Anything shown on hover and then clicked has a gap to cross**;
bridge it or forgive it.

### 10.8 A class the renderer sets and the stylesheet never mentions

The client builds its DOM in JavaScript, so a class set but never styled is
an unstyled box with no error. During the iMessage-style rework, a sweep
removing `.message*` rules also hit `.message-image-faded` (`\b` matches at a
hyphen), and the faded-picture placeholder lost its box.
`every_class_the_client_renders_is_styled` checks each class in `client.js`
against the stylesheet. That removal was needed because of §10.6's
duplicates: bulk stylesheet edits produce both failures, so both are
machine-checked.

### 10.9 A rule that shows something implies a rule that hides it

`.reply-preview.active { display: flex }` means nothing unless the base
`.reply-preview` is hidden. A bulk sweep removed the base rule; the class was
still mentioned, so §10.8's check passed, and the preview with its placeholder
text sat above the composer permanently ("Replying to / Message text…") until
a reader reported it. §10.3 the other way round: a sometimes-visible thing
must start invisible. Pinned by
`every_conditional_display_rule_has_a_base_that_hides_it`.

One bulk sweep caused three regressions (a duplicated selector, a class
caught by `\b` at a hyphen, and this). **A regex over a stylesheet doesn't
know what a rule is for**; mechanical CSS edits need mechanical verification
in the same commit.

### 10.10 Ask for what a panel needs; don't push it to everyone

Putting names in the `UserCount` broadcast would cost O(users) per recipient
(ten thousand name copies when someone in a hundred-person room blips) for a
panel almost nobody has open. `RequestRoster` is answered to the asker, and an
open panel stays current from `UserJoined`/`UserLeft`: O(users) once, when a
person opens something. **A feature that needs a collection asks for it when
it is looked at.** Reactions live in a side table with `reacted` resolved per
viewer for the same reason.

### 10.11 An id beats every class, so an id must never carry layout

`#userCount` styled the old header's inline count (`display: inline-flex`, a
gap, a background). After a redesign that id sat on the navigation bar's
centre button, a column of room name over subtitle, and the old rule
outranked every class: icons overlapping, text stacking, nothing logged. It
was reported as "the icons are really not showing up right and things are
very misaligned and stacking on eachother", the only signal this bug gives.

**Ids identify, classes style.** `stylesheet_ids_do_not_carry_layout` allows
a list of structural containers that exist once and are never restyled by
role, each checked to still be in the page.

The redesign also left `.brand` and `.brand:hover` styling nothing. Dead CSS is
a trap: the next `.brand` inherits someone else's styling.
`no_rule_styles_something_the_page_never_renders` fails on a class rule whose
every selector names something neither the markup nor the renderer can
produce. It reads template literals as the browser will: in
`` `reaction-pill${mine ? ' mine' : ''}` `` each `${…}` is cut out before
splitting, since splitting on whitespace first would discard the class glued
to the expression.

### 10.11a Both contracts skipped every selector inside `@media`

`.room-info` and `.logo-icon` survived §10.11 inside `@media (max-width:
768px)`, next to a live `#userCount { display: none; }`, the same bug at an
untested viewport. Both contracts skipped lines starting with whitespace or
`@`, meant for declarations and `@media` headers, but selectors inside a
media query are indented, so every rule there was skipped. The loop never
reached the line. Trimming before the `@` check fixed it.

That changed what "alive" meant. The dead-class contract accepted a selector
if any class was alive (`.iter().any(...)`), so `.logo-icon .icon` stayed
"alive" because `.icon` is everywhere, though a descendant selector matches
nothing without its container. Requiring every class (`.iter().all(...)`)
models the cascade, but widened the alive set's needs:
``row.className = `message ${isSent ? 'sent' : 'received'} run-end` `` hides
classes in a ternary, and `updateConnectionStatus('connected')` passes a
state to code that builds the class elsewhere. So any single-quoted string
shaped like a class name anywhere in the script counts as alive; in this file
a bare lowercase hyphenated word in quotes is overwhelmingly a class or state.

### 10.12 The client's own §1: per-message work is O(1) there too

`renderMessage` ran DOM queries over the whole chat on every message: one to
dedupe by `data-message-id`, `.message:last-of-type` for the previous bubble,
and `pruneRenderedMessages`'s two `querySelectorAll` calls. Replaying a full
history (up to 500 messages in a tight loop) was O(n²). One message pipeline
has a server half and a client half, and §1 doesn't stop at the WebSocket.

`renderedMessages`, a `Map<message_id, element>`, answers dedupe with `.has`;
since a `Map` iterates in insertion order, its `.size` is the count and its
oldest entries are what to prune, in O(excess). `lastMessageEl` replaces the
`:last-of-type` query. The date-separator sweep now runs only when
`excess > 0`, since a stale separator can only follow a prune.

The running counter tried before (§10.1) drifted because system notices
incremented it and removed themselves. `renderedMessages` doesn't, because
only `renderMessage` and `pruneRenderedMessages` touch it, and
`addSystemMessage` never goes through either.

---

## Appendix

This document was rewritten in July 2026. The file it replaced described a
Next.js portfolio site (React render paths, Lighthouse budgets, mobile tap
targets) that doesn't exist here, cited by an agent-instructions file
describing a component tree this repository doesn't have: §0.4's failure at
the largest scale. The laws above come from this server's lock, memory
ceiling and protocol, and the defects found writing them down.

### Release verification findings (2026-09-11)

The default `Dockerfile` had drifted to Rust 1.81 and omitted the operator
workspace and embedded browser assets, while CI inspected only
`Dockerfile.cloudflare`. It now uses the production recipe, the entry-point
contract fails on the old one, and both builds enforce `Cargo.lock`.

The Worker must forward `ADMIN_TOKEN` and `METRICS_TOKEN` (like
`NOVA_OPERATOR_TOKEN`) to the container explicitly: Worker bindings don't
become process environment variables. The Node Worker tests exercise the real
constructor and fetch handler with only Cloudflare's transport stubbed,
including absent secrets, handshake preservation and uncached 503s. Node
alone needs a duplex hint for streaming request bodies; the test adapter
supplies it without changing the Workers path.

The pnpm 11.12.0 pin broke frozen installs before any check ran; 11.22.0
installed the lockfile. The audits also found advisories in h2 0.4.15 and in
miniflare's exact `sharp` pin. Keep the audits in CI and the manual deploy;
a successful build doesn't detect these.

The smoke script used BSD-only `mktemp -t prefix`, which fails under GNU; it
uses an explicit six-X template now. Its EXIT trap fell back to `kill 0` when
compilation failed before a server PID existed, signalling the caller's
whole process group; cleanup now signals only a started child, and a
failing-compiler regression test intercepts `kill` and checks the exit code.

Container verification must build and run `linux/amd64`, the platform
Wrangler selects, even on an ARM machine; a native ARM smoke test says
nothing about the target's runtime libraries.

Branch protection on `main` (enabled 2026-09-12 on the previous repository)
required an up-to-date pull request passing Gate, Coverage floor, Worker
typecheck, Dependency audit and the secret scan, applied to administrators,
with force pushes and deletion disallowed and no extra approvals so a sole
maintainer can merge a green PR. The secret scan is now oss-guard's `guard`
job. Recheck the live setting whenever job names change, or the repository
is recreated.

gitleaks-action needs `GITHUB_TOKEN` to read pull-request metadata, unlike
the local CLI; PR #8 exposed the missing binding. Its job gets `contents:
read` and `pull-requests: read` and comments are disabled rather than granted
write. `secret_scan_can_read_pull_requests_without_write_permissions`
enforces this on `oss-guard.yml`.
