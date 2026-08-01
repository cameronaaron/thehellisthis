//! What the logs say, treated as a contract.
//!
//! An operator reads the log to understand a running server: who arrived, who
//! left, which room faded, why a connection was refused. None of that is
//! checked by any behavioural test — the server can do exactly the right thing
//! and say nothing about it, and that is indistinguishable from a server doing
//! the wrong thing quietly.
//!
//! It is not a hypothetical. The reported "skink left / stinks joined" bug ran
//! for as long as it did because *nothing said why a visitor got a new name*.
//! Every step behaved correctly; the log recorded arrivals and departures and
//! never the decision between them. `admit_user` now logs its outcome —
//! reclaimed, recognised or fresh — and the test below is the reason it cannot
//! quietly stop.

use super::*;

/// Every admission says what happened to the visitor's identity.
///
/// Three outcomes, and the difference between them is the whole of §5.9a:
/// `reclaimed` is a reconnecting user taking their slot back, `recognised` is
/// somebody whose cookie is good but who is new to this room, and `fresh` is a
/// visitor with no usable identity at all. A room repeating `fresh` for the
/// same person is the flapping bug, visible at a glance.
#[tokio::test]
async fn every_admission_says_what_became_of_the_identity() {
    let state = Arc::new(AppState::new());

    // No cookie: a brand new visitor.
    let logged = capturing_logs(|| async {
        admit_user(&state, "watched-room", "c1", None).await;
    })
    .await;
    assert!(
        logged.contains("admitted") && logged.contains("fresh"),
        "a visitor with no identity must be logged as fresh: {logged}"
    );

    // A good cookie for a room they have not been in: recognised, and issued a
    // slot of their own — under the same id the cookie carried in. That id is
    // what the client compares its own messages against, so a visitor's own
    // history in this room only keeps rendering as theirs if it matches.
    let carried = crate::identity::UserCookie {
        user_id: Uuid::new_v4().to_string(),
        animal_name: ANIMAL_NAMES[0].to_string(),
    };
    let mut issued = None;
    let logged = capturing_logs(|| async {
        issued = admit_user(&state, "watched-room", "c2", Some(&carried)).await;
    })
    .await;
    assert!(
        logged.contains("recognised"),
        "a visitor with a usable cookie must be logged as recognised: {logged}"
    );
    assert_eq!(
        issued.as_ref().map(|(id, _)| id.as_str()),
        Some(carried.user_id.as_str()),
        "recognised must keep the cookie's id, not mint a new one"
    );

    // Coming back with what the server actually set, which is what a browser
    // sends — and, since recognised now keeps the cookie's id, the same one it
    // arrived with.
    let (issued_id, issued_name) = issued.expect("admitted");
    let returning = crate::identity::UserCookie {
        user_id: issued_id,
        animal_name: issued_name,
    };
    let logged = capturing_logs(|| async {
        admit_user(&state, "watched-room", "c3", Some(&returning)).await;
    })
    .await;
    assert!(
        logged.contains("reclaimed"),
        "a reconnecting visitor must be logged as reclaiming, not as new — the \
         difference is exactly what a room experiences as somebody leaving and \
         a stranger arriving: {logged}"
    );
}

/// A departure is logged, with what is left behind.
#[tokio::test]
async fn a_departure_is_logged_with_the_room_it_leaves() {
    let state = Arc::new(AppState::new());
    let (user_id, _) = admit_user(&state, "leaving", "conn-1", None)
        .await
        .expect("admitted");

    let logged = capturing_logs(|| async {
        cleanup_user(&state, "leaving", &user_id, "conn-1", None).await;
    })
    .await;

    assert!(
        logged.contains("departed") && logged.contains("leaving"),
        "a departure must name the room: {logged}"
    );
    assert!(
        logged.contains("remaining"),
        "and say what is left, which is what tells an operator whether the room \
         is about to fade: {logged}"
    );
}

/// A refused connection says which ceiling refused it.
///
/// Before this, a refusal was silent: the visitor saw an error and the server
/// recorded nothing at all, so "why can nobody connect" had no answer in the
/// logs. Each refusal now names the limit and its configured value, because
/// the useful question is always whether the limit is right rather than
/// whether it fired.
#[tokio::test]
async fn a_refused_connection_says_which_ceiling_refused_it() {
    let state = Arc::new(AppState::new());
    state
        .resource_monitor
        .total_connections
        .store(MAX_CONCURRENT_USERS, Ordering::SeqCst);

    let logged = capturing_logs(|| async {
        let refused = crate::session::check_admission(&state, None, "full").await;
        assert!(refused.is_err(), "the server is at capacity");
    })
    .await;

    assert!(
        logged.contains("refused: server at capacity"),
        "a refusal must say which ceiling it hit: {logged}"
    );
    assert!(
        logged.contains(&MAX_CONCURRENT_USERS.to_string()),
        "and what that ceiling is set to, since that is the actionable part: \
         {logged}"
    );
}

/// A full room says so, rather than failing silently.
#[tokio::test]
async fn a_full_room_says_so_when_it_refuses() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for (i, animal) in ANIMAL_NAMES.iter().take(MAX_USERS_PER_ROOM).enumerate() {
            room.users.insert(
                format!("u{i}"),
                connected_user(&format!("u{i}"), animal, "c", Instant::now()),
            );
        }
        rooms.insert("packed".to_string(), room);
    }

    let logged = capturing_logs(|| async {
        assert!(
            admit_user(&state, "packed", "c1", None).await.is_none(),
            "the room is full"
        );
    })
    .await;

    assert!(
        logged.contains("room is full") || logged.contains("refused"),
        "a full room must say so rather than refusing silently: {logged}"
    );
}

/// A session that comes and goes leaves a readable trail.
///
/// This is the shape an operator actually reads: one visitor, one arrival, one
/// departure, and nothing in between claiming otherwise. It is also the shape
/// the flapping bug broke — that log had an arrival and a departure per
/// *reconnect*, each with a different name.
#[tokio::test]
async fn a_normal_session_reads_as_one_arrival_and_one_departure() {
    let state = Arc::new(AppState::new());

    let logged = capturing_logs(|| async {
        let (user_id, _) = admit_user(&state, "quiet", "conn-1", None)
            .await
            .expect("admitted");
        cleanup_user(&state, "quiet", &user_id, "conn-1", None).await;
    })
    .await;

    assert_eq!(
        logged.matches("admitted").count(),
        1,
        "one visitor is one arrival: {logged}"
    );
    assert_eq!(
        logged.matches("departed").count(),
        1,
        "and one departure: {logged}"
    );
    assert!(!logged.contains("refused"), "and nothing refused: {logged}");
}

// ========== MULTI-USER SIMULATION, ASSERTED FROM THE LOG ==========
//
// The tests above check that one event logs the right thing. These drive a
// sequence of events the way real users would — several people joining,
// talking, reacting and leaving the same room — and read the outcome back out
// of the log rather than out of return values, which is exactly the gap
// between "the code did the right thing" and "an operator could have told".

/// Two people join, one sends a message the other reacts to, then both leave —
/// the log alone must tell that whole story in order.
#[tokio::test]
async fn a_join_chat_react_leave_sequence_reads_back_from_the_log() {
    let state = Arc::new(AppState::new());

    let logged = capturing_logs_at(tracing::Level::TRACE, || async {
        let (alice, alice_name) = admit_user(&state, "lounge", "conn-a", None)
            .await
            .expect("alice admitted");
        let (bob, _bob_name) = admit_user(&state, "lounge", "conn-b", None)
            .await
            .expect("bob admitted");

        apply_client_event(
            &state,
            "lounge",
            &alice,
            &alice_name,
            ClientEvent::Message {
                text: "hey".to_string(),
                reply_to: None,
                attachment: None,
            },
        )
        .await;

        let message_id = {
            let rooms = state.rooms.read().await;
            rooms.get("lounge").unwrap().chat_history[0]
                .message_id
                .to_string()
        };

        apply_client_event(
            &state,
            "lounge",
            &bob,
            "bob",
            ClientEvent::React {
                message_id,
                emoji: "🔥".to_string(),
            },
        )
        .await;

        cleanup_user(&state, "lounge", &alice, "conn-a", None).await;
        cleanup_user(&state, "lounge", &bob, "conn-b", None).await;
    })
    .await;

    assert_eq!(logged.matches("admitted").count(), 2, "two joins: {logged}");
    assert!(
        logged.contains("message broadcast"),
        "the chat message must be visible: {logged}"
    );
    assert!(
        logged.contains("reaction toggled"),
        "and so must the reaction it drew: {logged}"
    );
    assert_eq!(
        logged.matches("departed").count(),
        2,
        "and both departures, since a room an operator watches must show \
         exactly as many leaves as arrivals: {logged}"
    );
}

/// A rejoin inside the room's cooldown is throttled, and the log says so
/// rather than the visitor just silently failing to reconnect.
#[tokio::test]
async fn a_rejoin_inside_the_cooldown_is_logged_as_throttled() {
    let state = Arc::new(AppState::new());
    let (user_id, animal_name) = admit_user(&state, "flappy", "conn-1", None)
        .await
        .expect("first join");

    let cookie = crate::identity::UserCookie {
        user_id,
        animal_name,
    };

    // The first join does not touch `can_join_room` at all — it is a brand
    // new user, admitted through the `None` arm of `is_user_allowed`. Every
    // rejoin after that counts against the same user's join-attempt budget,
    // so it takes `MAX_ROOM_JOIN_ATTEMPTS` of them, not one, to exhaust it —
    // a flapping connection reconnecting rapidly, not an ordinary reload.
    for n in 0..MAX_ROOM_JOIN_ATTEMPTS {
        let admitted = admit_user(&state, "flappy", "conn-2", Some(&cookie)).await;
        assert!(admitted.is_some(), "rejoin {n} is still within budget");
    }

    let logged = capturing_logs_at(tracing::Level::DEBUG, || async {
        let refused = admit_user(&state, "flappy", "conn-2", Some(&cookie)).await;
        assert!(
            refused.is_none(),
            "MAX_ROOM_JOIN_ATTEMPTS is exhausted by this point"
        );
    })
    .await;

    assert!(
        logged.contains("rejoin attempts exhausted this window"),
        "a throttled rejoin must say so, not just silently refuse: {logged}"
    );
}

/// The same text sent twice in a row is recognised as a duplicate, and the log
/// names the sender rather than only counting a dropped message.
#[tokio::test]
async fn a_duplicate_message_is_named_in_the_log() {
    let state = Arc::new(AppState::new());
    let (user_id, animal_name) = admit_user(&state, "echo", "conn-1", None)
        .await
        .expect("admitted");

    let send = || ClientEvent::Message {
        text: "same thing twice".to_string(),
        reply_to: None,
        attachment: None,
    };

    apply_client_event(&state, "echo", &user_id, &animal_name, send()).await;

    let logged = capturing_logs_at(tracing::Level::DEBUG, || async {
        apply_client_event(&state, "echo", &user_id, &animal_name, send()).await;
    })
    .await;

    assert_eq!(
        {
            let rooms = state.rooms.read().await;
            rooms.get("echo").unwrap().chat_history.len()
        },
        1,
        "the duplicate must not be stored twice"
    );
    assert!(
        logged.contains("duplicate") && logged.contains(&user_id),
        "the log must name who sent the duplicate, not just that one was \
         dropped: {logged}"
    );
}

/// A reconnect keeps the same identity and logs as one, not as a departure
/// paired with a fresh arrival — the exact shape of the flapping bug §5.9a
/// exists to prevent.
#[tokio::test]
async fn a_reconnect_under_load_still_reads_as_one_identity() {
    let state = Arc::new(AppState::new());
    let (user_id, animal_name) = admit_user(&state, "steady", "conn-1", None)
        .await
        .expect("first join");

    let cookie = crate::identity::UserCookie {
        user_id: user_id.clone(),
        animal_name: animal_name.clone(),
    };

    // The old connection tears down only after the new one is already in,
    // exactly as a real reconnect races: the browser opens the new socket
    // before the server notices the old one is gone.
    let logged = capturing_logs(|| async {
        let (reconnected_id, _) = admit_user(&state, "steady", "conn-2", Some(&cookie))
            .await
            .expect("reconnect admitted");
        assert_eq!(
            reconnected_id, user_id,
            "identity must survive the reconnect"
        );
        cleanup_user(&state, "steady", &user_id, "conn-1", None).await;
    })
    .await;

    assert!(
        logged.contains("reclaimed"),
        "a reconnect must be logged as reclaiming its identity: {logged}"
    );
    assert!(
        !logged.contains("departed"),
        "the superseded teardown must not announce a departure for a user who \
         is still connected under the new socket: {logged}"
    );
}

// ========== RATE LIMITING, ROOM FADE, AND MEMORY PRESSURE ==========

/// Ten messages a minute is the room-level budget; the eleventh is throttled
/// and the log says so.
#[tokio::test]
async fn exceeding_the_message_budget_is_logged() {
    let state = Arc::new(AppState::new());
    let (user_id, animal_name) = admit_user(&state, "busy", "conn-1", None)
        .await
        .expect("admitted");

    for i in 0..MAX_MESSAGES_PER_WINDOW {
        apply_client_event(
            &state,
            "busy",
            &user_id,
            &animal_name,
            ClientEvent::Message {
                text: format!("message {i}"),
                reply_to: None,
                attachment: None,
            },
        )
        .await;
    }

    let logged = capturing_logs_at(tracing::Level::DEBUG, || async {
        apply_client_event(
            &state,
            "busy",
            &user_id,
            &animal_name,
            ClientEvent::Message {
                text: "one too many".to_string(),
                reply_to: None,
                attachment: None,
            },
        )
        .await;
    })
    .await;

    assert!(
        logged.contains("rate") || logged.contains("throttle") || logged.contains("limit"),
        "an over-budget message must say why it was refused, not merely fail \
         to appear: {logged}"
    );
}

/// A room over its soft memory threshold is pruned, and the log names the
/// threshold it crossed — an operator's only way to tell a busy room from a
/// leak.
#[tokio::test]
async fn a_room_over_the_soft_memory_threshold_logs_the_prune() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let (num, den) = MEMORY_SOFT_LIMIT_RATIO;
    let soft_limit = (MAX_TOTAL_ROOMS_MEMORY * num) / den;
    room.total_memory_bytes
        .store(soft_limit + 1, Ordering::SeqCst);

    let logged = capturing_logs_at(tracing::Level::DEBUG, || async {
        room.trigger_cleanup(&tracker).await;
    })
    .await;

    assert!(
        logged.contains("over soft memory threshold"),
        "a room over budget must say so: {logged}"
    );
    assert!(
        logged.contains(&soft_limit.to_string()),
        "and name the threshold it crossed: {logged}"
    );
}

/// Messages past the retention window are dropped on the housekeeping pass,
/// and the log gives a count rather than a silent shrink.
#[tokio::test]
async fn aged_out_messages_report_a_count_when_dropped() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let old_timestamp = format!("{}", now_ms - MAX_MESSAGE_AGE.as_millis() - 1000);

    let old_msg = OutgoingMessage {
        message_id: Uuid::new_v4(),
        user_id: "u1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>ancient</p>".to_string(),
        timestamp: old_timestamp,
        reply_to: None,
        attachment: None,
    };
    let size = old_msg.estimate_size();
    room.chat_history.push(Arc::new(old_msg));
    room.total_memory_bytes.store(size, Ordering::SeqCst);
    tracker.add_bytes(size);

    let logged = capturing_logs_at(tracing::Level::DEBUG, || async {
        room.cleanup_messages(Instant::now(), &tracker).await;
    })
    .await;

    assert_eq!(room.chat_history.len(), 0, "the aged message is dropped");
    assert!(
        logged.contains("dropped aged messages"),
        "the sweep must say what it dropped and why: {logged}"
    );
}

/// The periodic housekeeping pass reports the process-wide memory total it
/// checked against, every time it runs — not only when the ceiling is hit.
#[tokio::test]
async fn the_periodic_sweep_reports_the_memory_it_checked() {
    let state = Arc::new(AppState::new());

    let logged = capturing_logs_at(tracing::Level::DEBUG, || async {
        state.cleanup().await;
    })
    .await;

    assert!(
        logged.contains("periodic memory sweep"),
        "every sweep must say it ran and what it saw: {logged}"
    );
}

/// Enough suspicious activity from one address earns a ban, and the log names
/// the address, the count, and how long the ban lasts.
#[tokio::test]
async fn a_banned_address_is_named_with_its_count_and_duration() {
    let security = SecurityManager::new();

    let logged = capturing_logs_at(tracing::Level::WARN, || async {
        for _ in 0..=MAX_SUSPICIOUS_EVENTS {
            let _ = security.record_suspicious_activity("10.0.0.7").await;
        }
    })
    .await;

    assert!(
        logged.contains("address banned") && logged.contains("10.0.0.7"),
        "a ban must name the address it was applied to: {logged}"
    );
    assert!(
        logged.contains(&(MAX_SUSPICIOUS_EVENTS + 1).to_string()),
        "and the count it tripped at: {logged}"
    );
    assert!(
        logged.contains(&IP_BAN_DURATION.as_secs().to_string()),
        "and how long the ban lasts: {logged}"
    );
}

/// A room nobody is in is deleted, one that is occupied survives, and `main`
/// is never touched — and the log must draw that same three-way distinction,
/// since an operator reading it has no other way to tell a deleted room from
/// one that is merely quiet.
#[tokio::test]
async fn room_deletion_names_what_died_and_leaves_the_rest_unlogged() {
    let state = Arc::new(AppState::new());
    let long_ago = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(60);

    {
        let mut rooms = state.rooms.write().await;

        let mut abandoned = create_room();
        let mut gone = connected_user("u1", "otter", "c1", Instant::now());
        gone.connection_state = ConnectionState::Disconnected {
            since: Instant::now(),
        };
        abandoned.users.insert("u1".to_string(), gone);
        abandoned.last_activity = long_ago;
        rooms.insert("abandoned".to_string(), abandoned);

        let mut occupied = create_room();
        occupied.users.insert(
            "u2".to_string(),
            connected_user("u2", "badger", "c2", Instant::now()),
        );
        occupied.last_activity = long_ago;
        rooms.insert("occupied".to_string(), occupied);

        let mut main = create_room();
        main.last_activity = long_ago;
        rooms.insert(MAIN_ROOM.to_string(), main);
    }

    let logged = capturing_logs(|| async {
        cleanup_rooms_at(&state, Instant::now()).await;
    })
    .await;

    assert!(
        logged.contains("deleted inactive room") && logged.contains("abandoned"),
        "the deleted room must be named: {logged}"
    );
    assert!(
        !logged.contains("occupied") && !logged.contains(MAIN_ROOM),
        "a room that was not touched must not appear in the log at all — \
         logging every room checked, not just the one deleted, would bury the \
         signal an operator actually wants: {logged}"
    );
}

/// `main` fading — trimming its history hard once idle — is a distinct event
/// from deletion, and must be logged as one: an operator seeing history shrink
/// needs to tell "this room faded, as designed" from "messages are vanishing".
#[tokio::test]
async fn mains_fade_is_logged_as_a_trim_not_a_deletion() {
    let state = Arc::new(AppState::new());
    let long_idle = Instant::now() - MAIN_ROOM_FADE_IDLE - Duration::from_secs(60);

    {
        let mut rooms = state.rooms.write().await;
        let mut main = create_room();
        main.last_activity = long_idle;
        for i in 0..(MAIN_ROOM_FADE_KEEP + 20) {
            main.chat_history.push(Arc::new(OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u1".to_string(),
                animal_name: "Lion".to_string(),
                text: format!("<p>{i}</p>"),
                timestamp: format!("{i}"),
                reply_to: None,
                attachment: None,
            }));
        }
        rooms.insert(MAIN_ROOM.to_string(), main);
    }

    let logged = capturing_logs(|| async {
        cleanup_rooms_at(&state, Instant::now()).await;
    })
    .await;

    assert!(
        logged.contains("trimming room history") && logged.contains(MAIN_ROOM),
        "main's fade must be logged as a trim, naming the room: {logged}"
    );
    assert!(
        !logged.contains("deleted inactive room"),
        "main must never be logged as deleted, however idle (constraint #3): \
         {logged}"
    );

    let rooms = state.rooms.read().await;
    assert_eq!(
        rooms.get(MAIN_ROOM).unwrap().chat_history.len(),
        MAIN_ROOM_FADE_KEEP,
        "and must actually have faded to the keep size the log claims"
    );
}
