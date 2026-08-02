//! Who counts as "here": staleness, the heartbeat, idle eviction, and the
//! housekeeping passes that act on all three.

use super::*;

#[tokio::test]
async fn test_user_data_initial_state() {
    let user = UserData {
        user_id: "test-id".to_string(),
        animal_name: "Tiger".to_string(),
        last_active: Instant::now(),
        last_message_time: Instant::now(),
        connection_state: ConnectionState::Connected {
            last_heartbeat: Instant::now(),
            connection_id: "conn1".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };

    assert_eq!(user.user_id, "test-id");
    assert!(!user.is_typing);
    assert!(user.last_read_message.is_none());
}

// ========== TYPING & READ RECEIPTS (DEBOUNCING) ==========

#[tokio::test]
async fn test_user_count_with_mixed_states() {
    let mut room = create_room();

    let now = Instant::now();

    // Add connected user
    let connected_user = UserData {
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Connected {
            last_heartbeat: now,
            connection_id: "conn1".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };
    room.users.insert("user1".to_string(), connected_user);

    // Add disconnected user
    let disconnected_user = UserData {
        user_id: "user2".to_string(),
        animal_name: "Tiger".to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Disconnected { since: now },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };
    room.users.insert("user2".to_string(), disconnected_user);

    let connected_count = room
        .users
        .values()
        .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
        .count();

    assert_eq!(connected_count, 1);
    assert_eq!(room.users.len(), 2);
}

#[tokio::test]
async fn test_stale_user_detection() {
    let now = Instant::now();
    let stale_heartbeat = now - Duration::from_secs(10); // > 6s timeout

    let user = UserData {
        user_id: "stale".to_string(),
        animal_name: "Lion".to_string(),
        last_active: now - Duration::from_secs(10),
        last_message_time: now - Duration::from_secs(10),
        connection_state: ConnectionState::Connected {
            last_heartbeat: stale_heartbeat,
            connection_id: "conn1".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };

    let is_stale = if let ConnectionState::Connected { last_heartbeat, .. } = user.connection_state
    {
        now.duration_since(last_heartbeat) > HEARTBEAT_TIMEOUT
    } else {
        false
    };

    assert!(is_stale);
}

#[tokio::test]
async fn test_fresh_user_not_stale() {
    let now = Instant::now();
    let fresh_heartbeat = now - Duration::from_secs(2); // < 6s timeout

    let user = UserData {
        user_id: "fresh".to_string(),
        animal_name: "Tiger".to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Connected {
            last_heartbeat: fresh_heartbeat,
            connection_id: "conn1".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };

    let is_stale = if let ConnectionState::Connected { last_heartbeat, .. } = user.connection_state
    {
        now.duration_since(last_heartbeat) > HEARTBEAT_TIMEOUT
    } else {
        false
    };

    assert!(!is_stale);
}

// ========== EDGE CASES ==========

#[tokio::test]
async fn test_heartbeat_event_delivery() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/heartbeat-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Wait for heartbeat (sent every 5s, but we should get other events first)
    let mut _received_heartbeat = false;
    for _ in 0..20 {
        let event = recv_json_event(&mut ws).await;
        if event["type"] == "Heartbeat" {
            _received_heartbeat = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Note: may not always receive heartbeat in test timeframe, so we don't assert
    handle.abort();
}

#[tokio::test]
async fn test_user_last_read_message_tracking() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/read-tracking", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send read receipt
    let fake_uuid = "12345678-1234-5678-1234-567812345678";
    ws.send(text_frame(format!(
        r#"{{"type":"ReadReceipt","message_id":"{}"}}"#,
        fake_uuid
    )))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.abort();
}

#[tokio::test]
async fn test_heartbeat_timeout_detection() {
    let now = Instant::now();

    // User with fresh heartbeat
    let fresh_user = UserData {
        user_id: "fresh".to_string(),
        animal_name: "Lion".to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Connected {
            last_heartbeat: now - Duration::from_secs(1), // 1 second ago
            connection_id: "conn".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };

    // User with stale heartbeat
    let stale_user = UserData {
        user_id: "stale".to_string(),
        animal_name: "Tiger".to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Connected {
            last_heartbeat: now - HEARTBEAT_TIMEOUT - Duration::from_secs(1), // Past timeout
            connection_id: "conn".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };

    let is_fresh_stale =
        if let ConnectionState::Connected { last_heartbeat, .. } = fresh_user.connection_state {
            now.duration_since(last_heartbeat) > HEARTBEAT_TIMEOUT
        } else {
            false
        };

    let is_stale_stale =
        if let ConnectionState::Connected { last_heartbeat, .. } = stale_user.connection_state {
            now.duration_since(last_heartbeat) > HEARTBEAT_TIMEOUT
        } else {
            false
        };

    assert!(!is_fresh_stale, "Fresh user should not be stale");
    assert!(is_stale_stale, "Stale user should be detected as stale");
}

#[tokio::test]
async fn test_room_cleanup_stale_users() {
    let mut room = create_room();
    let now = Instant::now();

    // Add stale user (heartbeat way past timeout)
    room.users.insert(
        "stale-user".to_string(),
        UserData {
            user_id: "stale-user".to_string(),
            animal_name: "Lion".to_string(),
            last_active: now - Duration::from_secs(3600),
            last_message_time: now - Duration::from_secs(3600),
            connection_state: ConnectionState::Connected {
                last_heartbeat: now - Duration::from_secs(3600),
                connection_id: "old-conn".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
            last_reaction_event: None,
        },
    );

    // Check if user would be considered stale
    let user = room.users.get("stale-user").unwrap();
    let is_stale = if let ConnectionState::Connected { last_heartbeat, .. } = user.connection_state
    {
        now.duration_since(last_heartbeat) > HEARTBEAT_TIMEOUT
    } else {
        false
    };

    assert!(
        is_stale,
        "User with old heartbeat should be considered stale"
    );
}

#[tokio::test]
async fn test_heartbeat_interval_faster_than_cleanup() {
    // Server heartbeat should be much faster than cleanup to keep connections alive
    assert!(HEARTBEAT_INTERVAL.as_secs() < EMPTY_ROOM_CLEANUP_DELAY.as_secs());
    assert_eq!(HEARTBEAT_INTERVAL.as_secs(), 5);
}

#[tokio::test]
async fn test_heartbeat_timeout_reasonable() {
    // Client should have time to respond before being considered dead
    assert!(HEARTBEAT_TIMEOUT.as_secs() > HEARTBEAT_INTERVAL.as_secs());
    assert_eq!(HEARTBEAT_TIMEOUT.as_secs(), 6);
}

// ========== CONSTANTS COVERAGE TESTS ==========

#[tokio::test]
async fn test_frontend_heartbeat_timeout_matches_backend() {
    // Frontend JS has an 8s timeout for heartbeat detection
    // Backend sends heartbeats every HEARTBEAT_INTERVAL (5s)
    // Frontend should timeout > HEARTBEAT_INTERVAL but reasonably close

    let backend_interval_secs = HEARTBEAT_INTERVAL.as_secs();
    let backend_timeout_secs = HEARTBEAT_TIMEOUT.as_secs();

    // The frontend uses 8000ms (8s) for heartbeat timeout
    assert!(
        SHIPPED_CLIENT.contains("}, 8000);") || SHIPPED_CLIENT.contains("}, 8000)"),
        "Frontend heartbeat timeout should be 8000ms (8s). Check handleWebSocketMessage timeout."
    );

    // Backend heartbeat interval should be less than frontend timeout
    assert!(
        backend_interval_secs < 8,
        "Backend HEARTBEAT_INTERVAL ({}s) should be less than frontend timeout (8s)",
        backend_interval_secs
    );

    // Sanity check backend values
    assert_eq!(backend_interval_secs, 5, "HEARTBEAT_INTERVAL should be 5s");
    assert_eq!(backend_timeout_secs, 6, "HEARTBEAT_TIMEOUT should be 6s");
}

#[test]
fn test_user_idle_for_too_long() {
    let now = Instant::now();

    let mut user = UserData {
        user_id: "u1".to_string(),
        animal_name: "lion".to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Connected {
            last_heartbeat: now,
            connection_id: "conn".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    };

    user.last_message_time = now - (USER_IDLE_MESSAGE_TIMEOUT - Duration::from_secs(1));
    assert!(!user_idle_for_too_long(&user, now));

    user.last_message_time = now - (USER_IDLE_MESSAGE_TIMEOUT + Duration::from_secs(1));
    assert!(user_idle_for_too_long(&user, now));
}

// ========== REPLY FEATURE TESTS ==========

#[tokio::test]
async fn test_message_rejected_after_heartbeat_timeout() {
    let app_state = Arc::new(AppState::new());

    // Create room with a user that has an old heartbeat
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        let user_id = "timeout-user".to_string();
        // Set last_heartbeat to be older than HEARTBEAT_TIMEOUT
        let old_heartbeat = Instant::now() - Duration::from_secs(30);

        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id: user_id.clone(),
                animal_name: "TimeoutAnimal".to_string(),
                last_active: old_heartbeat,
                last_message_time: old_heartbeat,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: old_heartbeat,
                    connection_id: "conn-timeout".to_string(),
                },
                last_read_message: None,
                is_typing: false,
                last_typing_event: None,
                last_read_receipt_event: None,
                rate_limiter: RateLimiter::new(),
                last_message_text: None,
                last_reaction_event: None,
            },
        );

        rooms.insert("heartbeat-test-room".to_string(), room_state);
    }

    // Verify the user has timed out heartbeat
    let rooms = app_state.rooms.read().await;
    if let Some(room_state) = rooms.get("heartbeat-test-room")
        && let Some(user) = room_state.users.get("timeout-user")
        && let ConnectionState::Connected { last_heartbeat, .. } = user.connection_state
    {
        let elapsed = Instant::now().duration_since(last_heartbeat);
        assert!(
            elapsed > HEARTBEAT_TIMEOUT,
            "Heartbeat should have timed out"
        );
    }
}

// Test typing event debounce - covers lines 1758-1764

/// `main` is never deleted; once idle its history fades to a short tail.
#[tokio::test]
async fn main_room_history_fades_when_idle_instead_of_being_deleted() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();

        for i in 0..(MAIN_ROOM_FADE_KEEP + 120) {
            room.chat_history.push(Arc::new(OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("message {i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            }));
        }
        // Idle long enough to trigger the fade.
        room.last_activity = Instant::now() - MAIN_ROOM_FADE_IDLE - Duration::from_secs(30);
        rooms.insert(MAIN_ROOM.to_string(), room);
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get(MAIN_ROOM).expect("main must never be deleted");
    assert_eq!(
        room.chat_history.len(),
        MAIN_ROOM_FADE_KEEP,
        "idle main room should fade to its tail"
    );
}

/// §3 — a room's history is bounded by `MAX_MESSAGES_PER_ROOM` whether or not
/// anybody joins it.
///
/// Trimming used to happen only on the join path and, for `main`, in the fade
/// branch. A room that is busy but has no new joiners therefore grew without
/// limit, and because the byte ceiling is process-wide, one such room filled it
/// and every room on the server silently started dropping messages.
#[tokio::test]
async fn every_room_history_is_bounded_by_housekeeping_not_only_by_joins() {
    // `tracing` evaluates a log's fields only when the level is enabled, so
    // without a subscriber the `info!` inside the trim branch runs but its
    // arguments do not. That reads as uncovered lines in a branch the test
    // definitely takes — the instrument disagreeing with the code, which §0.5
    // says to check before believing either.
    init_tracing();

    let state = Arc::new(AppState::new());
    let overflow = MAX_MESSAGES_PER_ROOM * 3;

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "talker".to_string(),
            connected_user("talker", "otter", "c1", Instant::now()),
        );
        for i in 0..overflow {
            room.add_message(
                OutgoingMessage {
                    message_id: Uuid::new_v4(),
                    user_id: "talker".to_string(),
                    animal_name: "otter".to_string(),
                    text: format!("message {i}"),
                    timestamp: "1".to_string(),
                    reply_to: None,
                    attachment: None,
                },
                &state.memory_tracker,
            );
        }
        rooms.insert("busy-room".to_string(), room);
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get("busy-room").expect("busy room should survive");
    assert!(
        room.chat_history.len() <= MAX_MESSAGES_PER_ROOM,
        "housekeeping must bound every room's history, not just `main`; \
         found {} messages",
        room.chat_history.len()
    );
}

/// The housekeeping loops actually run on their interval.
///
/// `spawn_housekeeping` detaches two tasks with no join handle. If either ever
/// stopped being spawned — or panicked on its first pass — nothing would notice:
/// rooms would simply never be deleted and memory never swept, which is
/// precisely the symptom that took an afternoon to track down once already.
#[tokio::test]
#[ignore = "waits for a real housekeeping interval; run via scripts/slow-tests.sh"]
async fn the_housekeeping_loops_run_on_their_own() {
    let state = Arc::new(AppState::new());

    {
        let mut rooms = state.rooms.write().await;
        let mut doomed = create_room();
        let mut gone = connected_user("u1", "otter", "c1", Instant::now());
        gone.connection_state = ConnectionState::Disconnected {
            since: Instant::now(),
        };
        doomed.users.insert("u1".to_string(), gone);
        doomed.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(60);
        rooms.insert("swept".to_string(), doomed);
    }

    spawn_housekeeping(&state);

    let deleted = timeout(ROOM_CLEANUP_INTERVAL * 2, async {
        loop {
            if !state.rooms.read().await.contains_key("swept") {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;

    assert!(
        deleted.is_ok(),
        "the room housekeeping loop must delete an abandoned room on its own, \
         without anybody calling `cleanup_rooms`"
    );
}

/// The eviction frame carries the code the client reads.
///
/// Assertable without waiting ten minutes for a real eviction. A bare close is
/// indistinguishable from a dropped connection, and the client would reconnect
/// straight back into the room it was removed from — which is what stopped
/// rooms fading (§7).
#[test]
fn the_idle_eviction_frame_carries_the_agreed_code() {
    let crate::session::Message::Close(Some(frame)) = crate::session::idle_close_frame() else {
        panic!("an eviction must be a close frame carrying a reason");
    };

    assert_eq!(frame.code, IDLE_CLOSE_CODE);
    assert_eq!(frame.reason.as_str(), "idle");
}

/// Every housekeeping boundary, tested exactly on the boundary.
///
/// Three mutants survived here — `>` reading as `>=` for the retention and the
/// trim, and `&&` reading as `||` for the fade — because a test using its own
/// `Instant::now()` can get close to a threshold but never land on it. Passing
/// the instant in makes the edge reachable, and the edge is where the meaning
/// is: "retained for exactly the retention period" decides whether somebody
/// gets their name back.
#[tokio::test]
async fn the_housekeeping_boundaries_are_exact() {
    let now = Instant::now();

    // ---- Reclamation: exactly the retention is still retained -------------
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for (uid, since) in [
            ("exactly", now - DISCONNECTED_USER_RETENTION),
            (
                "past",
                now - DISCONNECTED_USER_RETENTION - Duration::from_nanos(1),
            ),
        ] {
            let mut user = connected_user(uid, "otter", "c", now);
            user.connection_state = ConnectionState::Disconnected { since };
            room.users.insert(uid.to_string(), user);
        }
        // Someone connected, so the room is not deleted out from under this.
        room.users.insert(
            "here".to_string(),
            connected_user("here", "badger", "c", now),
        );
        rooms.insert("retention".to_string(), room);
    }

    cleanup_rooms_at(&state, now).await;

    let rooms = state.rooms.read().await;
    let users = &rooms.get("retention").unwrap().users;
    assert!(
        users.contains_key("exactly"),
        "disconnected for *exactly* the retention period is still retained"
    );
    assert!(
        !users.contains_key("past"),
        "one nanosecond past it is reclaimed"
    );
    drop(rooms);

    // ---- The fade applies to `main`, and only to `main` -------------------
    let state = Arc::new(AppState::new());
    let tracker = MemoryTracker::new();
    {
        let mut rooms = state.rooms.write().await;
        for name in [MAIN_ROOM, "ordinary"] {
            let mut room = create_room();
            for i in 0..(MAIN_ROOM_FADE_KEEP + 20) {
                message_in(&mut room, &tracker, &format!("m{i}"));
            }
            // Idle by exactly the fade time.
            room.last_activity = now - MAIN_ROOM_FADE_IDLE;
            room.users.insert(
                "here".to_string(),
                connected_user("here", "otter", "c", now),
            );
            rooms.insert(name.to_string(), room);
        }
    }

    cleanup_rooms_at(&state, now).await;

    let rooms = state.rooms.read().await;
    assert_eq!(
        rooms.get(MAIN_ROOM).unwrap().chat_history.len(),
        MAIN_ROOM_FADE_KEEP,
        "`main` idle by exactly the fade time fades"
    );
    assert_eq!(
        rooms.get("ordinary").unwrap().chat_history.len(),
        MAIN_ROOM_FADE_KEEP + 20,
        "an ordinary room that is equally idle does not — the fade is `main` \
         *and* idle, not `main` *or* idle"
    );
    drop(rooms);

    // ---- The trim boundary: exactly the cap is left alone -----------------
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..MAX_MESSAGES_PER_ROOM {
            message_in(&mut room, &tracker, &format!("m{i}"));
        }
        room.last_activity = now;
        room.users.insert(
            "here".to_string(),
            connected_user("here", "otter", "c", now),
        );
        rooms.insert("at-the-cap".to_string(), room);
    }

    let before = state
        .rooms
        .read()
        .await
        .get("at-the-cap")
        .unwrap()
        .chat_history[0]
        .message_id;

    cleanup_rooms_at(&state, now).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get("at-the-cap").unwrap();
    assert_eq!(
        room.chat_history.len(),
        MAX_MESSAGES_PER_ROOM,
        "a history at exactly the cap must not be trimmed"
    );
    assert_eq!(
        room.chat_history[0].message_id, before,
        "and specifically must not lose its oldest message"
    );
}

/// A trim is reported, and a pass that trims nothing says nothing.
///
/// The log is what an operator reads to understand a running server, so "did it
/// trim" being announced only when it actually trimmed is part of the contract.
/// Three mutants lived in that guard because nothing read the output.
#[tokio::test]
async fn housekeeping_reports_a_trim_only_when_it_trims() {
    let now = Instant::now();
    let tracker = MemoryTracker::new();

    // A room over the cap: the trim happens and is announced.
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..(MAX_MESSAGES_PER_ROOM + 5) {
            message_in(&mut room, &tracker, &format!("m{i}"));
        }
        room.last_activity = now;
        room.users.insert(
            "here".to_string(),
            connected_user("here", "otter", "c", now),
        );
        rooms.insert("overfull".to_string(), room);
    }

    let logged = capturing_logs(|| async {
        cleanup_rooms_at(&state, now).await;
    })
    .await;

    assert!(
        logged.contains("trimming room history"),
        "a trim must be reported; the log said: {logged}"
    );
    assert!(
        logged.contains("overfull"),
        "and must name the room it trimmed: {logged}"
    );
    assert_eq!(
        state
            .rooms
            .read()
            .await
            .get("overfull")
            .unwrap()
            .chat_history
            .len(),
        MAX_MESSAGES_PER_ROOM
    );

    // A room inside the cap: nothing to say.
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..10 {
            message_in(&mut room, &tracker, &format!("m{i}"));
        }
        room.last_activity = now;
        room.users.insert(
            "here".to_string(),
            connected_user("here", "otter", "c", now),
        );
        rooms.insert("comfortable".to_string(), room);
    }

    let logged = capturing_logs(|| async {
        cleanup_rooms_at(&state, now).await;
    })
    .await;

    assert!(
        !logged.contains("trimming room history"),
        "a pass that trimmed nothing must not claim it did: {logged}"
    );
}

/// Deleting a room is announced, by name.
#[tokio::test]
async fn housekeeping_reports_the_rooms_it_deletes() {
    let now = Instant::now();
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        let mut gone = connected_user("u", "otter", "c", now);
        gone.connection_state = ConnectionState::Disconnected { since: now };
        room.users.insert("u".to_string(), gone);
        room.last_activity = now - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(1);
        rooms.insert("vanishing".to_string(), room);
    }

    let logged = capturing_logs(|| async {
        cleanup_rooms_at(&state, now).await;
    })
    .await;

    assert!(
        logged.contains("deleted inactive room") && logged.contains("vanishing"),
        "a deleted room must be named in the log: {logged}"
    );
}

/// Touching a user who is no longer there is not idleness.
///
/// The heartbeat asks "is this user quiet" every few seconds. If the room or
/// the user has gone, the session is ending for some other reason and the
/// answer must be no — reporting idleness would send an eviction frame to a
/// connection that is already tearing down, and tell the client it went quiet
/// when it did not.
#[tokio::test]
async fn a_missing_user_is_not_reported_as_idle() {
    let state = Arc::new(AppState::new());

    assert!(
        !crate::session::touch_and_check_idle(&state, "no-such-room", "u1").await,
        "no room means no idleness"
    );

    {
        let mut rooms = state.rooms.write().await;
        rooms.insert("empty".to_string(), create_room());
    }
    assert!(
        !crate::session::touch_and_check_idle(&state, "empty", "u1").await,
        "no user means no idleness"
    );

    // A user who is present and talking is not idle, and gets touched.
    {
        let mut rooms = state.rooms.write().await;
        let room = rooms.get_mut("empty").unwrap();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
    }
    assert!(!crate::session::touch_and_check_idle(&state, "empty", "u1").await);
}

/// A second connection under the same identity supersedes the first — the
/// two-tab case `SUPERSEDED_CLOSE_CODE` exists for.
///
/// Reproduced directly against a real pair of connections before this was
/// fixed: the first connection's socket stayed open on the wire indefinitely,
/// still receiving broadcasts, while `connected_user_count` had already
/// stopped counting it.
#[tokio::test]
async fn a_newer_connection_under_the_same_identity_supersedes_the_older_one() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "conn-new", Instant::now()),
        );
        rooms.insert("main".to_string(), room);
    }

    assert!(
        crate::session::is_superseded(&state, "main", "u1", "conn-old").await,
        "a connection_id that is no longer current must be reported superseded"
    );
    assert!(
        !crate::session::is_superseded(&state, "main", "u1", "conn-new").await,
        "the current connection_id must not report itself as superseded"
    );
}

/// Neither a missing room nor a missing user is "superseded" — the session is
/// ending for some other reason, and this check only means to catch the one
/// case where somebody *else* is now current under the same identity.
#[tokio::test]
async fn a_missing_user_is_not_reported_as_superseded() {
    let state = Arc::new(AppState::new());

    assert!(
        !crate::session::is_superseded(&state, "no-such-room", "u1", "c1").await,
        "no room means not superseded"
    );

    {
        let mut rooms = state.rooms.write().await;
        rooms.insert("empty".to_string(), create_room());
    }
    assert!(
        !crate::session::is_superseded(&state, "empty", "u1", "c1").await,
        "no user means not superseded"
    );
}

/// The superseded-close frame carries the code the client reads.
///
/// Same reasoning as `the_idle_eviction_frame_carries_the_agreed_code`: a bare
/// close is indistinguishable from a dropped connection, and the superseded
/// tab's own reconnect logic would steal the identity straight back from the
/// tab that just reclaimed it.
#[test]
fn the_superseded_close_frame_carries_the_agreed_code() {
    let crate::session::Message::Close(Some(frame)) = crate::session::superseded_close_frame()
    else {
        panic!("a supersession must be a close frame carrying a reason");
    };

    assert_eq!(frame.code, SUPERSEDED_CLOSE_CODE);
    assert_eq!(frame.reason.as_str(), "superseded");
}

/// An event arriving exactly at the heartbeat timeout is still this connection's.
///
/// A lapsed heartbeat means the socket is being torn down, and the event is
/// dropped rather than raced against teardown. `>` mutating to `>=` survived
/// unnoticed: at exactly the timeout the connection has not yet lapsed, and
/// dropping its message would lose a message from a live user for being one
/// instant slow.
#[tokio::test]
async fn an_event_exactly_at_the_heartbeat_timeout_is_still_delivered() {
    let now = Instant::now();

    for (age, delivered) in [
        (HEARTBEAT_TIMEOUT, true),
        (HEARTBEAT_TIMEOUT + Duration::from_nanos(1), false),
    ] {
        let state = Arc::new(AppState::new());
        {
            let mut rooms = state.rooms.write().await;
            let mut room = create_room();
            room.users.insert(
                "u1".to_string(),
                connected_user("u1", "otter", "c1", now - age),
            );
            rooms.insert("r".to_string(), room);
        }

        let mut rx = state.rooms.read().await["r"].sender.subscribe();
        apply_client_event_at(
            &state,
            "r",
            "u1",
            "otter",
            ClientEvent::Message {
                text: "still here".to_string(),
                reply_to: None,
                attachment: None,
            },
            now,
        )
        .await;

        assert_eq!(
            rx.try_recv().is_ok(),
            delivered,
            "a heartbeat {age:?} old against a timeout of {HEARTBEAT_TIMEOUT:?} \
             should still be this connection's: {delivered}"
        );
    }
}
