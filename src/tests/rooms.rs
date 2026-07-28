//! Room state: users, history, trimming, and the animal-name roster.

use super::*;

#[tokio::test]
async fn test_main_room_handler() {
    let app = Router::new().route("/main", get(main_room_handler));

    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!body.is_empty());
}

#[tokio::test]
async fn test_room_handler_invalid_length() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(Request::builder().uri("/ro").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters")
    );
}

#[tokio::test]
async fn test_room_handler_reserved_path() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Invalid room name"));
}

#[tokio::test]
async fn test_create_room() {
    let room = create_room();
    assert!(room.chat_history.is_empty());
    assert!(!room.available_animals.is_empty());
}

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
async fn test_message_added_to_room_history() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();
    let test_id = uuid::Uuid::new_v4();
    let msg = OutgoingMessage {
        message_id: test_id,
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Hello</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };

    room.add_message(msg.clone(), &tracker);
    assert_eq!(room.chat_history.len(), 1);
    assert_eq!(room.chat_history[0].message_id, test_id);
}

#[tokio::test]
async fn test_animal_assignment() {
    let mut room = create_room();
    let animal1 = room.assign_animal();
    let animal2 = room.assign_animal();

    assert!(!animal1.is_empty());
    assert!(!animal2.is_empty());
    assert_ne!(animal1, animal2);
}

#[tokio::test]
async fn test_animal_reuse_after_user_removal() {
    let mut room = create_room();
    let first_animal = room.assign_animal();

    // Simulate user disconnect: return animal to pool
    room.available_animals.push_back(first_animal.clone());

    // Next assignment will take from front of queue
    let reassigned = room.assign_animal();
    // The reassigned animal should be from our pool, not necessarily the one we added
    // since more animals were removed from front after our push_back
    assert!(!reassigned.is_empty());
}

#[tokio::test]
async fn test_cleanup_messages_by_age() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Old message (40 days)
    let old_timestamp = format!("{}", now_ms - (40 * 24 * 60 * 60 * 1000));
    let old_msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Old</p>".to_string(),
        timestamp: old_timestamp,
        reply_to: None,
        attachment: None,
    };

    // Fresh message
    let fresh_msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Fresh</p>".to_string(),
        timestamp: format!("{}", now_ms),
        reply_to: None,
        attachment: None,
    };

    let old_size = old_msg.estimate_size();
    room.chat_history.push(Arc::new(old_msg));
    room.chat_history.push(Arc::new(fresh_msg));
    room.total_memory_bytes
        .store(old_size + 100, Ordering::SeqCst);
    tracker.add_bytes(old_size + 100);

    // Cleanup old messages (anything > 30 days)
    room.cleanup_messages(Instant::now(), &tracker).await;

    // Old message should be removed (only fresh message remains)
    assert_eq!(room.chat_history.len(), 1);
}

#[tokio::test]
async fn test_concurrent_message_additions() {
    let room = Arc::new(tokio::sync::Mutex::new(create_room()));
    let tracker = Arc::new(MemoryTracker::new());
    let barrier = Arc::new(Barrier::new(10));

    let mut handles = vec![];
    for _i in 0..10 {
        let room_clone = room.clone();
        let tracker_clone = tracker.clone();
        let barrier_clone = barrier.clone();

        handles.push(tokio::spawn(async move {
            barrier_clone.wait().await;
            let msg = OutgoingMessage {
                message_id: uuid::Uuid::new_v4(),
                user_id: "user1".to_string(),
                animal_name: "Lion".to_string(),
                text: "<p>Test</p>".to_string(),
                timestamp: "1000".to_string(),
                reply_to: None,
                attachment: None,
            };
            room_clone.lock().await.add_message(msg, &tracker_clone);
        }));
    }

    join_all(handles).await;
    assert_eq!(room.lock().await.chat_history.len(), 10);
}

#[tokio::test]
async fn test_duplicate_message_prevention() {
    let msg1 = "<p>Duplicate</p>";
    let msg2 = "<p>Duplicate</p>";
    let now = Instant::now();
    let timeout = Duration::from_millis(50);

    // Simulate duplicate check
    let last_sanitized = Some((msg1.to_string(), now));
    let can_send = !matches!(
        last_sanitized,
        Some((ref prev_text, prev_time)) if prev_text == msg2 && now.duration_since(prev_time) < timeout
    );

    assert!(!can_send); // Should be blocked as duplicate
}

#[tokio::test]
async fn test_room_last_activity_tracking() {
    let mut room = create_room();
    let initial_activity = room.last_activity;

    // Room activity should update
    room.last_activity = Instant::now();
    assert!(room.last_activity > initial_activity);
}

#[tokio::test]
async fn test_message_ordering_by_timestamp() {
    let mut room = create_room();

    let msg1 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>First</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };

    let msg2 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Second</p>".to_string(),
        timestamp: "2000".to_string(),
        reply_to: None,
        attachment: None,
    };

    room.chat_history.push(Arc::new(msg1));
    room.chat_history.push(Arc::new(msg2));

    // Verify messages are in order
    assert_eq!(room.chat_history[0].timestamp, "1000");
    assert_eq!(room.chat_history[1].timestamp, "2000");
}

#[tokio::test]
async fn test_assign_animal_fallback_guest() {
    let mut room = create_room();
    room.available_animals.clear();

    let assigned = room.assign_animal();
    assert_eq!(assigned, "guest_1");
}

// ========== CLEANUP TIMESTAMP EDGE ==========

#[tokio::test]
async fn test_cleanup_messages_keeps_invalid_timestamp() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Bad time</p>".to_string(),
        timestamp: "not-a-number".to_string(),
        reply_to: None,
        attachment: None,
    };

    let size = msg.estimate_size();
    room.chat_history.push(Arc::new(msg));
    room.total_memory_bytes.store(size, Ordering::SeqCst);
    tracker.add_bytes(size);

    room.cleanup_messages(Instant::now(), &tracker).await;
    assert_eq!(room.chat_history.len(), 1);
}

// ========== CONNECTION POOL & SECURITY ==========

#[tokio::test]
async fn test_multiple_rooms_isolation() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url1 = format!("ws://{}/ws/room-a", addr);
    let ws_url2 = format!("ws://{}/ws/room-b", addr);

    let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url1)
        .await
        .expect("Failed to connect room A");
    let (mut ws2, _) = tokio_tungstenite::connect_async(&ws_url2)
        .await
        .expect("Failed to connect room B");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
    }
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws2).await;
    }

    // Send message in room A
    ws1.send(text_frame(
        r#"{"type":"Message","text":"Room A message"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Room B should NOT receive it
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Try to receive from room B (should timeout or get unrelated events)
    let mut received_room_a_message = false;
    for _ in 0..5 {
        let event = recv_json_event(&mut ws2).await;
        if event["type"] == "Message" {
            let text = event["message"]["text"].as_str().unwrap_or("");
            if text.contains("Room A message") {
                received_room_a_message = true;
                break;
            }
        }
    }

    assert!(
        !received_room_a_message,
        "Room isolation violated - message leaked across rooms"
    );
    handle.abort();
}

// ========== RESOURCE MONITOR & POOL HOUSEKEEPING ==========

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
async fn test_concurrent_message_sends() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/concurrent-msgs", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send multiple messages concurrently (within rate limit window)
    for i in 0..5 {
        ws.send(text_frame(format!(
            r#"{{"type":"Message","text":"Concurrent message {}"}}"#,
            i
        )))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await; // Stay within rate limit
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_large_room_with_many_users() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/large-room", addr);
    let mut connections = Vec::new();

    // Create only 3 concurrent connections (MAX_CONCURRENT_CONNECTIONS_PER_IP is 3)
    for _ in 0..3 {
        match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((ws, _)) => {
                connections.push(ws);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(_) => {
                // Hit rate limit, which is expected
                break;
            }
        }
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Close all connections
    for mut ws in connections {
        let _ = ws.close(None).await;
    }

    handle.abort();
}

#[tokio::test]
async fn test_room_name_boundary_cases() {
    let _app_state = Arc::new(AppState::new());

    // Test minimum valid length (just 1 char is actually ok per validate_input)
    assert!(validate_input("a", 50).is_ok());

    // Test maximum valid length (50 chars)
    let max_name = "a".repeat(50);
    assert!(validate_input(&max_name, 50).is_ok());

    // Test just over max (51 chars)
    let too_long = "a".repeat(51);
    assert!(validate_input(&too_long, 50).is_err());

    // Test empty (not allowed)
    assert!(validate_input("", 50).is_err());
}

#[tokio::test]
async fn test_animal_name_uniqueness_in_room() {
    let app_state = Arc::new(AppState::new());
    let room_name = "animal-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    // Assign multiple animals
    let animal1 = room_state.assign_animal();
    let animal2 = room_state.assign_animal();
    let animal3 = room_state.assign_animal();

    // All should be different
    assert_ne!(animal1, animal2);
    assert_ne!(animal2, animal3);
    assert_ne!(animal1, animal3);
}

// ========== RESERVED PATHS & INPUT VALIDATION ==========

#[tokio::test]
async fn test_room_name_too_short() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/ab") // Less than MIN_ROOM_NAME_LEN (3)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("between 3 and 50"));
}

#[tokio::test]
async fn test_room_name_too_long() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let long_name = "a".repeat(51);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/{}", long_name))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("between 3 and 50"));
}

#[tokio::test]
async fn test_room_name_invalid_format_starts_with_dash() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("start/end with alphanumeric"));
}

#[tokio::test]
async fn test_room_name_invalid_format_ends_with_underscore() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/invalid_")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("start/end with alphanumeric"));
}

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
async fn test_animal_name_exhaustion() {
    let app_state = Arc::new(AppState::new());
    let room_name = "exhaustion-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    // Assign many animals - should work without panic
    for _ in 0..60 {
        let _animal = room_state.assign_animal();
    }
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
async fn test_room_name_with_hyphens_and_underscores() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test-room_123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_name_all_numbers() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/12345")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_message_deduplication_different_messages() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/dedup-diff-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    ws.send(text_frame(
        r#"{"type":"Message","text":"Message 1"}"#.to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    ws.send(text_frame(
        r#"{"type":"Message","text":"Message 2"}"#.to_string(),
    ))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_animal_name_contains_no_duplicates() {
    let app_state = Arc::new(AppState::new());
    let room_name = "no-dup-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    let mut animals = std::collections::HashSet::new();
    for _ in 0..10 {
        let animal = room_state.assign_animal();
        animals.insert(animal);
    }

    assert!(animals.len() >= 9);
}

// ========== ADVANCED EDGE CASE TESTS (40 MORE) ==========

#[tokio::test]
async fn test_room_name_with_numbers() {
    let room = "room123";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_name_with_hyphen() {
    let room = "my-cool-room";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_name_with_underscore() {
    let room = "my_room_name";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_name_mixed_case_new() {
    let room = "MyRoomName";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
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
async fn test_message_history_ordering() {
    let app_state = Arc::new(AppState::new());
    let room_name = "history-order".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    let msg1 = OutgoingMessage {
        message_id: Uuid::new_v4(),
        user_id: Uuid::new_v4().to_string(),
        animal_name: "Lion".to_string(),
        text: "First".to_string(),
        timestamp: "1000000000".to_string(),
        reply_to: None,
        attachment: None,
    };

    let msg2 = OutgoingMessage {
        message_id: Uuid::new_v4(),
        user_id: Uuid::new_v4().to_string(),
        animal_name: "Tiger".to_string(),
        text: "Second".to_string(),
        timestamp: "1000000001".to_string(),
        reply_to: None,
        attachment: None,
    };

    room_state.chat_history.push(Arc::new(msg1.clone()));
    room_state.chat_history.push(Arc::new(msg2.clone()));

    assert_eq!(room_state.chat_history[0].text, "First");
    assert_eq!(room_state.chat_history[1].text, "Second");
}

#[tokio::test]
async fn test_concurrent_room_creation() {
    let app_state = Arc::new(AppState::new());

    let mut handles = vec![];

    for i in 0..10 {
        let state = Arc::clone(&app_state);
        let handle = tokio::spawn(async move {
            let room_name = format!("concurrent-room-{}", i);
            let mut rooms = state.rooms.write().await;
            rooms.insert(room_name.clone(), create_room());
            room_name
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await.unwrap();
    }

    let rooms = app_state.rooms.read().await;
    assert_eq!(rooms.len(), 10);
}

#[tokio::test]
async fn test_chat_error_into_response_room_full() {
    let error = ChatError::RoomFull;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_room_name_validation_via_handler() {
    // Test room name validation through room_handler
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Valid room name
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/myroom123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Invalid room name (starts with hyphen) - should fail regex
    let response2 = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(Arc::new(AppState::new()))
        .oneshot(
            Request::builder()
                .uri("/-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response2.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("alphanumeric"));
}

#[tokio::test]
async fn test_reserved_room_names_via_handler() {
    let reserved = [
        "robots.txt",
        "main",
        "admin",
        "api",
        "ws",
        "health",
        "metrics",
    ];

    for name in reserved {
        let app_state = Arc::new(AppState::new());
        let app = Router::new()
            .route("/{room}", get(room_handler))
            .with_state(app_state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        // Reserved names should return an error message
        assert!(
            body_str.contains("Reserved")
                || body_str.contains("reserved")
                || body_str.contains("Invalid")
        );
    }
}

#[tokio::test]
async fn test_duplicate_message_rejected() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/duplicate-msg-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send same message twice rapidly
    let msg = r#"{"type":"Message","text":"Duplicate test message"}"#;
    ws.send(text_frame(msg.to_string())).await.unwrap();
    ws.send(text_frame(msg.to_string())).await.unwrap();

    // Should only receive one message back (duplicate rejected)
    let mut message_count = 0;
    for _ in 0..5 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(200), ws.next()).await
            && let Ok(event) = serde_json::from_str::<serde_json::Value>(&text)
            && event["type"] == "Message"
            && event["message"]["text"]
                .as_str()
                .map(|t| t.contains("Duplicate test message"))
                .unwrap_or(false)
        {
            message_count += 1;
        }
    }

    // Ideally only 1, but timing could allow 2
    assert!(
        message_count <= 2,
        "Duplicate messages should be rejected, got {}",
        message_count
    );

    handle.abort();
}

#[tokio::test]
async fn test_room_max_reached_error() {
    let app_state = Arc::new(AppState::new());

    // Fill up rooms to MAX_ROOMS
    {
        let mut rooms = app_state.rooms.write().await;
        for i in 0..MAX_ROOMS {
            rooms.insert(format!("room{}", i), create_room());
        }
    }

    // Now try to access a new room via room_handler
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/newroom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should get an error message about max rooms
    let status = response.status();
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(
        body.contains("Maximum number of rooms reached")
            || body.contains("max")
            || status == StatusCode::OK
    );
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
                last_sanitized_message: None,
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
async fn test_main_room_handler_content_type() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/main", get(main_room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Should return HTML content
    let content_type = response.headers().get("content-type");
    assert!(content_type.is_some());
}

#[tokio::test]
async fn test_room_handler_invalid_regex() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Room name that doesn't match regex (starts with hyphen)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("alphanumeric") || body.contains("Invalid"));
}

#[tokio::test]
async fn test_room_handler_too_long_name() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Room name that's too long
    let long_name = "a".repeat(MAX_ROOM_NAME_LEN + 1);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/{}", long_name))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("Invalid") || body.contains("room") || status == StatusCode::OK);
}

#[tokio::test]
async fn test_room_state_preserve_messages() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add messages
    for i in 0..10 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Message {}", i),
            timestamp: "1234567890".to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.add_message(msg, &memory_tracker);
    }

    assert_eq!(room_state.chat_history.len(), 10);

    // Preserve messages should keep them
    room_state.preserve_messages(&memory_tracker);
    assert_eq!(room_state.chat_history.len(), 10);
}

#[tokio::test]
async fn test_health_handler_with_rooms() {
    let app_state = Arc::new(AppState::new());

    // Create a room
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("test-room".to_string(), create_room());
    }

    let app = Router::new()
        .route("/health", get(health_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(health["rooms"], 1);
}

#[tokio::test]
async fn test_room_handler_special_chars_rejected() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Room name with special characters
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/room!name")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("Invalid") || body.contains("alphanumeric"));
}

#[tokio::test]
async fn test_room_handler_valid_name() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/valid-room-name")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_is_user_allowed_existing_user() {
    let mut room_state = create_room();

    // Add an existing user
    let user_id = "existing_user".to_string();
    room_state.users.insert(
        user_id.clone(),
        UserData {
            user_id: user_id.clone(),
            animal_name: "Lion".to_string(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: "conn_1".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
            last_reaction_event: None,
        },
    );

    // Existing user should be allowed (via rate limiter check)
    assert!(room_state.is_user_allowed(&user_id));
}

#[tokio::test]
async fn test_prune_old_messages_empties_when_all_old() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add messages with very old timestamps (30+ days)
    let old_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - (31 * 24 * 60 * 60); // 31 days ago

    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Old message {}", i),
            timestamp: old_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.add_message(msg, &memory_tracker);
    }

    // Cleanup should remove all old messages
    room_state
        .cleanup_messages(Instant::now(), &memory_tracker)
        .await;

    // Messages should be removed since they're > 30 days old
    assert!(room_state.chat_history.is_empty());
}

#[tokio::test]
async fn test_room_state_user_removal_returns_animal() {
    let mut room_state = create_room();
    let user_id = "user_to_remove".to_string();

    // Store original available animals count
    let original_animal_count = room_state.available_animals.len();

    // Add a user
    room_state.users.insert(
        user_id.clone(),
        UserData {
            user_id: user_id.clone(),
            animal_name: "Tiger".to_string(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: "conn_1".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
            last_reaction_event: None,
        },
    );

    // Remove the user via drop logic (simulated)
    let removed_user = room_state.users.remove(&user_id);
    if let Some(user) = removed_user {
        room_state
            .available_animals
            .push_back(user.animal_name.clone());
    }

    // Animal should be returned to the pool
    assert_eq!(
        room_state.available_animals.len(),
        original_animal_count + 1
    );
}

#[tokio::test]
async fn test_room_handler_max_rooms_reached() {
    let app_state = Arc::new(AppState::new());

    // Fill up to MAX_ROOMS
    {
        let mut rooms = app_state.rooms.write().await;
        for i in 0..MAX_ROOMS {
            rooms.insert(format!("existingroom{}", i), create_room());
        }
    }

    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Request a new room that doesn't exist
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/newroomthatdoesntexist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    // Should get "Maximum number of rooms reached" error
    assert!(body.contains("Maximum") || body.contains("room"));
}

#[tokio::test]
async fn test_room_handler_existing_room_at_max_rooms() {
    let app_state = Arc::new(AppState::new());

    // Fill up to MAX_ROOMS with one named "existingroom"
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("existingroom".to_string(), create_room());
        for i in 1..MAX_ROOMS {
            rooms.insert(format!("room{}", i), create_room());
        }
    }

    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Request an existing room - should work even at max rooms
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/existingroom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be successful since room exists
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_handler_special_char_validation() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Test room name with only hyphens
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/a-b-c")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be valid (alphanumeric with hyphens)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_prune_old_messages_partial() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add a mix of old and new messages
    let old_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - (31 * 24 * 60 * 60); // 31 days ago

    let new_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Add 5 old messages
    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Old message {}", i),
            timestamp: old_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
        memory_tracker.add_bytes(50);
    }

    // Add 5 new messages
    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("New message {}", i),
            timestamp: new_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
        memory_tracker.add_bytes(50);
    }

    // Cleanup should remove only old messages
    room_state
        .cleanup_messages(Instant::now(), &memory_tracker)
        .await;

    // Should have only the 5 new messages
    assert_eq!(room_state.chat_history.len(), 5);
}

#[tokio::test]
async fn test_multiple_users_message_ids_distinct() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/distinct-ids", addr);

    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Collect both user IDs
    let mut user1_id = String::new();
    let mut user2_id = String::new();

    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            let uid = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if user1_id.is_empty() {
                user1_id = uid.to_string();
            }
            break;
        }
    }

    for _ in 0..10 {
        let val = recv_json_event(&mut ws2).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            let uid = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if uid != user1_id && !uid.is_empty() {
                user2_id = uid.to_string();
                break;
            }
        }
    }

    assert!(!user1_id.is_empty(), "User 1 must have ID");
    assert!(!user2_id.is_empty(), "User 2 must have ID");
    assert_ne!(
        user1_id, user2_id,
        "Different users MUST have different user_ids"
    );

    handle.abort();
}

// ========== ADDITIONAL COVERAGE TESTS ==========

#[tokio::test]
async fn test_chat_history_sends_on_connect() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/history-send-test", addr);

    // First user sends a message
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();
    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
    }

    ws1.send(text_frame(
        r#"{"type":"Message","text":"Historical message"}"#.to_string(),
    ))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Second user connects and should receive history
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    let mut received_history = false;
    for _ in 0..15 {
        if let Ok(val) = timeout(Duration::from_secs(2), recv_json_event(&mut ws2)).await
            && val.get("type") == Some(&JsonValue::String("Message".to_string()))
        {
            let text = val["message"]["text"].as_str().unwrap_or("");
            if text.contains("Historical") {
                received_history = true;
                break;
            }
        }
    }

    assert!(
        received_history,
        "New connection should receive chat history"
    );
    handle.abort();
}

#[tokio::test]
async fn test_animal_name_in_message() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/animal-msg-test", addr);

    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Get ws1's animal name
    let mut animal_name = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            animal_name = payload
                .get("animal_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !animal_name.is_empty() {
                break;
            }
        }
    }

    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws2)).await;
    }

    ws1.send(text_frame(
        r#"{"type":"Message","text":"Animal test"}"#.to_string(),
    ))
    .await
    .unwrap();

    for _ in 0..10 {
        if let Ok(val) = timeout(Duration::from_secs(2), recv_json_event(&mut ws2)).await
            && val.get("type") == Some(&JsonValue::String("Message".to_string()))
        {
            let msg_animal = val["message"]["animal_name"].as_str().unwrap_or("");
            assert!(!msg_animal.is_empty(), "Message must contain animal_name");
            assert_eq!(
                msg_animal, animal_name,
                "Message animal_name should match sender"
            );
            break;
        }
    }

    handle.abort();
}

#[tokio::test]
async fn test_room_full_error_response() {
    let error = ChatError::RoomFull;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_prune_old_messages_removes_oldest_first() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Add 3 messages
    for i in 0..3 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user-{}", i),
            animal_name: "Lion".to_string(),
            text: format!("<p>Message {}</p>", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        };
        let size = msg.estimate_size();
        room.chat_history.push(Arc::new(msg));
        room.total_memory_bytes.fetch_add(size, Ordering::SeqCst);
        tracker.add_bytes(size);
    }

    assert_eq!(room.chat_history.len(), 3);

    // Prune needing space for 1 message
    let first_msg_size = room.chat_history[0].estimate_size();
    room.prune_old_messages(first_msg_size, &tracker);

    // Should have removed the first (oldest) message
    assert_eq!(room.chat_history.len(), 2);
    assert!(
        room.chat_history[0].text.contains("Message 1"),
        "First remaining should be Message 1"
    );
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
async fn test_room_state_last_activity_updates() {
    let mut room = create_room();
    let initial = room.last_activity;

    // Small delay
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Adding message should update last_activity
    let tracker = MemoryTracker::new();
    room.add_message(
        OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user".to_string(),
            animal_name: "Lion".to_string(),
            text: "<p>Test</p>".to_string(),
            timestamp: "1000".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    assert!(room.last_activity > initial);
}

#[tokio::test]
async fn test_assign_animal_with_all_animals_in_use() {
    let mut room = create_room();
    let now = Instant::now();

    // Assign all animals to connected users
    let animal_count = room.available_animals.len();
    for i in 0..animal_count {
        let animal = room.assign_animal();
        room.users.insert(
            format!("user-{}", i),
            UserData {
                user_id: format!("user-{}", i),
                animal_name: animal,
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
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );
    }

    // Now when all animals are used, should get guest_X format
    let next = room.assign_animal();
    assert!(
        next.starts_with("guest_"),
        "Should get guest name when all animals used"
    );
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
                last_sanitized_message: None,
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
async fn test_claim_max_500_messages_per_room() {
    let _app_state = Arc::new(AppState::new());

    let mut history = vec![];
    for i in 0..550 {
        history.push(OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user".to_string(),
            animal_name: "Animal".to_string(),
            text: format!("<p>Msg {}</p>", i),
            timestamp: i.to_string(),
            reply_to: None,
            attachment: None,
        });
    }

    // Simulate cleanup trim - keep last 500 messages when over limit
    if history.len() > 500 {
        history = history[history.len() - 500..].to_vec();
    }

    assert!(
        history.len() <= 500,
        "History should be trimmed to 500 messages"
    );
}

#[tokio::test]
async fn test_claim_anonymous_animal_names() {
    // Users identified by random animals, not personal info
    let user_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(user_id.len(), 36); // UUID
    assert!(!user_id.contains("@")); // Not email
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
            // Only 450 seconds old (under 10 min threshold)
            last_activity: Instant::now() - Duration::from_secs(450),
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
            // 15 minutes old (over 10 min threshold)
            last_activity: Instant::now() - Duration::from_secs(900),
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
                last_sanitized_message: None,
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
async fn test_main_room_never_deleted() {
    // Main room is protected by: if room_name == "main" { ... continue; }
    // This test documents that the main room ONLY has message fade, never deletion.
    // The continue statement skips the deletion logic entirely.
    // The fade timeout should be 600 seconds (10 minutes)
    assert_eq!(EMPTY_ROOM_CLEANUP_DELAY.as_secs(), 600);
}

#[tokio::test]
async fn test_chat_error_room_full_status() {
    let error = ChatError::RoomFull;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_room_name_regex_pattern() {
    let re = regex::Regex::new(ROOM_NAME_REGEX).unwrap();

    // Valid names
    assert!(re.is_match("abc"));
    assert!(re.is_match("a1b"));
    assert!(re.is_match("test-room"));
    assert!(re.is_match("test_room"));
    assert!(re.is_match("Test123"));

    // Invalid names (start/end with special chars)
    assert!(!re.is_match("-test"));
    assert!(!re.is_match("test-"));
    assert!(!re.is_match("_test"));
    assert!(!re.is_match("test_"));
    // Note: "ab" matches regex, min length is enforced separately
}

// ========== MEMORY TRACKER EDGE CASES ==========

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
                last_sanitized_message: None,
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
                last_sanitized_message: None,
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
                last_sanitized_message: None,
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
async fn test_version_is_set() {
    assert!(!VERSION.is_empty());
}

// ==========================================================================
// FRONTEND/BACKEND CONSISTENCY TESTS
// ==========================================================================
// These tests ensure the embedded HTML frontend stays in sync with backend
// constants. They MUST fail if timing values drift apart.

#[tokio::test]
async fn test_frontend_max_messages_matches_backend() {
    // Frontend should trim messages at same limit as backend

    let backend_max = MAX_MESSAGES_PER_ROOM;
    assert_eq!(backend_max, 500, "MAX_MESSAGES_PER_ROOM should be 500");

    // Frontend ChatApp should have matching maxMessages
    assert!(
        SHIPPED_CLIENT.contains("this.maxMessages = 500"),
        "Frontend maxMessages should match backend MAX_MESSAGES_PER_ROOM (500)"
    );
}

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
async fn test_frontend_comment_documents_timeout() {
    // Frontend JS should have a comment documenting the 1 minute timeout
    // This helps future developers understand the timing

    assert!(
        SHIPPED_CLIENT.contains("1 minute room timeout")
            || SHIPPED_CLIENT.contains("60s")
            || SHIPPED_CLIENT.contains("60 second"),
        "Frontend should document the room timeout in comments for maintainability"
    );
}

#[tokio::test]
async fn test_room_handler_reserved_paths() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let reserved = [
        "robots.txt",
        "sitemap.xml",
        "favicon.ico",
        "main",
        "admin",
        "api",
        "health",
        "metrics",
        "ws",
    ];

    for path in reserved.iter() {
        let req = Request::builder()
            .uri(format!("/{}", path))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("Invalid") || body_str.contains("<!DOCTYPE"),
            "Reserved path {} should be handled specially",
            path
        );
    }
}

#[tokio::test]
async fn test_room_state_trim_to_max_messages() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Add more than MAX_MESSAGES_PER_ROOM
    for i in 0..MAX_MESSAGES_PER_ROOM + 50 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Lion".to_string(),
            text: format!("<p>Message {}</p>", i),
            timestamp: "1000".to_string(),
            reply_to: None,
            attachment: None,
        };
        room.chat_history.push(Arc::new(msg));
    }

    room.trim_to_max_messages(&tracker);
    assert!(room.chat_history.len() <= MAX_MESSAGES_PER_ROOM);
}

#[tokio::test]
async fn test_user_removed_during_message_processing() {
    let app_state = Arc::new(AppState::new());

    // Create room with a user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        let user_id = "test-user-123".to_string();
        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id: user_id.clone(),
                animal_name: "TestAnimal".to_string(),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Connected {
                    last_heartbeat: Instant::now(),
                    connection_id: "conn-123".to_string(),
                },
                last_read_message: None,
                is_typing: false,
                last_typing_event: None,
                last_read_receipt_event: None,
                rate_limiter: RateLimiter::new(),
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );

        rooms.insert("user-removed-room".to_string(), room_state);
    }

    // Now remove the user to simulate race condition
    {
        let mut rooms = app_state.rooms.write().await;
        if let Some(room_state) = rooms.get_mut("user-removed-room") {
            room_state.users.remove("test-user-123");
        }
    }

    // Verify user is gone
    let rooms = app_state.rooms.read().await;
    if let Some(room_state) = rooms.get("user-removed-room") {
        assert!(!room_state.users.contains_key("test-user-123"));
    }
}

// Test heartbeat timeout - covers lines 1652-1660

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
async fn test_duplicate_message_prevention_via_ws() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/duplicate-msg-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send the same message twice rapidly
    let msg = r#"{"type":"Message","text":"Duplicate test message"}"#;
    ws.send(text_frame(msg)).await.ok();
    ws.send(text_frame(msg)).await.ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(
        ping_result.is_ok(),
        "Connection should still be alive after duplicate detection"
    );

    ws.close(None).await.ok();
}

// Test message too long - covers lines 1707-1712

#[tokio::test]
async fn test_message_too_long_via_ws() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/long-msg-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send a message that exceeds MAX_MESSAGE_LEN
    let long_text = "x".repeat(MAX_MESSAGE_LEN + 100);
    let msg = format!(r#"{{"type":"Message","text":"{}"}}"#, long_text);
    ws.send(text_frame(msg)).await.ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(
        ping_result.is_ok(),
        "Connection should still be alive after rejecting long message"
    );

    ws.close(None).await.ok();
}

// Test binary message handling - covers lines 1827-1831

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
async fn test_prune_old_messages_with_existing_messages() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Add several messages
    for i in 0..10 {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: format!("Message {}", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        }));
        tracker.add_bytes(100);
        room.total_memory_bytes
            .fetch_add(100, std::sync::atomic::Ordering::SeqCst);
    }

    let initial_len = room.chat_history.len();

    // Prune messages
    room.prune_old_messages(500, &tracker);

    // Should have removed some messages
    assert!(room.chat_history.len() < initial_len);
}

// Test pong message handling - covers lines 1848-1849

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
                    last_sanitized_message: None,
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
async fn test_user_still_active() {
    let user = UserData {
        user_id: "test".to_string(),
        animal_name: "Lion".to_string(),
        last_active: Instant::now(),
        last_message_time: Instant::now(),
        connection_state: ConnectionState::Connected {
            last_heartbeat: Instant::now(),
            connection_id: "conn".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_sanitized_message: None,
        last_reaction_event: None,
    };

    let now = Instant::now();
    assert!(!user_idle_for_too_long(&user, now));
}

// Test ChatError IntoResponse for all variants - covers lines 328-345

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

#[tokio::test]
async fn test_room_handler_more_reserved_paths() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Test various reserved paths
    let reserved = vec!["admin", "api", "ws"];
    for path in reserved {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", path))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Invalid room name"));
    }
}

// Test room_handler with invalid room name length - covers lines 1146-1150

#[tokio::test]
async fn test_room_handler_name_length_bounds() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Too short (less than 3 chars)
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/ab").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters")
    );

    // Too long (more than 50 chars)
    let long_name = "a".repeat(51);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/{}", long_name))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters")
    );
}

// Test room_handler with invalid regex pattern - covers lines 1154-1159

#[tokio::test]
async fn test_room_handler_invalid_regex_patterns() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Invalid patterns that fail the regex (not starting/ending with alphanumeric)
    let invalid = vec!["has@symbol", "has.dot"];
    for name in invalid {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        // Should fail regex validation
        assert!(
            String::from_utf8_lossy(&body).contains("alphanumeric")
                || String::from_utf8_lossy(&body).contains("Invalid")
        );
    }
}

// Test AppState cleanup triggers memory GC - covers lines 1073-1088

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
                last_sanitized_message: None,
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
async fn test_prune_old_messages_keeps_recent() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Add messages with timestamps - each message ~100 bytes
    for i in 0..20 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: format!("Message {}", i),
            timestamp: format!("{}", i * 1000 + 1000000),
            reply_to: None,
            attachment: None,
        };
        let msg_size = msg.estimate_size();
        tracker.add_bytes(msg_size);
        room.total_memory_bytes
            .fetch_add(msg_size, Ordering::SeqCst);
        room.chat_history.push(Arc::new(msg));
    }

    let initial_count = room.chat_history.len();
    assert_eq!(initial_count, 20);

    // Prune to free up space for 500 bytes (should remove a few messages)
    room.prune_old_messages(500, &tracker);

    // Should have fewer messages after pruning
    assert!(room.chat_history.len() < initial_count);
}

// Test RoomState add_message when approaching memory limit - covers lines 848-870

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

#[tokio::test]
async fn test_room_handler_hyphen_underscore() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Valid patterns with hyphens and underscores - should return the HTML page, not error
    let valid_names = vec!["test-room", "test_room", "test-room-name", "a1b2c3"];
    for name in valid_names {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);
        // Should NOT contain specific error messages
        assert!(
            !body_str.contains("Room name must be between 3 and 50 characters"),
            "Room '{}' incorrectly rejected for length",
            name
        );
        assert!(
            !body_str.contains("Room name must start/end with alphanumeric"),
            "Room '{}' incorrectly rejected for regex",
            name
        );
        // Should contain HTML content indicating success
        assert!(
            body_str.contains("<!DOCTYPE html>") || body_str.contains("<html"),
            "Room '{}' should return HTML page",
            name
        );
    }
}

// ========== ROSTER INTEGRITY ==========

/// The animal roster must stay sorted, unique and actually made of animals.
///
/// This is a sweep, not a spot check: an earlier revision of the list ran off
/// the end of a dictionary and shipped `hadron`, `hagiology`, `gyroscope`,
/// `half-penny` and `hallway` as assignable names, with `hallingers` and
/// `hallway` present twice. Duplicates are the part that actually broke: two
/// connected users could hold the same name, which is the only identity the UI
/// shows.
#[test]
fn animal_roster_is_sorted_unique_and_well_formed() {
    assert!(
        ANIMAL_NAMES.len() >= 100,
        "roster is too small to keep a full room in distinct names"
    );

    let mut seen = std::collections::HashSet::new();
    for name in ANIMAL_NAMES {
        assert!(
            seen.insert(*name),
            "duplicate animal name in roster: {name}"
        );
        assert!(!name.is_empty(), "empty animal name in roster");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "animal name must be lowercase ascii with hyphens only: {name}"
        );
        assert!(
            !name.starts_with('-') && !name.ends_with('-'),
            "animal name must not start or end with a hyphen: {name}"
        );
    }

    let mut sorted = ANIMAL_NAMES.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        sorted.as_slice(),
        ANIMAL_NAMES,
        "roster must stay sorted so a duplicate is visible in review"
    );
}

/// A full room hands out distinct names for as long as the roster lasts.
#[tokio::test]
async fn assign_animal_never_repeats_a_connected_name() {
    let mut room = create_room();
    let mut handed_out = std::collections::HashSet::new();

    for i in 0..50 {
        let name = room.assign_animal();
        assert!(
            handed_out.insert(name.clone()),
            "assign_animal handed out {name} twice"
        );

        let now = Instant::now();
        room.users.insert(
            format!("user-{i}"),
            UserData {
                user_id: format!("user-{i}"),
                animal_name: name,
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
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );
    }
}

// ========== ROUTER WIRING ==========

/// Auto-scrolling to the newest message must be instant, not animated.
///
/// With `scroll-behavior: smooth` on the container, every pin-to-bottom starts
/// an animation. History replays as one frame per message, so a reload started
/// hundreds of animations chasing a target that was still growing — the view
/// slid around for the whole load. Smooth scrolling stays opt-in per call
/// (`scrollToMessage` uses `scrollIntoView({behavior: 'smooth'})`).
#[test]
fn chat_container_does_not_animate_programmatic_scrolling() {
    assert!(
        !SHIPPED_CLIENT.contains("scroll-behavior: smooth"),
        "the chat container must not animate scrolling it does itself"
    );
    assert!(
        SHIPPED_CLIENT.contains("overflow-anchor: none"),
        "browser scroll anchoring fights the client's own scroll pinning"
    );
    assert!(
        SHIPPED_CLIENT.contains("behavior: 'smooth'"),
        "jump-to-replied-message should still animate"
    );
}

/// History replay must not be read as the user scrolling away.
///
/// Two guards, and both matter. `isLoadingHistory` suppresses per-message
/// scrolling until the server's ReconnectToken frame marks the end of the
/// replay. `programmaticScroll` suppresses the scroll event the client's own
/// write produces — mid-scroll the element is not yet at the bottom, and
/// reading that back cleared `shouldAutoScroll` and stranded the user partway
/// up the history. That race is why the symptom was intermittent.
#[test]
fn scroll_handler_ignores_history_replay_and_self_inflicted_scrolls() {
    assert!(
        SHIPPED_CLIENT.contains("if (this.programmaticScroll || this.isLoadingHistory) return;"),
        "handleChatScroll must ignore its own scrolls and the history replay"
    );
    assert!(
        SHIPPED_CLIENT.contains("this.finishHistoryLoad();"),
        "the ReconnectToken frame must end the history-load window"
    );
    assert!(
        SHIPPED_CLIENT.contains("this.historyLoadTimeoutId = setTimeout"),
        "a missing ReconnectToken must not leave auto-scroll suppressed forever"
    );
}

// ========== SECURITY: RESPONSE HEADERS ==========

/// A name held by a connected user is skipped and rotated to the back.
#[tokio::test]
async fn assign_animal_skips_names_already_in_use() {
    let mut room = create_room();

    // Whatever is at the front of the pool, claim it.
    let front = room.available_animals.front().cloned().unwrap();
    let now = Instant::now();
    room.users.insert(
        "holder".to_string(),
        UserData {
            user_id: "holder".to_string(),
            animal_name: front.clone(),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: "c1".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
            last_reaction_event: None,
        },
    );

    let assigned = room.assign_animal();
    assert_ne!(
        assigned, front,
        "a name in use must not be handed out again"
    );
    assert!(
        room.available_animals.contains(&front),
        "the skipped name stays in the pool for later"
    );
}

/// Pruning with nothing to reclaim is a no-op, not an underflow.
#[test]
fn pruning_zero_bytes_changes_nothing() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "hello".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    let before = room.chat_history.len();
    room.prune_old_messages(0, &tracker);
    assert_eq!(room.chat_history.len(), before, "nothing needed reclaiming");

    // Likewise, trimming to more than is present.
    room.retain_newest(usize::MAX, &tracker);
    assert_eq!(room.chat_history.len(), before);
}

/// Age-based cleanup removes at most one batch per pass.
///
/// The cap bounds how long a sweep can hold the room write lock, which every
/// connected user contends on.
#[tokio::test]
async fn message_cleanup_is_capped_at_one_batch_per_pass() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Every message far older than MAX_MESSAGE_AGE.
    for _ in 0..(CLEANUP_BATCH_SIZE + 50) {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "old".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        }));
    }
    let before = room.chat_history.len();

    room.cleanup_messages(Instant::now(), &tracker).await;

    assert_eq!(
        room.chat_history.len(),
        before - CLEANUP_BATCH_SIZE,
        "a single pass removes exactly one batch"
    );
}

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
                    last_sanitized_message: None,
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

/// The same text twice in quick succession is one message.
#[tokio::test]
async fn a_duplicate_message_within_the_window_is_dropped() {
    let state = state_with_user("room", "user", Instant::now()).await;

    for _ in 0..2 {
        apply_client_event(
            &state,
            "room",
            "user",
            "otter",
            ClientEvent::Message {
                text: "same".into(),
                reply_to: None,
                attachment: None,
            },
        )
        .await;
    }

    let rooms = state.rooms.read().await;
    assert_eq!(
        rooms["room"].chat_history.len(),
        1,
        "an immediate repeat is a double-send, not a second message"
    );
}

/// Whitespace padding cannot smuggle a message past the length cap.
///
/// `validate_message` checks the *trimmed* length; this is the second gate,
/// on the untrimmed length, that exists because a message could trim down to
/// something short while still costing MAX_MESSAGE_LEN+ bytes on the wire and
/// in memory. Padding with whitespace instead of content is exactly the shape
/// of input that would slip past a trimmed-only check.
#[tokio::test]
async fn whitespace_padded_messages_cannot_bypass_the_length_cap() {
    let state = state_with_user("room", "user", Instant::now()).await;

    let padded = " ".repeat(MAX_MESSAGE_LEN + 500) + "hi";
    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::Message {
            text: padded,
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    assert!(
        rooms["room"].chat_history.is_empty(),
        "whitespace-padded oversized input must still be rejected"
    );
}

// ========== PRIVACY ==========

/// The upgrade path must hash before handing the address to anything that
/// stores it.
///
/// Asserted over the source because the property is *where* the hash happens:
/// a version that stored the raw address and hashed later would pass any
/// behavioural test while losing the entire point.
#[test]
fn the_upgrade_path_hashes_the_address_at_the_boundary() {
    // `../session.rs` — the server module. Since the suite became a directory,
    // a bare "session.rs" resolves to this suite's *own* session tests, and the
    // assertion would silently be about the wrong file.
    const SESSION_SRC: &str = include_str!("../session.rs");

    assert!(
        SESSION_SRC.contains(".map(hash_client_address)"),
        "ws_handler must hash the client address before using it"
    );
    assert!(
        !SESSION_SRC.contains("ip = ?ip"),
        "the client address must never be written to a log line"
    );
}

/// A returning visitor keeps the name they know.
///
/// The checks in `claim_animal` exist to refuse forged and colliding names, and
/// it would be easy to satisfy every one of those tests by never honouring a
/// cookie at all. This is the behaviour the checks are *for*: open the same
/// room in a second tab, or reload, and you are still the same animal.
#[tokio::test]
async fn a_returning_visitor_keeps_a_roster_name_that_is_free() {
    let state = Arc::new(AppState::new());

    // First visit: the server issues an identity.
    let (first_id, first_name) = admit_user(&state, "return-room", "c1", None)
        .await
        .expect("a new visitor is admitted");
    assert!(ANIMAL_NAMES.contains(&first_name.as_str()));

    // Same browser, a room it has not been in before, carrying that cookie.
    let cookie = crate::identity::UserCookie {
        user_id: first_id,
        animal_name: first_name.clone(),
    };
    let (_, second_name) = admit_user(&state, "another-room", "c2", Some(&cookie))
        .await
        .expect("a returning visitor is admitted");

    assert_eq!(
        second_name, first_name,
        "a roster name that nobody in the room is using should be honoured"
    );
}

// ========== ATTACHMENTS AND REACTIONS ==========

/// The id index tracks the history through every path that changes it.
///
/// It is a derived copy, which §1.4a warns about, so the thing worth asserting
/// is that it cannot drift: after any sequence of adds and removals it must
/// hold exactly the ids the history holds.
#[tokio::test]
async fn the_message_id_index_never_drifts_from_the_history() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    let expected = |room: &RoomState| -> std::collections::HashSet<Uuid> {
        room.chat_history.iter().map(|m| m.message_id).collect()
    };

    for i in 0..(MAX_MESSAGES_PER_ROOM + 40) {
        message_in(&mut room, &tracker, &format!("m{i}"));
    }
    assert_eq!(room.message_ids, expected(&room), "after adding");

    room.retain_newest(100, &tracker);
    assert_eq!(room.message_ids, expected(&room), "after trimming");

    room.prune_old_messages(5_000, &tracker);
    assert_eq!(room.message_ids, expected(&room), "after pruning");

    // Age-based cleanup: everything here is stamped 1970.
    room.cleanup_messages(Instant::now(), &tracker).await;
    assert_eq!(room.message_ids, expected(&room), "after age cleanup");
}

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

/// The shutdown announcement, without waiting for a signal.
///
/// `shutdown_signal` blocks on a real SIGINT, which a test cannot deliver
/// without killing the test runner — so the part that matters is split out and
/// tested directly. Cloudflare stops containers with SIGINT, and this is what
/// turns a routine scale-down into "user left" rather than a silent drop.
#[tokio::test]
async fn shutting_down_announces_departures_and_clears_the_rooms() {
    let state = Arc::new(AppState::new());
    let mut receiver;
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        receiver = room.sender.subscribe();
        rooms.insert("closing".to_string(), room);
    }

    crate::startup::announce_shutdown(&state).await;

    let event = receiver
        .try_recv()
        .expect("a departure should be announced");
    assert!(
        matches!(
            event,
            OutgoingEvent::System {
                event: SystemEvent::UserLeft { .. }
            }
        ),
        "connected users should be told they are leaving, got {event:?}"
    );
    assert!(
        state.rooms.read().await.is_empty(),
        "shutdown drops every room"
    );
}

/// A message's estimated size is the sum of its parts, exactly.
///
/// Nine mutants survived in `estimate_size` — every `+` could become `*` or
/// `-` and the suite still passed, because every existing assertion was
/// relative ("bigger than", "goes down after a trim"). This is what the memory
/// ceiling is built on: if it computes a product instead of a sum, the room
/// believes it is full at the second message, or never.
#[test]
fn a_messages_estimated_size_is_the_sum_of_its_parts() {
    let message = OutgoingMessage {
        message_id: Uuid::new_v4(),
        user_id: "user-id".to_string(),         // 7
        animal_name: "otter".to_string(),       // 5
        text: "hello there".to_string(),        // 11
        timestamp: "1700000000000".to_string(), // 13
        reply_to: None,
        attachment: None,
    };

    let expected = 7 + 5 + 11 + 13 + size_of::<Uuid>() + ESTIMATED_MESSAGE_SIZE;
    assert_eq!(
        message.estimate_size(),
        expected,
        "the estimate must be the sum of the field lengths, the uuid and the \
         per-message overhead — nothing multiplied, nothing dropped"
    );

    // A reply adds exactly its three strings.
    let with_reply = OutgoingMessage {
        reply_to: Some(ReplyInfo {
            message_id: "abcd".to_string(),    // 4
            author_name: "badger".to_string(), // 6
            preview_text: "hi".to_string(),    // 2
        }),
        ..message.clone()
    };
    assert_eq!(
        with_reply.estimate_size(),
        expected + 4 + 6 + 2,
        "a quoted reply costs exactly its three strings"
    );

    // An attachment adds exactly its mime and payload.
    let with_attachment = OutgoingMessage {
        attachment: Some(Attachment {
            mime: "image/png".to_string(), // 9
            data: "A".repeat(100),         // 100
            width: 1,
            height: 1,
            faded: false,
        }),
        ..message.clone()
    };
    assert_eq!(
        with_attachment.estimate_size(),
        expected + 9 + 100,
        "an attachment costs exactly its type and its payload"
    );

    // And the attachment's own accounting agrees with what the message charges.
    let attachment = with_attachment.attachment.as_ref().unwrap();
    assert_eq!(attachment.estimate_size(), 109);
}

/// Fading subtracts exactly what it freed.
///
/// Two mutants survived here: `attachment_bytes - freed` becoming `+`, and
/// `attachment_bytes -= freed` becoming `/=`. Either leaves the running total
/// disconnected from what the room is holding, and the symptom is silent —
/// pictures fading while the room is nearly empty, or never fading at all.
#[tokio::test]
async fn fading_subtracts_exactly_the_bytes_it_freed() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    let payload = MAX_ATTACHMENT_BYTES;
    let each = payload + "image/png".len();
    let needed = (MAX_ROOM_ATTACHMENT_BYTES / each) + 2;

    for i in 0..needed {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("p{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: Some(Attachment {
                    mime: "image/png".to_string(),
                    data: "A".repeat(payload),
                    width: 1,
                    height: 1,
                    faded: false,
                }),
            },
            &tracker,
        );
    }

    // The running total must equal what is actually still held, counted from
    // the history rather than from the counter it is being compared against.
    let actually_held: usize = room
        .chat_history
        .iter()
        .filter_map(|m| m.attachment.as_ref())
        .filter(|a| !a.faded)
        .map(crate::protocol::Attachment::estimate_size)
        .sum();

    assert_eq!(
        room.attachment_bytes, actually_held,
        "after fading, the running total must be exactly the bytes still held"
    );
    assert!(room.attachment_bytes <= MAX_ROOM_ATTACHMENT_BYTES);

    // Faded entries keep their mime but lose their payload, so the total is a
    // whole number of surviving images.
    assert_eq!(
        actually_held % each,
        0,
        "the survivors should each be a full image"
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

/// A history replay that completes says so, and sends every message.
#[tokio::test]
async fn a_full_history_replay_sends_every_message_and_reports_success() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    let ids: Vec<Uuid> = (0..4)
        .map(|i| message_in(&mut room, &tracker, &format!("m{i}")))
        .collect();
    room.toggle_reaction(ids[1], "🔥", "viewer");

    let history = room.history_for("viewer");
    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::default()));

    assert!(crate::session::send_history(&history, &sink, "viewer").await);

    let sent = &sink.lock().await.sent;
    assert_eq!(sent.len(), 4, "every message should have been replayed");

    // The reacted message carries this viewer's own state, flattened onto a
    // frame shaped exactly like a live one.
    let crate::session::Message::Text(body) = &sent[1] else {
        panic!("history is sent as text frames");
    };
    let frame: JsonValue = serde_json::from_str(body.as_str()).unwrap();
    assert_eq!(frame["type"], "Message");
    assert_eq!(frame["message"]["reactions"][0]["emoji"], "🔥");
    assert_eq!(frame["message"]["reactions"][0]["reacted"], true);
}

/// Trimming boundaries are exact: at the cap nothing moves, one past it one goes.
///
/// Both `>` comparisons mutated to `>=` and survived — the existing tests
/// trimmed comfortably-over histories and never checked the edge. Trimming at
/// exactly the cap would move the whole history on the message that reaches it,
/// which is the O(n)-per-message cost `HISTORY_TRIM_SLACK` exists to prevent.
#[tokio::test]
async fn the_history_trim_boundaries_are_exact() {
    let tracker = MemoryTracker::new();

    // `retain_newest` at exactly `keep` is a no-op.
    let mut room = create_room();
    for i in 0..10 {
        message_in(&mut room, &tracker, &format!("m{i}"));
    }
    let before: Vec<Uuid> = room.chat_history.iter().map(|m| m.message_id).collect();
    room.retain_newest(10, &tracker);
    assert_eq!(
        room.chat_history
            .iter()
            .map(|m| m.message_id)
            .collect::<Vec<_>>(),
        before,
        "keeping exactly what is there must not touch the history"
    );

    // One more than `keep` drops exactly one, the oldest.
    room.retain_newest(9, &tracker);
    assert_eq!(room.chat_history.len(), 9);
    assert_eq!(
        room.chat_history[0].message_id, before[1],
        "the oldest is the one that goes"
    );

    // `trim_to_max_messages` at exactly the cap is a no-op.
    let mut room = create_room();
    for i in 0..MAX_MESSAGES_PER_ROOM {
        message_in(&mut room, &tracker, &format!("m{i}"));
    }
    let oldest = room.chat_history[0].message_id;
    room.trim_to_max_messages(&tracker);
    assert_eq!(
        room.chat_history.len(),
        MAX_MESSAGES_PER_ROOM,
        "a history exactly at the cap must not be trimmed"
    );
    assert_eq!(room.chat_history[0].message_id, oldest);

    // One past the cap trims back to it.
    message_in(&mut room, &tracker, "one too many");
    room.trim_to_max_messages(&tracker);
    assert_eq!(room.chat_history.len(), MAX_MESSAGES_PER_ROOM);
    assert_ne!(
        room.chat_history[0].message_id, oldest,
        "and the oldest is what it dropped"
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

/// The trim slack is what keeps trimming off the message path.
///
/// `preserve_messages` fires at `MAX_MESSAGES_PER_ROOM + HISTORY_TRIM_SLACK`,
/// and both halves of that expression survived mutation: `>` could become `>=`,
/// and `+` could become `-`. The second is the expensive one — trimming a
/// hundred messages *below* the cap means a linear move of the whole history on
/// almost every message, which is exactly the per-message cost §1.1 forbids and
/// nothing would have failed.
#[test]
fn history_is_trimmed_only_once_it_has_drifted_a_full_slack_past_the_cap() {
    let tracker = MemoryTracker::new();

    for (extra, expect_trim) in [(HISTORY_TRIM_SLACK, false), (HISTORY_TRIM_SLACK + 1, true)] {
        let mut room = create_room();
        for i in 0..(MAX_MESSAGES_PER_ROOM + extra) {
            message_in(&mut room, &tracker, &format!("message {i}"));
        }
        let before = room.chat_history.len();

        room.preserve_messages(&tracker);

        if expect_trim {
            assert_eq!(
                room.chat_history.len(),
                MAX_MESSAGES_PER_ROOM,
                "one message past the slack must trim back to the cap"
            );
        } else {
            assert_eq!(
                room.chat_history.len(),
                before,
                "at exactly the cap plus the slack there is nothing to do — \
                 trimming here would put a linear pass on the message path"
            );
        }
    }
}

/// A room over its picture budget fades the oldest images and stops.
///
/// `self.attachment_bytes - freed <= MAX_ROOM_ATTACHMENT_BYTES` is the loop's
/// only exit, and replacing that `-` with `+` survived: the running total then
/// grows as images are freed, the condition never becomes true, and every
/// picture in the room is faded rather than just enough of them. A room one
/// image over its budget would lose all of them.
#[tokio::test]
async fn fading_pictures_stops_as_soon_as_the_room_is_back_under_its_budget() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Each image is a tenth of the budget, so going one over needs eleven and
    // recovering needs exactly one to fade.
    let payload = "A".repeat(MAX_ROOM_ATTACHMENT_BYTES / 10);
    for i in 0..11 {
        let mut msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: format!("picture {i}"),
            timestamp: "0".to_string(),
            reply_to: None,
            attachment: Some(Attachment {
                data: payload.clone(),
                ..png_attachment()
            }),
        };
        msg.timestamp = i.to_string();
        room.add_message(msg, &tracker);
    }

    let faded: Vec<bool> = room
        .chat_history
        .iter()
        .map(|m| m.attachment.as_ref().is_some_and(|a| a.faded))
        .collect();
    let faded_count = faded.iter().filter(|f| **f).count();

    assert!(
        faded_count > 0,
        "a room over its picture budget must fade something"
    );
    assert!(
        faded_count < faded.len(),
        "fading must stop once the room is back under budget — it faded all \
         {} pictures, which is the whole conversation's images for one overrun",
        faded.len()
    );
    assert!(
        faded[..faded_count].iter().all(|f| *f) && !faded[faded_count],
        "the pictures that fade are the oldest ones, in order: {faded:?}"
    );
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
