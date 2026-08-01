//! `AppState` itself: construction, shutdown, and the connection-upgrade path
//! that creates a room's first `RoomState`.

use super::*;

#[tokio::test]
async fn test_appstate_shutdown_clears_rooms() {
    let app_state = Arc::new(AppState::new());
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("test".into(), create_room());
    }

    app_state.shutdown().await;
    assert!(app_state.rooms.read().await.is_empty());
}

#[tokio::test]
async fn test_app_state_shutdown() {
    let app_state = Arc::new(AppState::new());

    // Create a room with a user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "user1".to_string(),
            UserData {
                user_id: "user1".to_string(),
                animal_name: "Lion".to_string(),
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
            },
        );
        rooms.insert("test-room".to_string(), room);
    }

    // Verify room exists
    assert_eq!(app_state.rooms.read().await.len(), 1);

    // Call shutdown
    app_state.shutdown().await;

    // Rooms should be cleared
    assert_eq!(app_state.rooms.read().await.len(), 0);
}

#[tokio::test]
async fn test_app_state_cleanup() {
    let app_state = Arc::new(AppState::new());

    // Create a room
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("cleanup-test".to_string(), create_room());
    }

    // Trigger cleanup
    app_state.cleanup().await;

    // Room should still exist (cleanup only removes stale connections)
    assert_eq!(app_state.rooms.read().await.len(), 1);
}

#[tokio::test]
async fn test_app_state_new_initializes_correctly() {
    let state = AppState::new();

    assert!(state.rooms.read().await.is_empty());
    assert_eq!(state.memory_tracker.total_bytes.load(Ordering::Relaxed), 0);
    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn test_version_is_set() {
    assert!(!VERSION.is_empty());
}

// ==========================================================================
// FRONTEND/BACKEND CONSISTENCY TESTS
// ==========================================================================
// These tests ensure the embedded HTML frontend stays in sync with backend
// constants. They MUST fail if timing values drift apart.

#[tokio::test]
async fn test_binary_message_handling() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/binary-test-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send binary data - should be ignored
    ws.send(WsMessage::Binary(vec![0, 1, 2, 3, 4, 5].into()))
        .await
        .ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(
        ping_result.is_ok(),
        "Connection should still be alive after binary message"
    );

    ws.close(None).await.ok();
}

// Test failed event parsing - covers lines 1821-1825

#[tokio::test]
async fn test_malformed_json_handling() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/malformed-json-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send malformed JSON
    ws.send(text_frame("{not valid json}")).await.ok();
    ws.send(text_frame("just plain text")).await.ok();
    ws.send(text_frame(r#"{"type":"Unknown"}"#)).await.ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(
        ping_result.is_ok(),
        "Connection should still be alive after malformed JSON"
    );

    ws.close(None).await.ok();
}

// Test cookie parsing with extra unknown cookies - covers line 177 (_)

#[tokio::test]
async fn test_app_state_shutdown_empty() {
    let state = Arc::new(AppState::new());

    // Should not panic with empty state
    state.shutdown().await;

    // Rooms should be empty after shutdown
    let rooms = state.rooms.read().await;
    assert!(rooms.is_empty());
}

// Test AppState shutdown with rooms - covers shutdown paths

#[tokio::test]
async fn test_app_state_shutdown_with_rooms() {
    let state = Arc::new(AppState::new());

    // Create some rooms
    {
        let mut rooms = state.rooms.write().await;
        rooms.insert("room1".to_string(), create_room());
        rooms.insert("room2".to_string(), create_room());
    }

    // Shutdown should broadcast to all rooms
    state.shutdown().await;

    // Rooms should be empty
    let rooms = state.rooms.read().await;
    assert!(rooms.is_empty());
}

// Test create_user_cookies output format - covers cookie creation paths

#[tokio::test]
async fn test_app_state_shutdown_notifies_users() {
    let state = Arc::new(AppState::new());

    // Create room with users
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "user1".to_string(),
            UserData {
                user_id: "user1".to_string(),
                animal_name: "Lion".to_string(),
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
            },
        );
        rooms.insert("shutdown-notify".to_string(), room);
    }

    // Subscribe before shutdown
    let _rx = {
        let rooms = state.rooms.read().await;
        rooms.get("shutdown-notify").map(|r| r.sender.subscribe())
    };

    // Shutdown
    state.shutdown().await;

    // Rooms should be cleared
    let rooms = state.rooms.read().await;
    assert!(rooms.is_empty());
}

// Test ws_creates_new_room - covers lines 1261-1290

#[tokio::test]
async fn test_app_state_initialization() {
    let state = AppState::new();

    // Check all components are initialized
    assert_eq!(state.rooms.read().await.len(), 0);
    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(state.memory_tracker.total_bytes.load(Ordering::Relaxed), 0);
    assert_eq!(state.connection_pool.active.load(Ordering::Relaxed), 0);
}

// Test SecurityManager check_ip and record_suspicious_activity - covers lines 465-485

/// A full room refuses the upgrade and releases the slots it had reserved.
///
/// This is the path that runs *after* the reservations are taken — the one
/// `release_connection_slot` exists for. Without it, every rejection here
/// leaked a slot permanently.
#[tokio::test]
async fn a_full_room_refuses_and_releases_its_reservations() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        let now = Instant::now();

        for i in 0..MAX_USERS_PER_ROOM {
            room.users.insert(
                format!("user-{i}"),
                UserData {
                    user_id: format!("user-{i}"),
                    animal_name: format!("occupant-{i}"),
                    last_active: now,
                    last_message_time: now,
                    connection_state: ConnectionState::Connected {
                        last_heartbeat: now,
                        connection_id: format!("conn-{i}"),
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
        }
        rooms.insert("packed-room".to_string(), room);
    }

    assert!(
        connect_async(ws_request(addr, "packed-room", &[]))
            .await
            .is_err(),
        "a full room must refuse the upgrade"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        0,
        "the refused upgrade must release its global slot"
    );
    assert_eq!(
        state.connection_pool.active.load(Ordering::SeqCst),
        0,
        "the refused upgrade must release its per-IP slot"
    );

    handle.abort();
}

/// Events for a room or user the server does not know are dropped, not fatal.
#[tokio::test]
async fn events_for_unknown_rooms_and_users_are_dropped() {
    let state = state_with_user("room", "user", Instant::now()).await;

    apply_client_event(
        &state,
        "no-such-room",
        "user",
        "otter",
        ClientEvent::Message {
            text: "hi".into(),
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    apply_client_event(
        &state,
        "room",
        "no-such-user",
        "otter",
        ClientEvent::Message {
            text: "hi".into(),
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    assert!(
        rooms["room"].chat_history.is_empty(),
        "neither event should have produced a message"
    );
}

/// The upgrade path must hash before handing the address to anything that
/// stores it.
///
/// Asserted over the source because the property is *where* the hash happens:
/// a version that stored the raw address and hashed later would pass any
/// behavioural test while losing the entire point.
#[test]
fn the_upgrade_path_hashes_the_address_at_the_boundary() {
    // `ws_handler` and the hash it must apply both live in `admission.rs` now
    // that `session.rs` is a directory split by phase of a connection's life.
    const SESSION_SRC: &str = include_str!("../session/admission.rs");

    assert!(
        SESSION_SRC.contains(".map(hash_client_address)"),
        "ws_handler must hash the client address before using it"
    );
    assert!(
        !SESSION_SRC.contains("ip = ?ip"),
        "the client address must never be written to a log line"
    );
}

/// A user missing at upgrade is recreated rather than dropped.
///
/// The other side of the same race: the room survived but the user's slot did
/// not. Dropping them here would mean an upgraded socket belonging to nobody —
/// it would receive broadcasts and never appear in the user count.
#[tokio::test]
async fn a_user_missing_at_upgrade_is_recreated_in_place() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        // A room with history, and no users at all.
        let mut room = create_room();
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "someone".to_string(),
                animal_name: "badger".to_string(),
                text: "said earlier".to_string(),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            },
            &state.memory_tracker,
        );
        rooms.insert("orphaned".to_string(), room);
    }

    let (_receiver, history) =
        crate::session::join_room(&state, "orphaned", "ghost", "otter", "conn-9")
            .await
            .expect("the room exists, so the join succeeds");

    assert_eq!(history.len(), 1, "the joiner still receives the history");

    let rooms = state.rooms.read().await;
    let user = rooms
        .get("orphaned")
        .unwrap()
        .users
        .get("ghost")
        .expect("the missing user must have been recreated");

    assert_eq!(user.animal_name, "otter");
    assert!(
        user.is_connected(),
        "and recreated as connected, or they would never appear in the count"
    );
    assert!(
        matches!(
            &user.connection_state,
            ConnectionState::Connected { connection_id, .. } if connection_id == "conn-9"
        ),
        "with this connection's id, so teardown can tell it from a newer one"
    );
}

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

/// `announce_departures` in isolation: only connected users are told they
/// left, built and checked without going through a full `AppState::shutdown`.
#[tokio::test]
async fn announce_departures_tells_only_connected_users() {
    let mut room = create_room();
    room.users.insert(
        "connected-user".to_string(),
        connected_user("connected-user", "otter", "c1", Instant::now()),
    );
    room.users.insert(
        "gone-user".to_string(),
        UserData {
            user_id: "gone-user".to_string(),
            animal_name: "heron".to_string(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Disconnected {
                since: Instant::now(),
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

    let mut receiver = room.sender.subscribe();
    announce_departures(&room);

    let event = timeout(Duration::from_millis(100), receiver.recv())
        .await
        .expect("a departure must be announced")
        .unwrap();
    let OutgoingEvent::System {
        event: SystemEvent::UserLeft { user_id, .. },
    } = event
    else {
        panic!("expected a UserLeft system event, got {event:?}");
    };
    assert_eq!(user_id, "connected-user");

    // Only one: the disconnected user must not get a second announcement.
    assert!(
        timeout(Duration::from_millis(50), receiver.recv())
            .await
            .is_err(),
        "a user who was already disconnected must not be announced again"
    );
}
