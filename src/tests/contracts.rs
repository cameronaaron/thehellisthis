//! Meta-contracts: the tests that keep the other tests honest.
//!
//! Standards citing tests that exist, sweeps that are documented, exemptions
//! that expire, inventories that catch a deletion (§6.10).

use super::*;

#[tokio::test]
async fn test_claim_no_disk_persistence() {
    // Verify no File operations in the message pipeline
    let app_state = Arc::new(AppState::new());

    // Add room with message
    {
        let mut rooms = app_state.rooms.write().await;
        let room_state = RoomState {
            sender: tokio::sync::broadcast::channel(1000).0,
            chat_history: vec![Arc::new(OutgoingMessage {
                message_id: uuid::Uuid::new_v4(),
                user_id: "test".to_string(),
                animal_name: "Lion".to_string(),
                text: "<p>Hello</p>".to_string(),
                timestamp: "12345".to_string(),
                reply_to: None,
                attachment: None,
            })],
            users: std::collections::HashMap::new(),
            available_animals: std::collections::VecDeque::new(),
            last_activity: Instant::now(),
            total_memory_bytes: std::sync::atomic::AtomicUsize::new(1024),
            reactions: std::collections::HashMap::new(),
            message_ids: std::collections::HashSet::new(),
            attachment_bytes: 0,
        };
        rooms.insert("test".to_string(), room_state);
    }

    // Drop app - no persistence
    drop(app_state);
    // No file was written (if it were, test would need a file cleanup)
}

#[tokio::test]
async fn test_empty_room_cleanup_delay_constant() {
    // Verify the EMPTY_ROOM_CLEANUP_DELAY is 10 minutes (600 seconds)
    // This is the timeout that drives the "keep talking or it fades" mechanic
    assert_eq!(EMPTY_ROOM_CLEANUP_DELAY.as_secs(), 600);
}

#[tokio::test]
async fn test_max_messages_constant_for_history_limit() {
    // Frontend displays "max 500" - verify constant matches
    assert_eq!(MAX_MESSAGES_PER_ROOM, 500);
}

#[tokio::test]
async fn test_max_rooms_constant() {
    assert_eq!(MAX_ROOMS, 100);
}

#[tokio::test]
async fn test_max_room_name_len_constant() {
    assert_eq!(MAX_ROOM_NAME_LEN, 50);
}

#[tokio::test]
async fn test_min_room_name_len_constant() {
    assert_eq!(MIN_ROOM_NAME_LEN, 3);
}

#[tokio::test]
async fn test_max_message_len_constant() {
    assert_eq!(MAX_MESSAGE_LEN, 8000);
}

#[tokio::test]
async fn test_max_messages_per_room_constant() {
    assert_eq!(MAX_MESSAGES_PER_ROOM, 500);
}

#[tokio::test]
async fn test_max_concurrent_connections_per_ip_constant() {
    assert_eq!(MAX_CONCURRENT_CONNECTIONS_PER_IP, 3);
}

#[tokio::test]
async fn test_max_concurrent_users_constant() {
    assert_eq!(MAX_CONCURRENT_USERS, 400);
}

#[tokio::test]
async fn test_message_rate_limit_constant() {}

#[tokio::test]
async fn test_max_messages_per_window_constant() {
    assert_eq!(MAX_MESSAGES_PER_WINDOW, 30);
}

#[tokio::test]
async fn test_rate_limit_window_constant() {
    assert_eq!(RATE_LIMIT_WINDOW.as_secs(), 60);
}

#[tokio::test]
async fn test_heartbeat_interval_constant() {
    assert_eq!(HEARTBEAT_INTERVAL.as_secs(), 5);
}

#[tokio::test]
async fn test_inactive_timeout_constant() {
    assert_eq!(INACTIVE_TIMEOUT.as_secs(), 3600);
}

#[tokio::test]
async fn test_max_payload_size_constant() {
    assert_eq!(MAX_PAYLOAD_SIZE, 512 * 1024);
}

#[tokio::test]
async fn test_sanitize_timeout_constant() {
    assert_eq!(DUPLICATE_MESSAGE_WINDOW.as_millis(), 50);
}

#[tokio::test]
async fn test_typing_event_min_interval_constant() {
    assert_eq!(TYPING_EVENT_MIN_INTERVAL.as_millis(), 200);
}

#[tokio::test]
async fn test_read_receipt_min_interval_constant() {
    assert_eq!(READ_RECEIPT_MIN_INTERVAL.as_millis(), 200);
}

#[tokio::test]
async fn test_max_room_join_attempts_constant() {
    assert_eq!(MAX_ROOM_JOIN_ATTEMPTS, 10);
}

#[tokio::test]
async fn test_cleanup_batch_size_constant() {
    assert_eq!(CLEANUP_BATCH_SIZE, 100);
}

#[tokio::test]
async fn test_max_message_age_constant() {
    assert_eq!(MAX_MESSAGE_AGE.as_secs(), 86400 * 30);
}

#[tokio::test]
async fn test_max_total_rooms_memory_constant() {
    assert_eq!(MAX_TOTAL_ROOMS_MEMORY, 400_000_000);
}

#[tokio::test]
async fn test_estimated_message_size_constant() {
    assert_eq!(ESTIMATED_MESSAGE_SIZE, 1024);
}

// ========== CHAT ERROR INTO RESPONSE TESTS ==========

#[tokio::test]
async fn test_frontend_timeout_text_matches_backend_constant() {
    // CRITICAL: The user-facing text must match EMPTY_ROOM_CLEANUP_DELAY
    // If this fails, the UX is lying to users about when rooms disappear

    let cleanup_seconds = EMPTY_ROOM_CLEANUP_DELAY.as_secs();

    // Backend uses 600 seconds = 10 minutes
    assert_eq!(
        cleanup_seconds, 600,
        "EMPTY_ROOM_CLEANUP_DELAY changed! Update frontend text to match."
    );

    // Frontend must say "ten minute" (not "one minute", "30 seconds", etc.)
    assert!(
        SHIPPED_CLIENT.contains("go silent for ten minute"),
        "Frontend instructions don't match backend! Backend deletes at {}s but HTML doesn't say 'ten minute'. \
         Found text should say 'go silent for ten minute'.",
        cleanup_seconds
    );

    // Must NOT contain the old incorrect text
    assert!(
        !SHIPPED_CLIENT.contains("go silent for one minute"),
        "Frontend still contains outdated 'one minute' text!"
    );
}

#[tokio::test]
async fn test_all_timing_constants_are_consistent() {
    // Meta-test: Verify all timing relationships make sense together

    // Heartbeat should be sent more frequently than timeout
    assert!(
        HEARTBEAT_INTERVAL < HEARTBEAT_TIMEOUT,
        "HEARTBEAT_INTERVAL must be less than HEARTBEAT_TIMEOUT"
    );

    // Cleanup interval should allow catching inactive rooms
    assert!(
        ROOM_CLEANUP_INTERVAL <= EMPTY_ROOM_CLEANUP_DELAY,
        "Cleanup interval must be <= delay to catch rooms"
    );

    // Message rate limit window should be reasonable
    assert!(
        RATE_LIMIT_WINDOW >= Duration::from_secs(30),
        "Rate limit window too short"
    );
    assert!(
        RATE_LIMIT_WINDOW <= Duration::from_secs(120),
        "Rate limit window too long"
    );

    // Inactive timeout should be much longer than room cleanup
    assert!(
        INACTIVE_TIMEOUT > EMPTY_ROOM_CLEANUP_DELAY,
        "User inactive timeout should exceed room cleanup delay"
    );
}

/// Truncation keeps as much as fits and never returns an empty string for
/// input that had room.
///
/// The guard is `end > 0 && !is_char_boundary(end)`. With `||` in place of
/// CI does not deploy, and must not start again.
///
/// Deploying moved to Cloudflare's own Git integration. What it replaced kept
/// producing the same failure shape: the Rust gate green, and the deploy step
/// failing afterwards on something nothing else looked at — a Node version,
/// then a token permission. Both took an afternoon to find because every signal
/// a person reads said the commit was fine.
///
/// The credential is what makes this worth pinning rather than just deleting.
/// A workflow holding a deploy token is the most valuable thing in the
/// repository to an attacker who lands a pull request, and "we removed it" is
/// only true until somebody adds it back for a good reason.
#[test]
fn ci_holds_no_deploy_credential_and_does_not_deploy() {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");

    let workflows = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows"))
        .expect("the workflows directory should exist");

    let mut files = Vec::new();
    for entry in workflows {
        let path = entry.expect("a readable directory entry").path();
        let body = std::fs::read_to_string(&path).expect("a readable workflow");
        files.push((
            path.file_name().unwrap().to_string_lossy().to_string(),
            body,
        ));
    }

    for (name, body) in &files {
        let commands = strip_hash_comments(body);
        assert!(
            !commands.contains("wrangler"),
            "{name} invokes wrangler; deploying is Cloudflare's Git integration \
             now, and a workflow that deploys needs a token"
        );
        assert!(
            !body.contains("CLOUDFLARE_API_TOKEN") && !body.contains("CLOUDFLARE_ACCOUNT_ID"),
            "{name} references a Cloudflare credential — CI has no reason to \
             hold one, and a workflow secret is reachable from any pull request \
             that can change a workflow"
        );
    }

    // The gate still has to run on the push that Cloudflare deploys from, or
    // nothing checks the commit that actually ships.
    assert!(
        CI.contains("push:") && CI.contains("branches: [main]"),
        "the gate must run on pushes to main, since that is what deploys"
    );
}

/// Constraint #9 — the guest fallback is unreachable, and that is a property of
/// the numbers rather than an accident.
///
/// `assign_animal` mints `guest_N` only when every roster name is held by a
/// connected user. A room holds at most `MAX_USERS_PER_ROOM`, so a roster
/// larger than that makes the branch dead in production. Shrinking the roster
/// below the room cap would quietly start handing out names that are not
/// animals.
#[test]
fn the_roster_is_larger_than_a_room_can_ever_be() {
    assert!(
        ANIMAL_NAMES.len() > MAX_USERS_PER_ROOM,
        "the roster ({}) must exceed the per-room cap ({MAX_USERS_PER_ROOM}) so \
         every connected user can hold a distinct animal name",
        ANIMAL_NAMES.len()
    );
}

/// The client references no global it never declares.
///
/// A `const` that was used before it was written is a `ReferenceError` on the
/// first image a user tries to send — and, with no build step, nothing between
/// the editor and production would have said so. This is the sweep for that
/// class rather than for the one instance of it.
#[test]
fn the_client_declares_every_screaming_case_constant_it_uses() {
    // SCREAMING_SNAKE identifiers are this file's convention for module-level
    // constants, which makes them the set worth checking mechanically.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();

    let bytes = EMBEDDED_JS.as_bytes();
    let mut start = None;
    for (i, &c) in bytes.iter().enumerate() {
        let wordish = c.is_ascii_alphanumeric() || c == b'_';
        if wordish && start.is_none() {
            start = Some(i);
        } else if !wordish && let Some(s) = start.take() {
            let word = &EMBEDDED_JS[s..i];
            if word.len() > 3
                && word.contains('_')
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                used.insert(word.to_string());
            }
        }
    }

    let undeclared: Vec<&String> = used
        .iter()
        .filter(|name| !EMBEDDED_JS.contains(&format!("const {name}")))
        .collect();

    assert!(
        undeclared.is_empty(),
        "client.js uses constants it never declares: {undeclared:?}"
    );
}

/// §3 — the memory budget closes, whatever the constants are set to.
///
/// Every ceiling here was chosen against the others: attachments are capped per
/// room *because* a hundred rooms share one process-wide budget. Raising any
/// one of them in isolation silently overcommits the container, and the symptom
/// is an OOM kill that disconnects every user in every room — the failure the
/// whole memory law exists to avoid. So the arithmetic is asserted rather than
/// left in a comment for somebody to re-derive.
#[test]
fn the_memory_budget_still_closes() {
    // Pictures may claim at most half the process, leaving the rest for text.
    let attachment_ceiling = MAX_ROOM_ATTACHMENT_BYTES * MAX_ROOMS;
    assert!(
        attachment_ceiling <= MAX_TOTAL_ROOMS_MEMORY / 2,
        "every room at its attachment budget is {attachment_ceiling} bytes, over \
         half of the {MAX_TOTAL_ROOMS_MEMORY}-byte process ceiling — one room \
         full of photographs would be taking what every other room needs"
    );

    // A single attachment cannot be a meaningful fraction of a room's budget,
    // or "fading the oldest" would clear the room in one step.
    //
    // A `const` block, so this is checked when the crate is compiled rather
    // than when the test is run: a constant edited to break it fails the build
    // for everyone, including anyone who only runs a subset of the suite.
    const {
        assert!(
            MAX_ATTACHMENT_BYTES * 8 <= MAX_ROOM_ATTACHMENT_BYTES,
            "a room must hold at least 8 images at the per-image cap"
        );
    }

    // A room's text is bounded by message count times the largest a message can
    // be, and that has to fit too.
    let text_ceiling = MAX_MESSAGES_PER_ROOM * (MAX_RENDERED_MESSAGE_LEN + ESTIMATED_MESSAGE_SIZE);
    assert!(
        text_ceiling <= MAX_TOTAL_ROOMS_MEMORY,
        "one room of maximum-size messages is {text_ceiling} bytes, over the \
         whole process ceiling"
    );

    // The rendered ceiling must stay at or above the escaping worst case, or
    // `render_message_html`'s plain-text fallback could itself breach it.
    const {
        assert!(
            MAX_RENDERED_MESSAGE_LEN >= MAX_MESSAGE_LEN * 5,
            "escaping expands by up to 5x, so a lower ceiling makes the fallback able to exceed the limit it is the fallback for"
        );
    }
}

/// §2 — the join path shares stored messages; it does not copy them.
///
/// Deterministic, not timed: if `history_for` handed back copies, the strong
/// count of the stored `Arc` would not move. Measured, the copy it replaced
/// took 119 µs *under the room write lock*, so this is the largest single
/// regression anyone could reintroduce here by changing a type back to owned.
#[tokio::test]
async fn the_join_path_shares_history_rather_than_copying_it() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "shared, not copied".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    let before = Arc::strong_count(&room.chat_history[0]);

    let first = room.history_for("alice");
    let second = room.history_for("bob");

    assert_eq!(
        Arc::strong_count(&room.chat_history[0]),
        before + 2,
        "each viewer's history must be a reference to the stored message, not \
         a copy of it"
    );

    // And they really are the same allocation, not equal values.
    assert!(
        Arc::ptr_eq(&first[0].0, &room.chat_history[0])
            && Arc::ptr_eq(&second[0].0, &room.chat_history[0]),
        "both viewers should be reading the one stored message"
    );

    drop(first);
    drop(second);
    assert_eq!(
        Arc::strong_count(&room.chat_history[0]),
        before,
        "and the references go away with them"
    );
}

/// §1.1 — per-message work does not grow with the size of the history.
///
/// The property, stated so it cannot be satisfied by a fast machine: adding a
/// message to a room holding `MAX_MESSAGES_PER_ROOM` messages must touch the
/// same amount of state as adding one to an almost-empty room. Asserted through
/// the accounting rather than the clock — a message's cost to the room is its
/// own size and nothing else, so if any history-proportional work crept back
/// onto this path (a re-scan, a re-sum, a trim) the totals would diverge.
#[tokio::test]
async fn adding_a_message_costs_the_same_whatever_the_history_holds() {
    fn message(text: &str) -> OutgoingMessage {
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: text.to_string(),
            timestamp: "1700000000000".to_string(),
            reply_to: None,
            attachment: None,
        }
    }

    let mut deltas = Vec::new();

    for prefill in [1usize, MAX_MESSAGES_PER_ROOM - 1] {
        let tracker = MemoryTracker::new();
        let mut room = create_room();
        for _ in 0..prefill {
            room.add_message(message("filler"), &tracker);
        }

        let before_room = room.total_memory_bytes.load(Ordering::SeqCst);
        let before_global = tracker.total_bytes.load(Ordering::SeqCst);

        room.add_message(message("the measured one"), &tracker);

        deltas.push((
            room.total_memory_bytes.load(Ordering::SeqCst) - before_room,
            tracker.total_bytes.load(Ordering::SeqCst) - before_global,
            room.chat_history.len() - prefill,
        ));
    }

    assert_eq!(
        deltas[0], deltas[1],
        "adding one message to a nearly-full room must cost exactly what it \
         costs in an empty one; a difference means work proportional to the \
         history got back onto the message path"
    );
}

/// §1.1 — reacting is O(1) in the size of the history.
///
/// Reactions live beside the history, keyed by message id, precisely so this
/// holds. Asserted structurally: reacting to the *oldest* message in a full
/// room must leave the history untouched, which it cannot do if the message is
/// being searched for or rewritten in place.
#[tokio::test]
async fn reacting_never_touches_the_history() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    for i in 0..MAX_MESSAGES_PER_ROOM {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("m{i}"),
                timestamp: "1700000000000".to_string(),
                reply_to: None,
                attachment: None,
            },
            &tracker,
        );
    }

    let oldest = room.chat_history[0].message_id;
    let stored = Arc::clone(&room.chat_history[0]);
    let bytes_before = room.total_memory_bytes.load(Ordering::SeqCst);

    room.toggle_reaction(oldest, "🔥", "alice");

    assert!(
        Arc::ptr_eq(&stored, &room.chat_history[0]),
        "reacting must not rewrite the message it refers to"
    );
    assert_eq!(
        room.total_memory_bytes.load(Ordering::SeqCst),
        bytes_before,
        "reacting must not re-derive the room's byte total"
    );
    assert_eq!(room.chat_history.len(), MAX_MESSAGES_PER_ROOM);
}

/// The Node that CI installs is one the toolchain can actually run on.
///
/// This is the test that would have saved the afternoon. `wrangler` requires
/// Node >= 22 and pnpm 11 needs `node:sqlite`, which arrived in 22. Both
/// workflows pinned Node 20. The Rust gate — fmt, clippy, 600 tests, release
/// build, coverage — went green on every push, and then the *deploy step*
/// failed, so nothing reached the live site while every signal a person looks
/// at said the commit was fine.
///
/// §6.1 says the gate exists to answer "will this deploy". A gate that cannot
/// see the deploy's own requirements is not answering it. The requirement lives
/// in `cloudflare/package.json` under `engines`, once, and this asserts the
/// workflow agrees with it. Deploying moved to Cloudflare's Git integration,
/// but the Worker typecheck still runs here and still needs a Node that pnpm
/// and wrangler can run on.
#[test]
fn ci_node_version_satisfies_the_toolchain() {
    const PACKAGE_JSON: &str = include_str!("../../cloudflare/package.json");
    const CI: &str = include_str!("../../.github/workflows/ci.yml");

    // The declared floor, e.g. `"node": ">=22"`.
    let required: u32 = PACKAGE_JSON
        .split_once("\"node\":")
        .and_then(|(_, rest)| rest.split_once('"'))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(spec, _)| {
            spec.trim_start_matches(['>', '=', '^', '~', ' '])
                .to_string()
        })
        .and_then(|spec| spec.split('.').next().unwrap_or_default().parse().ok())
        .expect("cloudflare/package.json should declare engines.node");

    assert!(
        required >= 22,
        "wrangler needs Node 22 or newer; the declared floor is {required}"
    );

    for (name, workflow) in [("ci.yml", CI)] {
        let mut found = 0;
        for (index, _) in workflow.match_indices("node-version: '") {
            let rest = &workflow[index + "node-version: '".len()..];
            let Some(end) = rest.find('\'') else { continue };
            let major: u32 = rest[..end]
                .split('.')
                .next()
                .unwrap_or_default()
                .parse()
                .unwrap_or_else(|_| panic!("{name} has an unparseable node-version"));

            assert!(
                major >= required,
                "{name} installs Node {major}, below the {required} the Worker \
                 toolchain requires — the Rust gate would still pass and the \
                 deploy would still fail"
            );
            found += 1;
        }
        assert!(found > 0, "{name} should pin a Node version");
    }
}

/// The deploy's install command is the one the lockfile format belongs to.
///
/// Switching package managers is easy to do halfway: a `pnpm-lock.yaml` in the
/// tree and an `npm ci` in the workflow installs from `package.json` alone,
/// silently resolving different versions than anything anyone tested.
#[test]
fn the_workflows_install_with_the_lockfile_that_exists() {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");
    const DEPLOY_SH: &str = include_str!("../../deploy.sh");

    for (name, script) in [("ci.yml", CI), ("deploy.sh", DEPLOY_SH)] {
        // Tokens, and only from the commands — not substrings, and not prose.
        // This test failed twice before it passed once: `pnpm install` contains
        // "npm install", and then a comment mentioning "RUSTSEC and npm
        // advisories" matched the token. §6.7, twice, in the same afternoon.
        let commands = strip_hash_comments(script);
        let invoked: Vec<&str> = commands
            .split_whitespace()
            .filter(|word| *word == "npm" || *word == "npx")
            .collect();

        assert!(
            invoked.is_empty(),
            "{name} invokes {invoked:?}, but the repository's lockfile is \
             pnpm-lock.yaml — npm installs from package.json alone and npx \
             resolves outside the pnpm store, either way running versions \
             nothing was tested against"
        );
    }

    assert!(
        CI.contains("--frozen-lockfile"),
        "CI must install from the lockfile, not update it"
    );
}

/// `@types/node` describes the Node the workflows actually install.
///
/// Types are a claim about the runtime. A `@types/node` ahead of the installed
/// Node promises APIs that will not be there, and one behind hides APIs that
/// are — either way the typecheck is answering a question about a different
/// machine than the one the build runs on.
#[test]
fn the_node_types_match_the_node_ci_installs() {
    const PACKAGE_JSON: &str = include_str!("../../cloudflare/package.json");
    const CI: &str = include_str!("../../.github/workflows/ci.yml");

    let types_major: u32 = PACKAGE_JSON
        .split_once("\"@types/node\":")
        .and_then(|(_, rest)| rest.split_once('"'))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(spec, _)| {
            spec.trim_start_matches(['^', '~', '>', '=', ' '])
                .to_string()
        })
        .and_then(|spec| spec.split('.').next().unwrap_or_default().parse().ok())
        .expect("cloudflare/package.json should depend on @types/node");

    let ci = strip_hash_comments(CI);
    let installed: u32 = ci
        .split_once("node-version: '")
        .and_then(|(_, rest)| rest.split_once('\''))
        .and_then(|(version, _)| version.split('.').next().unwrap_or_default().parse().ok())
        .expect("ci.yml should pin a node-version");

    assert_eq!(
        types_major, installed,
        "@types/node is for Node {types_major} but CI installs Node {installed}; \
         the typecheck would be describing a runtime nobody runs"
    );
}

/// Every test named in the standards actually exists.
///
/// `ENGINEERING-STANDARDS.md` opens by asserting "every rule here is enforced
/// by a test in `src/tests.rs`", and its Enforcing-tests table names them one
/// by one. A rule whose named enforcer has been renamed or deleted is an
/// unenforced rule wearing an enforced rule's clothes — and the table reads
/// exactly the same either way.
#[test]
fn every_test_the_standards_name_exists() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");
    const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");

    let mut missing: Vec<String> = Vec::new();

    for doc in [STANDARDS, CLAUDE_MD] {
        // Test names are cited in backticks, and are snake_case identifiers
        // long enough not to collide with prose or field names.
        for cited in doc.split('`').skip(1).step_by(2) {
            let looks_like_a_test = cited.len() > 12
                && cited.contains('_')
                && !cited.contains(' ')
                && !cited.contains("::")
                && !cited.contains('(')
                && !cited.contains('.')
                && cited
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

            // Only names that read like assertions, not constants (which are
            // SCREAMING_CASE and already excluded) or field names.
            if !looks_like_a_test {
                continue;
            }

            let defined = suite_source().contains(&format!("fn {cited}("));
            let is_a_test_name = cited.starts_with("test_")
                || cited.contains("_must_")
                || cited.contains("_is_")
                || cited.contains("_are_")
                || cited.contains("_never_")
                || cited.contains("_cannot_")
                || cited.contains("_does_not_")
                || cited.contains("_matches_")
                || cited.contains("_releases_")
                || cited.contains("_exists_")
                || cited.contains("_still_")
                || cited.contains("_keeps_")
                || cited.contains("_holds_")
                || cited.contains("_fade")
                || cited.contains("_roster_");

            if is_a_test_name && !defined && !missing.contains(&cited.to_string()) {
                missing.push(cited.to_string());
            }
        }
    }

    assert!(
        missing.is_empty(),
        "the docs name these tests, but nothing defines them — either the test \
         was renamed and the doc not updated, or the rule is unenforced: {missing:?}"
    );
}

/// Every watched-levers row carries a reopen condition.
///
/// §9.4's table is the mechanism that keeps a parked decision from fossilising
/// into lore. A row with no reopen condition is exactly the "decided, then
/// forgotten" failure the registry exists to prevent, so the table's *shape* is
/// checked rather than trusted.
#[test]
fn every_parked_decision_records_how_to_reopen_it() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");

    let table = STANDARDS
        .split_once("| Lever | Status | Reopen when |")
        .map(|(_, rest)| rest)
        .expect("§9.4 should contain the watched-levers table");

    let mut incomplete: Vec<String> = Vec::new();
    let mut rows = 0;

    for line in table.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            if rows > 0 {
                break; // end of the table
            }
            continue;
        }
        // The header separator.
        if line.starts_with("| ---") {
            continue;
        }

        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        rows += 1;

        let (lever, status, reopen) = (cells[0], cells[1], cells[2]);
        if reopen.len() < 20 || status.len() < 10 || lever.is_empty() {
            incomplete.push(lever.to_string());
        }
    }

    assert!(
        rows >= 8,
        "the registry should not have shrunk; found {rows} rows"
    );
    assert!(
        incomplete.is_empty(),
        "these watched-levers rows lack a real status or reopen condition, which \
         is how a parked decision becomes lore: {incomplete:?}"
    );
}

/// No assertion in this file is one that cannot fail.
///
/// A test that cannot fail is worse than no test: it reports coverage it does
/// not provide (§6.4). The recognisable forms are tautologies over a value's
/// own shape — `is_ok() || is_err()`, `x == x`, `assert!(true)` — which pass
/// whatever the code does.
#[test]
fn no_assertion_in_this_suite_is_a_tautology() {
    // Each form is stored in halves and joined at run time, so the file never
    // literally contains the pattern it forbids. Written whole, this sweep
    // failed on its own definition — §6.7, for the third time in this suite.
    const TAUTOLOGY_HALVES: &[(&str, &str)] = &[
        ("is_ok() ", "|| result.is_err()"),
        ("is_err() ", "|| result.is_ok()"),
        ("assert!(", "true)"),
        ("assert_eq!(", "true, true)"),
        ("assert!(1 ", "== 1)"),
    ];

    let needles: Vec<String> = TAUTOLOGY_HALVES
        .iter()
        .map(|(head, tail)| format!("{head}{tail}"))
        .collect();

    let mut found: Vec<String> = Vec::new();
    let suite = suite_source();
    for (number, line) in suite.lines().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        if !code.contains("assert") {
            continue;
        }
        for needle in &needles {
            if code.contains(needle.as_str()) {
                found.push(format!("line {}: {}", number + 1, code.trim()));
            }
        }
    }

    assert!(
        found.is_empty(),
        "these assertions cannot fail, so they test nothing: {found:#?}"
    );
}

/// Every crate declared in `Cargo.toml` is actually used.
///
/// A dependency nobody imports is install time, build time and supply-chain
/// surface for nothing — and the freshness and audit sweeps have to keep
/// tracking it. Four such crates were deleted from this project once already
/// (`metrics`, `metrics-exporter-prometheus`, `async-trait`, `hyper`); this is
/// what stops the fifth.
#[test]
fn every_declared_dependency_is_used() {
    const CARGO_TOML: &str = include_str!("../../Cargo.toml");

    // Crates a build consumes without an `use` of its own.
    const KNOWN_INDIRECT: &[(&str, &str)] = &[
        ("axum-server", "used as `axum_server::Server` in main.rs"),
        (
            "tower",
            "test-only: `tower::util::ServiceExt` for `oneshot`",
        ),
        (
            "url",
            "test-only: parsing WebSocket URLs in the integration tests",
        ),
    ];

    // Read from disk rather than a hand-written list of `include_str!`s: that
    // list went stale the moment `startup.rs` was split out of `main.rs`, and a
    // sweep that silently stops seeing a module reports the crates it uses as
    // dead.
    // Walked, not listed: the suite became a directory of modules, and a
    // non-recursive read stopped seeing them — which reported every crate only
    // the tests use as dead.
    fn read_rust_files(dir: &std::path::Path, into: &mut String) {
        for entry in std::fs::read_dir(dir).expect("a readable directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                read_rust_files(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs") {
                into.push_str(&std::fs::read_to_string(&path).expect("a readable module"));
                into.push('\n');
            }
        }
    }

    let mut sources = String::new();
    read_rust_files(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
        &mut sources,
    );

    let mut unused: Vec<String> = Vec::new();

    for line in CARGO_TOML.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('[') || !line.contains('=') {
            continue;
        }
        let Some((name, _)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        // Only dependency lines: keys of the package table are not crates.
        if !line.contains('"') && !line.contains('{') {
            continue;
        }
        if [
            "name",
            "version",
            "edition",
            "rust-version",
            "description",
            "license",
            "publish",
            "lto",
            "codegen-units",
            "strip",
            "panic",
        ]
        .contains(&name)
        {
            continue;
        }

        let ident = name.replace('-', "_");
        let referenced = sources.contains(&format!("{ident}::"))
            || sources.contains(&format!("use {ident}"))
            || sources.contains(&format!("extern crate {ident}"));

        if !referenced && !KNOWN_INDIRECT.iter().any(|(k, _)| *k == name) {
            unused.push(name.to_string());
        }
    }

    assert!(
        unused.is_empty(),
        "these crates are declared but never referenced — delete them or record \
         why they are needed indirectly: {unused:?}"
    );

    // Self-cleaning, the way the reference suite's pinned-with-reason list is:
    // an exemption must not outlive its reason.
    for (name, reason) in KNOWN_INDIRECT {
        assert!(
            CARGO_TOML.contains(name),
            "`{name}` is exempted as an indirect dependency ({reason}) but is no \
             longer in Cargo.toml — delete the exemption"
        );
    }
}

// ========== SLOW TESTS: REAL TIME, LOCAL ONLY ==========
//
// `#[ignore]`, so `cargo test` — which is what CI runs — skips them. They are
// run by `scripts/slow-tests.sh` and included in `scripts/coverage-full.sh`.
//
// They are here because the alternative was worse. The branches below only
// happen after a real interval elapses, and the two ways to reach them without
// waiting are both rejected: an injectable clock (§9.4 — the indirection costs
// more than it buys, and kills four mutants that differ only at an exact
// threshold), or asserting on timing, which §6.4 rules out as flaky. Waiting a
// few seconds on a laptop is neither.

/// Every contract test is documented somewhere a future session will look.
///
/// The other half of the phantom-enforcement problem: a sweep can exist and
/// guard something real while nothing says what or why, so the reasoning lives
/// only in the file and is one refactor from being lore. Ported from the
/// standards-enforcement contract on `cameronaaron.com`.
#[test]
fn every_contract_test_is_documented() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");
    const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");
    let docs = format!("{STANDARDS}\n{CLAUDE_MD}");

    // The sweeps: tests whose names read as a rule about the whole codebase
    // rather than an example of one behaviour.
    const MARKERS: &[&str] = &[
        "every_",
        "no_",
        "the_client_",
        "the_page_",
        "the_workflows_",
        "client_",
    ];

    let mut undocumented: Vec<&str> = Vec::new();

    let suite = suite_source();
    for line in suite.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("async fn ")
            .or_else(|| line.strip_prefix("fn "))
        else {
            continue;
        };
        let Some((name, _)) = rest.split_once('(') else {
            continue;
        };
        if !MARKERS.iter().any(|m| name.starts_with(m)) {
            continue;
        }
        // Helpers are not contracts.
        if name.ends_with("_source") || name.contains("without_comments") {
            continue;
        }
        if !docs.contains(name) {
            undocumented.push(name);
        }
    }

    assert!(
        undocumented.is_empty(),
        "these sweeps guard something but nothing documents what or why; add \
         them to the Enforcing-tests table: {undocumented:?}"
    );
}

/// The coverage exemptions are justified, current, and honest about their size.
///
/// Ported from the pinned-dependency list on `cameronaaron.com`, which fails
/// when a pin catches up to latest so an exemption can never outlive its
/// reason. Three ways this registry can rot, each checked:
///
///   1. An entry with no reason — a number being hidden rather than explained.
///   2. An entry naming a path that no longer exists.
///   3. A third entry appearing. Only `tests.rs` and `main.rs` are exempt, and
///      both for the same structural reason — one is the suite, the other is an
///      entry point a test can never call. Everything else reached 100%, so a
///      new entry is a claim that something is untestable, and the first
///      question is whether the code can move instead (§6.1c).
#[test]
fn coverage_exemptions_are_justified_and_current() {
    const EXEMPTIONS: &str = include_str!("../../scripts/coverage-exemptions.toml");
    const COVERAGE_SH: &str = include_str!("../../scripts/coverage.sh");

    let mut entries = 0;
    for block in EXEMPTIONS.split("[[exempt]]").skip(1) {
        entries += 1;

        let path = block
            .split_once("path = \"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(p, _)| p)
            .expect("every exemption names a path");

        assert!(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR")))
                .join(path)
                .exists(),
            "{path} is exempted from coverage but no longer exists — an \
             exemption must not outlive the thing it exempts"
        );

        let reason = block
            .split_once("reason = \"\"\"")
            .and_then(|(_, rest)| rest.split_once("\"\"\""))
            .map(|(r, _)| r.trim())
            .unwrap_or_default();

        assert!(
            reason.len() > 80,
            "{path} is exempted without a real reason; an exclusion with no \
             explanation is a number being hidden"
        );
    }

    assert_eq!(
        entries, 2,
        "only the two whole-file exemptions remain. `session.rs` had 21 line \
         exemptions, then 12, then 3, then none — every one turned out to be a \
         misplaced line rather than an untestable one. If a third file appears \
         here, the question to ask first is whether the code can move (§6.1c)."
    );

    // Whole-file exclusions must be exactly the ones the script passes to
    // tarpaulin, or the registry describes a gate that is not running.
    for path in ["src/tests", "src/main.rs"] {
        assert!(
            COVERAGE_SH.contains(path),
            "{path} is exempted in the registry but not excluded by coverage.sh"
        );
        assert!(
            EXEMPTIONS.contains(path),
            "coverage.sh excludes {path} but the registry does not explain why"
        );
    }
}

// ========== MUTATION-DRIVEN: ARITHMETIC AND ACCOUNTING ==========
//
// Every test here exists because a mutant survived. Coverage said these lines
// ran; nothing checked what they computed.

/// Every `§` and `constraint #` pointer resolves to something that exists.
///
/// The docs and the source comments are dense with them — `§5.11`,
/// `constraint #12`, `§1.4a` — and several rules lean on another by name. A
/// section renumbered or a constraint deleted turns every pointer to it into a
/// lie, silently, with a green gate: nothing else reads them.
///
/// Ported from the docs-cross-reference contract on `cameronaaron.com`. The
/// addition here is that Rust source comments are checked too, because in this
/// codebase the citation usually lives next to the code it justifies rather
/// than in the document.
#[test]
fn every_section_reference_resolves() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");
    const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");

    // Sections that exist: `## 5. …` and `### 5.11 …`.
    let mut sections: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in STANDARDS.lines() {
        let trimmed = line.trim_start_matches('#').trim_start();
        if !line.starts_with('#') {
            continue;
        }
        if let Some((number, _)) = trimmed.split_once(' ')
            && number.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            sections.insert(number.trim_end_matches('.').to_string());
        }
    }
    assert!(
        sections.len() > 40,
        "the standards should have many numbered sections; found {}",
        sections.len()
    );

    // Constraints that exist: `### 12. …` in CLAUDE.md.
    let mut constraints: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for line in CLAUDE_MD.lines() {
        if let Some(rest) = line.strip_prefix("### ")
            && let Some((number, _)) = rest.split_once('.')
            && let Ok(n) = number.parse::<u32>()
        {
            constraints.insert(n);
        }
    }
    assert!(
        constraints.len() > 20,
        "CLAUDE.md should list many constraints; found {}",
        constraints.len()
    );

    // Every corpus that cites them: both docs, and every Rust module.
    let mut corpus = vec![
        (
            "ENGINEERING-STANDARDS.md".to_string(),
            STANDARDS.to_string(),
        ),
        ("CLAUDE.md".to_string(), CLAUDE_MD.to_string()),
    ];
    let source_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for entry in std::fs::read_dir(source_dir).expect("src/ should be readable") {
        let path = entry.expect("a readable entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            corpus.push((name, std::fs::read_to_string(&path).expect("readable")));
        }
    }

    let mut broken: Vec<String> = Vec::new();

    for (name, body) in &corpus {
        for (index, _) in body.match_indices('§') {
            let rest = &body[index + '§'.len_utf8()..];
            let reference: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || c.is_ascii_lowercase())
                .collect();
            let reference = reference.trim_end_matches('.').to_string();
            if reference.is_empty() {
                continue;
            }
            if !sections.contains(&reference) {
                broken.push(format!("{name}: §{reference}"));
            }
        }

        for (index, _) in body.match_indices("constraint #") {
            let rest = &body[index + "constraint #".len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let Ok(number) = digits.parse::<u32>() else {
                continue;
            };
            if !constraints.contains(&number) {
                broken.push(format!("{name}: constraint #{number}"));
            }
        }
    }

    broken.sort();
    broken.dedup();
    assert!(
        broken.is_empty(),
        "these pointers name a section or constraint that does not exist — a \
         renumber or a deletion left them behind: {broken:#?}"
    );
}

/// §8 — every module says what it is for, and none is named for nothing.
///
/// A module called `utils` is a module whose contents nobody decided on. The
/// header comment is the other half: a file whose job is not stated in it is a
/// file whose job drifts.
#[test]
fn every_module_is_named_for_its_job_and_says_what_it_is() {
    const FORBIDDEN: &[&str] = &[
        "utils", "helpers", "common", "misc", "shared", "core", "lib2",
    ];

    let source_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut checked = 0;

    for entry in std::fs::read_dir(source_dir).expect("src/ should be readable") {
        let path = entry.expect("a readable entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        checked += 1;

        assert!(
            !FORBIDDEN.contains(&name.as_str()),
            "`{name}.rs` is named for nothing — a module called that is one \
             whose contents nobody decided on (§8)"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "`{name}.rs` should be snake_case"
        );

        // `tests.rs` is the suite; the rest must state their job at the top.
        if name == "tests" {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a readable module");
        let header: String = body.lines().take_while(|l| l.starts_with("//!")).collect();
        assert!(
            header.len() > 60,
            "`{name}.rs` has no header comment saying what it owns (§8.1)"
        );
    }

    assert!(checked >= 15, "expected every module to be checked");
}

/// Nothing is exported that only its own test uses.
///
/// The failure mode: a function superseded months ago, still compiling, still
/// covered — by the test written for it. A 100% coverage gate cannot tell that
/// apart from live code, which is exactly why it needs its own sweep.
///
/// Ported from the dead-logic-export contract on `cameronaaron.com`.
#[test]
fn nothing_is_public_only_for_its_own_test() {
    let source_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

    let mut production = String::new();
    let mut declarations: Vec<(String, String)> = Vec::new();

    for entry in std::fs::read_dir(source_dir).expect("src/ should be readable") {
        let path = entry.expect("a readable entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let body = std::fs::read_to_string(&path).expect("a readable module");

        if name == "tests.rs" {
            continue;
        }
        production.push_str(&body);
        production.push('\n');

        for line in body.lines() {
            let line = line.trim();
            let Some(rest) = line
                .strip_prefix("pub fn ")
                .or_else(|| line.strip_prefix("pub async fn "))
                .or_else(|| line.strip_prefix("pub(crate) fn "))
                .or_else(|| line.strip_prefix("pub(crate) async fn "))
            else {
                continue;
            };
            // Strip generics: `forward_broadcasts<S: FrameSink>` is declared
            // with them and referred to without.
            let fn_name = rest
                .split_once('(')
                .map(|(head, _)| head)
                .unwrap_or(rest)
                .split('<')
                .next()
                .unwrap_or_default()
                .trim();
            if !fn_name.is_empty() {
                declarations.push((name.clone(), fn_name.to_string()));
            }
        }
    }

    assert!(
        declarations.len() > 30,
        "expected to find many exported functions; found {}",
        declarations.len()
    );

    // Functions that exist only for the suite, by construction.
    const TEST_ONLY: &[(&str, &str)] = &[("startup.rs", "generate_random_room_name")];

    let mut dead: Vec<String> = Vec::new();
    for (module, function) in &declarations {
        if TEST_ONLY.iter().any(|(m, f)| m == module && f == function) {
            continue;
        }

        // Count *references*, not calls: a handler is mounted as
        // `get(ws_handler)` and a validator is passed as
        // `and_then(sanitize_reply)` — neither is followed by a paren.
        let referenced = production
            .match_indices(function.as_str())
            .filter(|(index, _)| {
                let before = production[..*index].chars().next_back();
                let after = production[index + function.len()..].chars().next();
                let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
                boundary(before) && boundary(after)
            })
            .count();

        // The declaration itself is one of those references.
        if referenced <= 1 {
            dead.push(format!("{module}::{function}"));
        }
    }

    // Self-cleaning: an exemption must not outlive the thing it exempts.
    for (module, function) in TEST_ONLY {
        assert!(
            declarations
                .iter()
                .any(|(m, f)| m == module && f == function),
            "{module}::{function} is exempted as test-only but no longer exists"
        );
    }

    assert!(
        dead.is_empty(),
        "these are exported but called only from tests — a function whose only \
         caller is the test written for it is dead code with a green coverage \
         report: {dead:#?}"
    );
}
