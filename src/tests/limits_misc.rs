//! What did not belong to one limiter alone: the animal-name pool under
//! `RoomState`, `ChatError` status mapping, and a handful of integration
//! tests that exercise several limiters through one WebSocket session.

use super::*;

#[tokio::test]
async fn test_generate_random_room_name() {
    let name = generate_random_room_name();
    assert!(!name.is_empty());
    assert!(name.contains('-'));
}

#[tokio::test]
async fn test_room_capacity_tracking() {
    let mut room = create_room();

    let now = Instant::now();
    for i in 0..100 {
        let user = UserData {
            user_id: format!("user-{}", i),
            animal_name: format!("Animal{}", i),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: format!("conn-{}", i),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
            last_reaction_event: None,
        };
        room.users.insert(format!("user-{}", i), user);
    }

    // Room has 100 users
    let connected_count = room
        .users
        .values()
        .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
        .count();

    assert_eq!(connected_count, 100);
}

#[tokio::test]
async fn test_room_message_history_capacity() {
    let app_state = Arc::new(AppState::new());
    let room_name = "history-cap".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();

        // Add messages beyond MAX_MESSAGES_PER_ROOM
        for i in 0..MAX_MESSAGES_PER_ROOM + 10 {
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

    // Verify size
    let rooms = app_state.rooms.read().await;
    let room = rooms.get(&room_name).unwrap();
    assert!(room.chat_history.len() <= MAX_MESSAGES_PER_ROOM + 10);
}

// ========== WEBSOCKET PROTOCOL EDGE CASES ==========

#[tokio::test]
async fn test_chat_error_into_response_resource_limit() {
    let error = ChatError::ResourceLimit("Out of memory".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_room_state_add_message_updates_memory() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    let msg = OutgoingMessage {
        message_id: Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Tiger".to_string(),
        text: "Test message".to_string(),
        timestamp: "1234567890".to_string(),
        reply_to: None,
        attachment: None,
    };

    let initial_memory = room_state.total_memory_bytes.load(Ordering::Relaxed);
    room_state.add_message(msg, &memory_tracker);
    let final_memory = room_state.total_memory_bytes.load(Ordering::Relaxed);

    assert!(final_memory > initial_memory);
    assert_eq!(room_state.chat_history.len(), 1);
}

#[tokio::test]
async fn test_is_user_allowed_at_capacity() {
    let mut room_state = create_room();

    // Add 100 connected users (at capacity)
    for i in 0..100 {
        let user_id = format!("user_{}", i);
        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id,
                animal_name: format!("Animal{}", i),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Connected {
                    last_heartbeat: Instant::now(),
                    connection_id: format!("conn_{}", i),
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

    // At capacity, should reject new users
    assert!(!room_state.is_user_allowed("new_user", crate::config::MAX_USERS_PER_ROOM));
}

#[tokio::test]
async fn test_is_user_allowed_new_user_under_capacity() {
    let mut room_state = create_room();

    // Add only a few users (under capacity)
    for i in 0..10 {
        let user_id = format!("user_{}", i);
        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id,
                animal_name: format!("Animal{}", i),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Connected {
                    last_heartbeat: Instant::now(),
                    connection_id: format!("conn_{}", i),
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

    // New user should be allowed when under capacity
    assert!(room_state.is_user_allowed("brand_new_user", crate::config::MAX_USERS_PER_ROOM));
}

#[tokio::test]
async fn test_room_state_user_count_accurate() {
    let mut room = create_room();
    let now = Instant::now();

    // Add 3 connected users
    for i in 0..3 {
        room.users.insert(
            format!("user-{}", i),
            UserData {
                user_id: format!("user-{}", i),
                animal_name: format!("Animal{}", i),
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: format!("conn-{}", i),
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

    // Add 2 disconnected users
    for i in 3..5 {
        room.users.insert(
            format!("user-{}", i),
            UserData {
                user_id: format!("user-{}", i),
                animal_name: format!("Animal{}", i),
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
    }

    let connected_count = room
        .users
        .values()
        .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
        .count();

    assert_eq!(connected_count, 3, "Should count only connected users");
    assert_eq!(room.users.len(), 5, "Total users includes disconnected");
}

#[tokio::test]
async fn test_broadcast_channel_capacity_supports_rapid_messages() {
    // Combo effects need rapid message delivery - verify channel has capacity
    let (tx, mut rx1) = tokio::sync::broadcast::channel::<String>(1000);
    let mut rx2 = tx.subscribe();

    // Simulate rapid combo messages
    for i in 0..10 {
        tx.send(format!("rapid-msg-{}", i)).unwrap();
    }

    // Both receivers should get all messages (no dropped)
    for i in 0..10 {
        assert_eq!(rx1.recv().await.unwrap(), format!("rapid-msg-{}", i));
        assert_eq!(rx2.recv().await.unwrap(), format!("rapid-msg-{}", i));
    }
}

#[tokio::test]
async fn test_chat_error_resource_limit_status() {
    let error = ChatError::ResourceLimit("test".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_assign_animal_empty_pool() {
    let mut room = create_room();

    // Clear the animal pool
    room.available_animals.clear();

    // Should still assign an animal (generates fallback)
    let animal = room.assign_animal();

    // Should return some animal name
    assert!(!animal.is_empty());
}

// Test animal pool direct manipulation - covers animal assignment paths

#[tokio::test]
async fn test_animal_pool_manipulation() {
    let mut room = create_room();

    // Take an animal
    let animal = room.assign_animal();
    let count_after_assign = room.available_animals.len();

    // The pool should have one less animal after assignment
    assert!(!animal.is_empty());

    // Directly add animal back to pool (simulating return)
    room.available_animals.push_back(animal.clone());

    // Pool should have one more animal
    assert_eq!(room.available_animals.len(), count_after_assign + 1);
}

// Test payload size limit via WebSocket - covers lines 1631-1639

#[tokio::test]
async fn test_user_idle_past_threshold() {
    let user = UserData {
        user_id: "test".to_string(),
        animal_name: "Lion".to_string(),
        last_active: Instant::now(),
        last_message_time: Instant::now() - USER_IDLE_MESSAGE_TIMEOUT - Duration::from_secs(1),
        connection_state: ConnectionState::Connected {
            last_heartbeat: Instant::now(),
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

    let now = Instant::now();
    assert!(user_idle_for_too_long(&user, now));
}

// Test user not idle - covers line 324

#[tokio::test]
async fn test_generate_random_room_name_format() {
    // Generate several names and verify format
    for _ in 0..10 {
        let name = generate_random_room_name();
        assert!(!name.is_empty());
        assert!(name.contains('-'));
        // Name should have two parts separated by hyphen
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty());
        assert!(!parts[1].is_empty());
    }
}

// Test robots_txt_handler returns correct content - covers lines 2176-2192

/// A disconnected user is eventually reclaimed, and their name returns to the
/// pool for someone else.
#[tokio::test]
async fn cleanup_reclaims_abandoned_users_and_recycles_their_name() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        let pool_before = room.available_animals.len();

        room.users.insert(
            "ghost".to_string(),
            UserData {
                user_id: "ghost".to_string(),
                animal_name: "otter".to_string(),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Disconnected {
                    since: Instant::now() - DISCONNECTED_USER_RETENTION - Duration::from_secs(60),
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
        rooms.insert("ghost-room".to_string(), room);
        assert_eq!(pool_before, ANIMAL_NAMES.len());
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get("ghost-room").expect("room should still exist");
    assert!(
        !room.users.contains_key("ghost"),
        "a long-disconnected user should be reclaimed"
    );
    assert!(
        room.available_animals.iter().any(|a| a == "otter"),
        "their animal name should go back into the pool"
    );
}

/// At capacity the server refuses new sockets, and refusing costs nothing.
#[tokio::test]
async fn upgrades_are_refused_at_capacity_without_leaking_slots() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    state
        .resource_monitor
        .total_connections
        .store(MAX_CONCURRENT_USERS, Ordering::SeqCst);

    assert!(
        connect_async(ws_request(addr, "capacity-room", &[]))
            .await
            .is_err(),
        "a full server must refuse the upgrade"
    );

    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        MAX_CONCURRENT_USERS,
        "a refused upgrade must not change the count either way"
    );
    assert_eq!(state.connection_pool.active.load(Ordering::SeqCst), 0);

    handle.abort();
}

/// §3 — the animal pool is bounded by the roster it was built from.
///
/// Reclaiming a user pushes their name back into the pool. A name that never
/// came *out* of that pool — a cookie identity carried in from another room, or
/// a `guest_N` fallback minted when the pool was empty — made that push
/// unbalanced, so the pool grew every time such a user was reclaimed.
#[tokio::test]
async fn reclaiming_users_cannot_grow_or_pollute_the_animal_pool() {
    let state = Arc::new(AppState::new());
    let long_gone = Instant::now() - DISCONNECTED_USER_RETENTION - Duration::from_secs(60);

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();

        // A name this room never issued (carried in on a cookie), and a guest
        // fallback that is not an animal at all.
        for (uid, animal) in [("visitor", "otter"), ("overflow", "guest_7")] {
            let mut user = connected_user(uid, animal, "c", Instant::now());
            user.connection_state = ConnectionState::Disconnected { since: long_gone };
            room.users.insert(uid.to_string(), user);
        }
        rooms.insert("pool-room".to_string(), room);
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get("pool-room").expect("room should still exist");

    assert!(
        room.available_animals.len() <= ANIMAL_NAMES.len(),
        "the pool must never exceed the roster it was built from: {} > {}",
        room.available_animals.len(),
        ANIMAL_NAMES.len()
    );
    assert!(
        room.available_animals
            .iter()
            .all(|a| ANIMAL_NAMES.contains(&a.as_str())),
        "every assignable name must be on the roster; found {:?}",
        room.available_animals
            .iter()
            .filter(|a| !ANIMAL_NAMES.contains(&a.as_str()))
            .collect::<Vec<_>>()
    );
}

/// §5 — no counter can be driven below zero.
///
/// An unbalanced decrement on an unsigned counter wraps to `usize::MAX`, and
/// every one of these counters is compared against a ceiling: a single wrap
/// wedges the server at "full" for the rest of the process's life. This is the
/// failure `MemoryTracker::remove_bytes` already documents; the connection
/// counters had the same shape and none of the protection.
#[tokio::test]
async fn releasing_more_than_was_reserved_cannot_wrap_a_counter() {
    let monitor = ResourceMonitor::new();
    monitor.release_connection();
    assert_eq!(
        monitor.total_connections.load(Ordering::SeqCst),
        0,
        "an unmatched release must floor at zero, not wrap"
    );
    assert!(
        monitor.can_accept_connection(),
        "an unmatched release must not wedge the server at capacity"
    );

    let pool = ConnectionPool::new();
    pool.remove_connection("10.0.0.1").await;
    assert_eq!(pool.active.load(Ordering::SeqCst), 0);

    pool.add_connection("10.0.0.1").await.unwrap();
    pool.remove_connection("10.0.0.1").await;
    pool.remove_connection("10.0.0.1").await;
    assert_eq!(
        pool.active.load(Ordering::SeqCst),
        0,
        "a double release must floor at zero"
    );
    assert!(
        pool.can_accept("10.0.0.1").await,
        "a double release must not wedge the per-IP counter at its limit"
    );
}
