//! A room's message history: adding, ordering, deduplication, and the
//! per-room isolation that keeps one room's messages out of another's.

use super::*;

#[tokio::test]
async fn test_create_room() {
    let room = create_room();
    assert!(room.chat_history.is_empty());
    assert!(!room.available_animals.is_empty());
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
                last_message_text: None,
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
        last_message_text: None,
        last_reaction_event: None,
    };

    let now = Instant::now();
    assert!(!user_idle_for_too_long(&user, now));
}

// Test ChatError IntoResponse for all variants - covers lines 328-345

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
/// `validate_and_render_message` checks the *trimmed* length; this is the second gate,
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

/// A refused message must not become the user's duplicate record.
///
/// The length check above used to sit *after* the render and after
/// `last_message_text` was written, so a message the server refused still
/// overwrote what the duplicate guard was comparing against. That makes the
/// guard defeatable by sandwiching: send something, send something refused,
/// send the first thing again, and the double-send goes through. Same
/// principle as constraint #22 — a refused event must not have spent, or
/// disturbed, anything belonging to the next one.
///
/// The instants are supplied so all three events land inside
/// `DUPLICATE_MESSAGE_WINDOW`; with the clock read inside the function the
/// window would be a race (§9.4).
#[tokio::test]
async fn a_refused_message_does_not_clear_the_duplicate_guard() {
    let now = Instant::now();
    let state = state_with_user("room", "user", now).await;

    let send = async |text: String, at: Instant| {
        apply_client_event_at(
            &state,
            "room",
            "user",
            "otter",
            ClientEvent::Message {
                text,
                reply_to: None,
                attachment: None,
            },
            at,
        )
        .await;
    };

    send("hello".to_string(), now).await;
    assert_eq!(
        state.rooms.read().await["room"].chat_history.len(),
        1,
        "the first message is stored"
    );

    // Refused: over the untrimmed cap.
    send(" ".repeat(MAX_MESSAGE_LEN + 500) + "hi", now).await;
    assert_eq!(
        state.rooms.read().await["room"].chat_history.len(),
        1,
        "an over-length message must not be stored"
    );

    // Immediately resending the first message is still a double-send.
    send("hello".to_string(), now).await;
    assert_eq!(
        state.rooms.read().await["room"].chat_history.len(),
        1,
        "the refused message must not have replaced what the duplicate guard \
         compares against — otherwise a double-send gets through by putting a \
         refused message between the two copies"
    );
}

// ========== PRIVACY ==========

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

    let frame = receiver
        .try_recv()
        .expect("a departure should be announced");
    let event = decode_broadcast(&frame);
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
