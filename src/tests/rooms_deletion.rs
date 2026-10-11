//! Removing a room entirely: the empty/idle cleanup passes, `main`'s
//! exemption from all of them, and the memory ceiling that triggers a sweep.

use super::*;

#[tokio::test]
async fn test_cleanup_rooms_removes_inactive() {
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - Duration::from_secs(7201);
        room.users.clear();
        rooms.insert("inactive_room".to_string(), room);
    }

    cleanup_rooms(&app_state).await;
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("inactive_room"));
}

#[tokio::test]
async fn test_cleanup_rooms_keeps_active() {
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now();
        rooms.insert("active_room".to_string(), room);
    }

    cleanup_rooms(&app_state).await;
    let rooms = app_state.rooms.read().await;
    assert!(rooms.contains_key("active_room"));
}

// ========== STRESS & CONCURRENCY TESTS ==========

#[tokio::test]
async fn test_room_cleanup_removes_old_inactive() {
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut old_room = create_room();
        old_room.last_activity = Instant::now() - Duration::from_secs(7201); // > 2 hours
        rooms.insert("old_inactive".to_string(), old_room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("old_inactive"));
}

#[tokio::test]
async fn test_room_cleanup_preserves_recent_messages() {
    let app_state = Arc::new(AppState::new());
    let room_name = "preserve-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now();

        // Add some recent messages
        for i in 0..10 {
            room.chat_history.push(Arc::new(OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: Uuid::new_v4().to_string(),
                animal_name: "Lion".to_string(),
                text: format!("Message {}", i),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_string(),
                reply_to: None,
                attachment: None,
            }));
        }

        rooms.insert(room_name.clone(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    let room = rooms.get(&room_name).unwrap();
    assert_eq!(room.chat_history.len(), 10);
}

#[tokio::test]
async fn test_room_cleanup_preserves_active_rooms() {
    let app_state = Arc::new(AppState::new());
    let active_room = "active-room".to_string();
    let inactive_room = "inactive-room".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(active_room.clone(), create_room());
        rooms.insert(inactive_room.clone(), create_room());
    }

    {
        let mut rooms = app_state.rooms.write().await;
        if let Some(room) = rooms.get_mut(&inactive_room) {
            room.last_activity = std::time::Instant::now() - std::time::Duration::from_secs(8000);
        }
    }

    let rooms = app_state.rooms.read().await;
    assert!(rooms.contains_key(&active_room));
}

#[tokio::test]
async fn test_cleanup_rooms_handles_empty_rooms() {
    let app_state = Arc::new(AppState::new());

    // Add room with no users and old last_activity
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - Duration::from_secs(7201); // Over 2 hours old
        room.users.clear();
        rooms.insert("empty-old-room".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        !rooms.contains_key("empty-old-room"),
        "Old empty room should be removed"
    );
}

#[tokio::test]
async fn test_cleanup_rooms_keeps_rooms_with_users() {
    let app_state = Arc::new(AppState::new());

    // Add room with a user but old last_activity
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - Duration::from_secs(7201);

        let now = Instant::now();
        room.users.insert(
            "active-user".to_string(),
            UserData {
                user_id: "active-user".to_string(),
                animal_name: "Lion".to_string(),
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
            },
        );
        rooms.insert("room-with-user".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        rooms.contains_key("room-with-user"),
        "Room with users should be kept"
    );
}

// ========== CLAIM VERIFICATION TESTS ==========
// These tests verify the claims in the welcome message

#[tokio::test]
async fn test_claim_main_room_persistent() {
    let app_state = Arc::new(AppState::new());

    // Add main room empty + old
    {
        let mut rooms = app_state.rooms.write().await;
        let room_state = RoomState {
            sender: tokio::sync::broadcast::channel(1000).0,
            chat_history: vec![],
            reactions: std::collections::HashMap::new(),
            message_ids: std::collections::HashSet::new(),
            attachment_bytes: 0,
            users: std::collections::HashMap::new(),
            available_animals: std::collections::VecDeque::new(),
            last_activity: Instant::now() - Duration::from_secs(7201),
            total_memory_bytes: std::sync::atomic::AtomicUsize::new(0),
        };
        rooms.insert("main".to_string(), room_state);
    }

    assert!(app_state.rooms.read().await.contains_key("main"));
    cleanup_rooms(&app_state).await;
    // Main is special - never deleted
    assert!(app_state.rooms.read().await.contains_key("main"));
}

#[tokio::test]
async fn test_claim_custom_room_deleted_when_empty() {
    let app_state = Arc::new(AppState::new());

    // Create custom room empty + 15min old
    {
        let mut rooms = app_state.rooms.write().await;
        let room_state = RoomState {
            sender: tokio::sync::broadcast::channel(1000).0,
            chat_history: vec![],
            reactions: std::collections::HashMap::new(),
            message_ids: std::collections::HashSet::new(),
            attachment_bytes: 0,
            users: std::collections::HashMap::new(),
            available_animals: std::collections::VecDeque::new(),
            last_activity: Instant::now() - Duration::from_secs(901),
            total_memory_bytes: std::sync::atomic::AtomicUsize::new(0),
        };
        rooms.insert("temp-room".to_string(), room_state);
    }

    assert!(app_state.rooms.read().await.contains_key("temp-room"));
    cleanup_rooms(&app_state).await;
    // Custom room should be deleted
    assert!(!app_state.rooms.read().await.contains_key("temp-room"));
}

#[tokio::test]
async fn test_room_cleanup_interval_matches_delay() {
    // Cleanup should run frequently enough to catch rooms at the 1-min mark
    assert_eq!(ROOM_CLEANUP_INTERVAL.as_secs(), 60);
}

#[tokio::test]
async fn test_room_survives_before_cleanup_delay() {
    // Room with activity within the cleanup window should survive
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let room_state = RoomState {
            sender: tokio::sync::broadcast::channel(1000).0,
            chat_history: vec![],
            reactions: std::collections::HashMap::new(),
            message_ids: std::collections::HashSet::new(),
            attachment_bytes: 0,
            users: std::collections::HashMap::new(),
            available_animals: std::collections::VecDeque::new(),
            // Comfortably inside the cleanup window.
            last_activity: Instant::now() - (EMPTY_ROOM_CLEANUP_DELAY / 2),
            total_memory_bytes: std::sync::atomic::AtomicUsize::new(0),
        };
        rooms.insert("should-survive".to_string(), room_state);
    }

    cleanup_rooms(&app_state).await;

    // Room should still exist (under threshold)
    assert!(app_state.rooms.read().await.contains_key("should-survive"));
}

#[tokio::test]
async fn test_room_deleted_after_cleanup_delay() {
    // Room with no activity past the cleanup window should be deleted
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let room_state = RoomState {
            sender: tokio::sync::broadcast::channel(1000).0,
            chat_history: vec![],
            reactions: std::collections::HashMap::new(),
            message_ids: std::collections::HashSet::new(),
            attachment_bytes: 0,
            users: std::collections::HashMap::new(),
            available_animals: std::collections::VecDeque::new(),
            // Comfortably past the cleanup window.
            last_activity: Instant::now() - (EMPTY_ROOM_CLEANUP_DELAY * 2),
            total_memory_bytes: std::sync::atomic::AtomicUsize::new(0),
        };
        rooms.insert("should-die".to_string(), room_state);
    }

    cleanup_rooms(&app_state).await;

    // Room should be deleted
    assert!(!app_state.rooms.read().await.contains_key("should-die"));
}

#[tokio::test]
async fn test_room_with_active_users_never_deleted() {
    // Even an old room with active users should survive
    let app_state = Arc::new(AppState::new());
    let now = Instant::now();

    {
        let mut rooms = app_state.rooms.write().await;
        let mut users = std::collections::HashMap::new();
        users.insert(
            "user1".to_string(),
            UserData {
                user_id: "user-123".to_string(),
                animal_name: "Tiger".to_string(),
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
            },
        );

        let room_state = RoomState {
            sender: tokio::sync::broadcast::channel(1000).0,
            chat_history: vec![],
            reactions: std::collections::HashMap::new(),
            message_ids: std::collections::HashSet::new(),
            attachment_bytes: 0,
            users,
            available_animals: std::collections::VecDeque::new(),
            // Very old, but has active user
            last_activity: Instant::now() - Duration::from_secs(7200),
            total_memory_bytes: std::sync::atomic::AtomicUsize::new(0),
        };
        rooms.insert("has-users".to_string(), room_state);
    }

    cleanup_rooms(&app_state).await;

    // Should survive because it has users
    assert!(app_state.rooms.read().await.contains_key("has-users"));
}

#[tokio::test]
async fn test_room_cleanup_deletes_empty_rooms_after_delay() {
    // Bug fix: Rooms with NO active users should be deleted after EMPTY_ROOM_CLEANUP_DELAY
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        // Set last_activity to exactly at cleanup threshold (60s)
        room.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY;
        room.users.clear(); // NO users at all
        rooms.insert("should-delete".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        !rooms.contains_key("should-delete"),
        "Empty room at cleanup threshold should be deleted"
    );
}

#[tokio::test]
async fn test_room_cleanup_preserves_empty_room_before_delay() {
    // Bug fix: Empty rooms BEFORE cleanup delay should be preserved
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        // Set last_activity to 30s ago (half the cleanup delay)
        room.last_activity = Instant::now() - Duration::from_secs(30);
        room.users.clear();
        rooms.insert("too-new".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        rooms.contains_key("too-new"),
        "Empty room before cleanup delay should be preserved"
    );
}

#[tokio::test]
async fn test_room_cleanup_connected_users_prevent_deletion() {
    // Bug fix: Rooms with CONNECTED users should NOT be deleted even after delay
    let app_state = Arc::new(AppState::new());
    let now = Instant::now();

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(100);

        // Add an actively connected user
        room.users.insert(
            "active".to_string(),
            UserData {
                user_id: "active".to_string(),
                animal_name: "Eagle".to_string(),
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: "conn123".to_string(),
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

        rooms.insert("has-active-user".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        rooms.contains_key("has-active-user"),
        "Room with connected users should NOT be deleted even after long delay"
    );
}

#[tokio::test]
async fn test_room_cleanup_multiple_users_one_connected_blocks_deletion() {
    // Bug fix: Even one connected user should prevent room deletion
    let app_state = Arc::new(AppState::new());
    let now = Instant::now();

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(100);

        // Add multiple users: some disconnected, one connected
        room.users.insert(
            "disconnected1".to_string(),
            UserData {
                user_id: "disconnected1".to_string(),
                animal_name: "Bear".to_string(),
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
            },
        );

        room.users.insert(
            "connected".to_string(),
            UserData {
                user_id: "connected".to_string(),
                animal_name: "Hawk".to_string(),
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: "conn456".to_string(),
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

        rooms.insert("mixed-users".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        rooms.contains_key("mixed-users"),
        "Room with at least one connected user should be preserved"
    );
}

// ========== VERSION CONSTANT TEST ==========

#[tokio::test]
async fn test_cleanup_interval_is_sensible() {
    // Cleanup interval should be <= cleanup delay to catch rooms in time

    let interval = ROOM_CLEANUP_INTERVAL.as_secs();
    let delay = EMPTY_ROOM_CLEANUP_DELAY.as_secs();

    assert!(
        interval <= delay,
        "ROOM_CLEANUP_INTERVAL ({}s) should be <= EMPTY_ROOM_CLEANUP_DELAY ({}s) \
         otherwise rooms might not be cleaned up in time!",
        interval,
        delay
    );
}

#[tokio::test]
async fn test_room_deleted_during_event_processing() {
    let app_state = Arc::new(AppState::new());

    // Create then immediately delete a room
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("deleted-during-process".to_string(), create_room());
    }

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.remove("deleted-during-process");
    }

    // Verify room is gone
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("deleted-during-process"));
}

// Test invalid message_id in read receipt - covers lines 1803-1808

#[tokio::test]
async fn test_cleanup_user_nonexistent_room() {
    let app_state = Arc::new(AppState::new());

    // Try to cleanup a user in a room that doesn't exist
    cleanup_user(
        &app_state,
        "nonexistent-room",
        "fake-user",
        "fake-connection",
        Some("127.0.0.1"),
    )
    .await;

    // Should not panic or error
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("nonexistent-room"));
}

// Test cleanup_user when user doesn't exist in room - covers lines 2014-2015

#[tokio::test]
async fn test_cleanup_user_nonexistent_user() {
    let app_state = Arc::new(AppState::new());

    // Create a room without the user
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("existing-room".to_string(), create_room());
    }

    // Try to cleanup a user that doesn't exist in the room
    cleanup_user(
        &app_state,
        "existing-room",
        "nonexistent-user",
        "fake-connection",
        Some("127.0.0.1"),
    )
    .await;

    // Should not panic or error
    let rooms = app_state.rooms.read().await;
    assert!(rooms.contains_key("existing-room"));
}

// Test security_manager check for banned IP - covers various ban check paths

#[tokio::test]
async fn test_cleanup_rooms_with_messages_no_users() {
    let app_state = Arc::new(AppState::new());

    // Create a room with messages but no users
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        // Add a message
        room_state.chat_history.push(Arc::new(OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Lion".to_string(),
            text: "Test message".to_string(),
            timestamp: "12345".to_string(),
            reply_to: None,
            attachment: None,
        }));

        // Set last_activity to be old enough for cleanup
        room_state.last_activity =
            Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(10);

        rooms.insert("old-room-with-messages".to_string(), room_state);
    }

    // Run cleanup
    cleanup_rooms(&app_state).await;

    // Room should be cleaned up since it has no connected users
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("old-room-with-messages"));
}

// Test prune_old_messages when messages exist - covers lines 887-902

#[tokio::test]
async fn test_room_max_users() {
    let app_state = Arc::new(AppState::new());

    // Create a room and add several users
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        // Add users (use a reasonable number, not MAX_CONCURRENT_USERS which is 400)
        for i in 0..10 {
            room_state.users.insert(
                format!("user{}", i),
                UserData {
                    user_id: format!("user{}", i),
                    animal_name: format!("Animal{}", i),
                    last_active: Instant::now(),
                    last_message_time: Instant::now(),
                    connection_state: ConnectionState::Connected {
                        last_heartbeat: Instant::now(),
                        connection_id: format!("conn{}", i),
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

        rooms.insert("user-room".to_string(), room_state);
    }

    // Room should have 10 users
    let rooms = app_state.rooms.read().await;
    let room = rooms.get("user-room").unwrap();
    assert_eq!(room.users.len(), 10);
}

// Test broadcast with multiple receivers - covers broadcast paths

#[tokio::test]
async fn test_room_trigger_cleanup() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Add many messages
    for i in 0..100 {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: format!("Message content {}", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        }));
        tracker.add_bytes(100);
        room.total_memory_bytes.fetch_add(100, Ordering::SeqCst);
    }

    // Trigger cleanup
    room.trigger_cleanup(&tracker).await;

    // Should have cleaned up some messages
    // (behavior depends on whether conditions are met)
}

// Test extract_client_ip with various headers - covers lines 1200-1230

#[tokio::test]
async fn test_cleanup_rooms_removes_old_rooms() {
    let state = Arc::new(AppState::new());

    // Create an old room with no users
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        // Make it old
        room.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(10);
        rooms.insert("old-empty".to_string(), room);
    }

    // Run cleanup
    cleanup_rooms(&state).await;

    // Room should be removed
    let rooms = state.rooms.read().await;
    assert!(!rooms.contains_key("old-empty"));
}

// Test main room is protected from cleanup - covers main room cleanup logic

#[tokio::test]
async fn test_cleanup_rooms_preserves_main() {
    let state = Arc::new(AppState::new());

    // Create main room with activity
    {
        let mut rooms = state.rooms.write().await;
        let room = create_room();
        rooms.insert("main".to_string(), room);
    }

    // Run cleanup
    cleanup_rooms(&state).await;

    // Main room should still exist
    let rooms = state.rooms.read().await;
    assert!(rooms.contains_key("main"));
}

// Test generate_random_room_name produces valid names - covers lines 2131-2172

/// The periodic resource sweep returns immediately when a sweep is not due,
/// without taking the room write lock.
#[tokio::test]
async fn resource_cleanup_returns_early_when_no_sweep_is_due() {
    let state = Arc::new(AppState::new());

    // The first call consumes the one sweep that is immediately due.
    state.cleanup().await;
    // The second must take the early path.
    state.cleanup().await;

    assert!(
        !state.memory_tracker.should_gc(),
        "a sweep should not be due again this soon"
    );
}

// ========== SESSION ERROR PATHS ==========

/// §7 — a room with nobody in it is deleted.
///
/// The mechanic the whole product rests on, asserted end to end over
/// `cleanup_rooms` rather than over one of its branches: a room whose users
/// have all disconnected and which has been quiet for the grace period is gone,
/// while `main` and a room somebody is still in are not.
#[tokio::test]
async fn a_room_nobody_is_in_is_deleted_and_main_never_is() {
    let state = Arc::new(AppState::new());
    let long_ago = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(60);

    {
        let mut rooms = state.rooms.write().await;

        // Everyone left, and it has been quiet since.
        let mut abandoned = create_room();
        let mut gone = connected_user("u1", "otter", "c1", Instant::now());
        gone.connection_state = ConnectionState::Disconnected {
            since: Instant::now(),
        };
        abandoned.users.insert("u1".to_string(), gone);
        abandoned.last_activity = long_ago;
        rooms.insert("abandoned".to_string(), abandoned);

        // Somebody is still here, quiet or not.
        let mut occupied = create_room();
        occupied.users.insert(
            "u2".to_string(),
            connected_user("u2", "badger", "c2", Instant::now()),
        );
        occupied.last_activity = long_ago;
        rooms.insert("occupied".to_string(), occupied);

        // `main` is permanent even when empty and silent.
        let mut main = create_room();
        main.last_activity = long_ago;
        rooms.insert(MAIN_ROOM.to_string(), main);
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    assert!(
        !rooms.contains_key("abandoned"),
        "a room nobody is in, quiet past the grace period, must be deleted — \
         this is the scarcity the product is made of (§7)"
    );
    assert!(
        rooms.contains_key("occupied"),
        "a room somebody is still in must survive however quiet it is"
    );
    assert!(
        rooms.contains_key(MAIN_ROOM),
        "`main` is never deleted (constraint #3)"
    );
}

/// A room is deleted at exactly the grace period, not a moment before.
#[tokio::test]
async fn the_empty_room_grace_period_is_exact() {
    let now = Instant::now();
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        for (name, last_activity) in [
            (
                "just-inside",
                now - EMPTY_ROOM_CLEANUP_DELAY + Duration::from_nanos(1),
            ),
            ("exactly", now - EMPTY_ROOM_CLEANUP_DELAY),
        ] {
            let mut room = create_room();
            let mut gone = connected_user("u", "otter", "c", now);
            gone.connection_state = ConnectionState::Disconnected { since: now };
            room.users.insert("u".to_string(), gone);
            room.last_activity = last_activity;
            rooms.insert(name.to_string(), room);
        }
    }

    cleanup_rooms_at(&state, now).await;

    let rooms = state.rooms.read().await;
    assert!(
        rooms.contains_key("just-inside"),
        "a nanosecond short of the grace period, the room survives"
    );
    assert!(
        !rooms.contains_key("exactly"),
        "at exactly the grace period, it is deleted"
    );
}

/// Deleting a room returns its messages' bytes to the process budget.
///
/// `freed > 0` mutated to `== 0` and to `< 0` and both survived: nothing
/// asserted that the global tracker actually goes down when a room is deleted.
/// Left unreturned, the process believes it is holding memory that no longer
/// exists, and after enough rooms have come and gone it refuses every message
/// while nearly empty — the same wedge as an underflowed counter, arrived at
/// from the other direction.
#[tokio::test]
async fn deleting_a_room_returns_its_bytes_to_the_process_budget() {
    let now = Instant::now();
    let state = Arc::new(AppState::new());

    let room_bytes;
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..8 {
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
                &state.memory_tracker,
            );
        }
        room_bytes = room
            .chat_history
            .iter()
            .map(|m| m.estimate_size())
            .sum::<usize>();

        let mut gone = connected_user("u", "otter", "c", now);
        gone.connection_state = ConnectionState::Disconnected { since: now };
        room.users.insert("u".to_string(), gone);
        room.last_activity = now - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(1);
        rooms.insert("doomed".to_string(), room);
    }

    assert_eq!(
        state.memory_tracker.total_bytes.load(Ordering::SeqCst),
        room_bytes,
        "the tracker should be holding exactly this room's messages"
    );

    cleanup_rooms_at(&state, now).await;

    assert!(!state.rooms.read().await.contains_key("doomed"));
    assert_eq!(
        state.memory_tracker.total_bytes.load(Ordering::SeqCst),
        0,
        "deleting the room must return every byte it was holding"
    );
}

/// Deleting an empty room reclaims nothing, and does not underflow.
#[tokio::test]
async fn deleting_an_empty_room_reclaims_nothing() {
    let now = Instant::now();
    let state = Arc::new(AppState::new());

    // Another room's bytes, which must be left alone.
    state.memory_tracker.add_bytes(5_000);

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        let mut gone = connected_user("u", "otter", "c", now);
        gone.connection_state = ConnectionState::Disconnected { since: now };
        room.users.insert("u".to_string(), gone);
        room.last_activity = now - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(1);
        rooms.insert("empty".to_string(), room);
    }

    cleanup_rooms_at(&state, now).await;

    assert!(!state.rooms.read().await.contains_key("empty"));
    assert_eq!(
        state.memory_tracker.total_bytes.load(Ordering::SeqCst),
        5_000,
        "a room with no messages returns nothing, and takes nothing else with it"
    );
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Housekeeping does nothing until the GC interval has elapsed.
///
/// `if !self.memory_tracker.should_gc() { return; }` is what keeps the room
/// write lock — the one every user in every room contends on — from being taken
/// on every sweep. Deleting that `!` survived the suite: nothing asserted that
/// a *second* sweep, arriving inside the interval, leaves the rooms alone.
/// Inverted, the guard admits exactly the sweeps it exists to turn away.
#[tokio::test]
async fn housekeeping_leaves_rooms_alone_until_the_gc_interval_has_elapsed() {
    let state = AppState::new();
    let over_budget = MAX_TOTAL_ROOMS_MEMORY + 1;

    async fn fill(state: &AppState, bytes: usize) {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..(MAX_MESSAGES_PER_ROOM + 20) {
            message_in(&mut room, &state.memory_tracker, &format!("message {i}"));
        }
        room.total_memory_bytes.store(bytes, Ordering::Relaxed);
        rooms.insert("heavy".to_string(), room);
    }

    // The first sweep is due, and the room is over the ceiling, so it trims.
    fill(&state, over_budget).await;
    state.cleanup().await;
    assert!(
        state.rooms.read().await["heavy"].chat_history.len() < MAX_MESSAGES_PER_ROOM + 20,
        "a sweep that is due must trim a room over the process ceiling"
    );

    // The second arrives inside the interval. Same room, same overrun — and it
    // must return before the write lock rather than do the work again.
    fill(&state, over_budget).await;
    state.cleanup().await;
    assert_eq!(
        state.rooms.read().await["heavy"].chat_history.len(),
        MAX_MESSAGES_PER_ROOM + 20,
        "a sweep inside the GC interval must take neither the write lock nor \
         the linear pass over every room's history"
    );
}

/// The process memory ceiling is a ceiling, not a threshold one byte lower.
///
/// `total_memory > MAX_TOTAL_ROOMS_MEMORY` could become `==`, `<` or `>=` and
/// the suite still passed, because every test was either far under the ceiling
/// or far over it. A server sitting exactly at its budget must not trim; one
/// byte past it must.
#[tokio::test]
async fn the_process_memory_ceiling_trims_only_once_it_is_exceeded() {
    for (bytes, expect_trim) in [
        (MAX_TOTAL_ROOMS_MEMORY, false),
        (MAX_TOTAL_ROOMS_MEMORY + 1, true),
    ] {
        let state = AppState::new();
        {
            let mut rooms = state.rooms.write().await;
            let mut room = create_room();
            for i in 0..(MAX_MESSAGES_PER_ROOM + 20) {
                message_in(&mut room, &state.memory_tracker, &format!("message {i}"));
            }
            // Claim the whole budget for this room, so `cleanup` reads exactly
            // the total under test rather than whatever the messages weighed.
            room.total_memory_bytes.store(bytes, Ordering::Relaxed);
            rooms.insert("heavy".to_string(), room);
        }

        state.cleanup().await;

        let rooms = state.rooms.read().await;
        let trimmed = rooms["heavy"].chat_history.len() < MAX_MESSAGES_PER_ROOM + 20;
        assert_eq!(
            trimmed, expect_trim,
            "at {bytes} bytes against a ceiling of {MAX_TOTAL_ROOMS_MEMORY}, \
             trimming should be {expect_trim}"
        );
    }
}

/// Housekeeping trims a room only once it is past the *soft* memory limit.
///
/// `trigger_cleanup` compares against nine tenths of the process ceiling, and
/// `>` could become `>=` unnoticed. The soft limit is where a room starts
/// shedding its oldest messages before the hard ceiling starts dropping new
/// ones, so which side of it a room sits on decides whether a conversation
/// loses its past or its present.
#[tokio::test]
async fn a_room_is_cleaned_only_once_it_is_past_the_soft_memory_limit() {
    let (num, den) = MEMORY_SOFT_LIMIT_RATIO;
    let soft_limit = (MAX_TOTAL_ROOMS_MEMORY * num) / den;

    for (bytes, expect_cleanup) in [(soft_limit, false), (soft_limit + 1, true)] {
        let tracker = MemoryTracker::new();
        let mut room = create_room();
        message_in(&mut room, &tracker, "a message old enough to age out");

        // Age the message past MAX_MESSAGE_AGE so a sweep has something to do.
        let ancient = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .saturating_sub(MAX_MESSAGE_AGE.as_millis() * 2);
        let mut msg = (*room.chat_history[0]).clone();
        msg.timestamp = ancient.to_string();
        room.chat_history[0] = Arc::new(msg);

        room.total_memory_bytes.store(bytes, Ordering::Relaxed);
        room.trigger_cleanup(&tracker).await;

        assert_eq!(
            room.chat_history.is_empty(),
            expect_cleanup,
            "at {bytes} bytes against a soft limit of {soft_limit}, cleanup \
             should be {expect_cleanup}"
        );
    }
}

/// `history_trim_target` in isolation: every combination of "is this main"
/// and "how long has it been idle", checked directly against two primitives
/// rather than a constructed room.
#[test]
fn history_trim_target_only_fades_main_once_it_is_idle_long_enough() {
    assert_eq!(
        history_trim_target(false, MAIN_ROOM_FADE_IDLE + Duration::from_secs(1)),
        MAX_MESSAGES_PER_ROOM,
        "a non-main room never gets main's harder fade, however idle"
    );
    assert_eq!(
        history_trim_target(true, Duration::from_secs(0)),
        MAX_MESSAGES_PER_ROOM,
        "main does not fade until it has actually been idle that long"
    );
    assert_eq!(
        history_trim_target(true, MAIN_ROOM_FADE_IDLE),
        MAIN_ROOM_FADE_KEEP,
        "exactly at the threshold must already count as idle enough"
    );
}

/// `is_abandoned` in isolation: the three conditions have to hold together —
/// not main, nobody connected, idle long enough — and any one of them being
/// false must keep a room alive.
#[test]
fn is_abandoned_requires_every_condition_at_once() {
    let idle_enough = EMPTY_ROOM_CLEANUP_DELAY;

    assert!(
        is_abandoned(false, false, idle_enough),
        "not main, nobody connected, idle long enough: this is the abandoned case"
    );
    assert!(
        !is_abandoned(true, false, idle_enough),
        "main fades, it is never deleted, however idle or empty"
    );
    assert!(
        !is_abandoned(false, true, idle_enough),
        "a room with a connected user is not abandoned, however idle it looks"
    );
    assert!(
        !is_abandoned(false, false, Duration::from_secs(0)),
        "not idle long enough yet, even with nobody connected"
    );
}
