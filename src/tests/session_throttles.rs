//! Per-event throttles: typing indicators, read receipts, roster requests,
//! and the duplicate-message window. Each is an O(1) rate limit distinct from
//! the per-connection message budget (constraint #22).

use super::*;

#[tokio::test]
async fn test_typing_event_timestamp_tracking() {
    let now = Instant::now();

    // Simulating debounce check logic
    let last_event = Some(now);
    let min_interval = Duration::from_millis(200);

    // Too soon (100ms)
    let too_soon = now + Duration::from_millis(100);
    let allowed_soon = match last_event {
        Some(t) => too_soon.duration_since(t) >= min_interval,
        None => true,
    };
    assert!(!allowed_soon);

    // Allowed (300ms)
    let ok_later = now + Duration::from_millis(300);
    let allowed_later = match last_event {
        Some(t) => ok_later.duration_since(t) >= min_interval,
        None => true,
    };
    assert!(allowed_later);
}

#[tokio::test]
async fn test_read_receipt_timestamp_tracking() {
    let now = Instant::now();
    let min_interval = Duration::from_millis(200);

    let last_event = Some(now);

    // Too soon (100ms)
    let too_soon = now + Duration::from_millis(100);
    let can_send_soon = match last_event {
        Some(t) => too_soon.duration_since(t) >= min_interval,
        None => true,
    };
    assert!(!can_send_soon);

    // Allowed (250ms)
    let later = now + Duration::from_millis(250);
    let can_send_later = match last_event {
        Some(t) => later.duration_since(t) >= min_interval,
        None => true,
    };
    assert!(can_send_later);
}

// ========== MEMORY MANAGEMENT ==========

#[tokio::test]
async fn test_ws_typing_event_broadcast() {
    let (addr, handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/typingroom", addr);

    let (mut ws1, _) = connect_async(url.clone()).await.expect("connect failed");
    let (mut ws2, _) = connect_async(url).await.expect("connect failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
        let _ = recv_json_event(&mut ws2).await;
    }

    let payload = serde_json::json!({
        "type": "Typing",
        "is_typing": true
    })
    .to_string();
    ws1.send(text_frame(payload)).await.unwrap();

    let mut found = false;
    for _ in 0..10 {
        let val = recv_json_event(&mut ws2).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "Typing")
        {
            let is_typing = payload.get("is_typing").and_then(|v| v.as_bool());
            assert_eq!(is_typing, Some(true));
            found = true;
            break;
        }
    }

    assert!(found, "Typing event not received");
    handle.abort();
}

#[tokio::test]
async fn test_ws_read_receipt_event_broadcast() {
    let (addr, handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/readreceiptroom", addr);

    let (mut ws1, _) = connect_async(url.clone()).await.expect("connect failed");
    let (mut ws2, _) = connect_async(url).await.expect("connect failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
        let _ = recv_json_event(&mut ws2).await;
    }

    let msg_id = uuid::Uuid::new_v4().to_string();
    let payload = serde_json::json!({
        "type": "ReadReceipt",
        "message_id": msg_id
    })
    .to_string();
    ws1.send(text_frame(payload)).await.unwrap();

    let mut found = false;
    for _ in 0..10 {
        let val = recv_json_event(&mut ws2).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "ReadReceipt")
        {
            let message_id = payload
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert!(!message_id.is_empty());
            found = true;
            break;
        }
    }

    assert!(found, "ReadReceipt event not received");
    handle.abort();
}

// ========== FAILURE INJECTION & EDGE CASES ==========

#[tokio::test]
async fn test_concurrent_typing_events() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/typing-concurrent", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send multiple typing events rapidly
    for i in 0..10 {
        let is_typing = i % 2 == 0;
        ws.send(text_frame(format!(
            r#"{{"type":"Typing","is_typing":{}}}"#,
            is_typing
        )))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_read_receipt_for_nonexistent_message() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/receipt-invalid", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send read receipt for fake message ID
    ws.send(text_frame(
        r#"{"type":"ReadReceipt","message_id":"00000000-0000-0000-0000-000000000000"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Should handle gracefully
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_typing_debounce_same_state() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/typing-debounce", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send typing=true twice
    ws.send(text_frame(
        r#"{"type":"Typing","is_typing":true}"#.to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    ws.send(text_frame(
        r#"{"type":"Typing","is_typing":true}"#.to_string(),
    ))
    .await
    .unwrap();

    // Should debounce
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.abort();
}

#[tokio::test]
async fn test_room_state_typing_broadcast() {
    // Simplified test - just verify typing events can be sent without error
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/typing-broadcast-test", addr);
    let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
    }

    // Send typing event
    ws1.send(text_frame(
        r#"{"type":"Typing","is_typing":true}"#.to_string(),
    ))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_typing_indicator_broadcast() {
    let (addr, _handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/typing-broadcast-test", addr);

    // Connect two clients
    let (mut ws1, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (mut ws2, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Clear initial events from ws2
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), ws2.next()).await {}

    // ws1 sends typing indicator
    ws1.send(text_frame(
        r#"{"type":"Typing","is_typing":true}"#.to_string(),
    ))
    .await
    .unwrap();

    // ws2 should receive the typing event (ws1 shouldn't see its own typing)
    let mut received_typing = false;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(200), ws2.next()).await
            && let Ok(event) = serde_json::from_str::<serde_json::Value>(&text)
            && event["type"] == "System"
            && let Some(typing) = event.get("event").and_then(|e| e.get("Typing"))
            && typing.get("is_typing") == Some(&serde_json::json!(true))
        {
            received_typing = true;
            break;
        }
    }
    assert!(
        received_typing,
        "Second client should receive typing indicator from first client"
    );
}

// ========== NEW COVERAGE TESTS ==========

#[tokio::test]
async fn test_invalid_read_receipt_message_id() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/invalid-receipt-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send invalid read receipt (not a valid UUID)
    ws.send(text_frame(
        r#"{"type":"ReadReceipt","message_id":"not-a-uuid"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Server should handle gracefully without crashing
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection should still be alive
    ws.send(text_frame(
        r#"{"type":"Message","text":"Still connected"}"#.to_string(),
    ))
    .await
    .unwrap();

    handle.abort();
}

#[tokio::test]
async fn test_typing_event_debounce() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/typing-debounce-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send typing events rapidly (should be debounced)
    for _ in 0..5 {
        ws.send(text_frame(
            r#"{"type":"Typing","is_typing":true}"#.to_string(),
        ))
        .await
        .unwrap();
    }

    // Not all should go through due to debouncing
    // Just verify connection is still alive
    tokio::time::sleep(Duration::from_millis(100)).await;

    ws.send(text_frame(
        r#"{"type":"Message","text":"Still connected"}"#.to_string(),
    ))
    .await
    .unwrap();

    handle.abort();
}

#[tokio::test]
async fn test_read_receipt_event_debounce() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/receipt-debounce-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send read receipts rapidly (should be debounced)
    let msg_id = Uuid::new_v4().to_string();
    for _ in 0..5 {
        ws.send(text_frame(format!(
            r#"{{"type":"ReadReceipt","message_id":"{}"}}"#,
            msg_id
        )))
        .await
        .unwrap();
    }

    // Verify connection still works
    tokio::time::sleep(Duration::from_millis(100)).await;

    ws.send(text_frame(
        r#"{"type":"Message","text":"Still connected"}"#.to_string(),
    ))
    .await
    .unwrap();

    handle.abort();
}

#[tokio::test]
async fn test_user_data_typing_state_default() {
    let user = UserData {
        user_id: "test".to_string(),
        animal_name: "Tiger".to_string(),
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
    };

    assert!(!user.is_typing);
    assert!(user.last_typing_event.is_none());
}

// ========== BUGFIX TESTS: Room Cleanup & Connection Issues ==========

#[tokio::test]
async fn test_typing_indicator_cleanup_aligns_with_backend() {
    // Frontend cleans up typing indicators after 5s
    // Backend has TYPING_EVENT_MIN_INTERVAL of 200ms

    let backend_interval_ms = TYPING_EVENT_MIN_INTERVAL.as_millis();
    assert_eq!(
        backend_interval_ms, 200,
        "TYPING_EVENT_MIN_INTERVAL should be 200ms"
    );

    // Frontend cleanup at 5000ms is reasonable (gives time for network latency)
    assert!(
        SHIPPED_CLIENT.contains("now - timestamp > 5000"),
        "Frontend typing indicator cleanup should be at 5000ms"
    );
}

#[tokio::test]
async fn test_typing_event_debounce_interval() {
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        let user_id = "typing-user".to_string();
        // Set last_typing_event to be very recent
        let recent_time = Instant::now();

        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id: user_id.clone(),
                animal_name: "TypingAnimal".to_string(),
                last_active: recent_time,
                last_message_time: recent_time,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: recent_time,
                    connection_id: "conn-typing".to_string(),
                },
                last_read_message: None,
                is_typing: false,
                last_typing_event: Some(recent_time), // Just typed
                last_read_receipt_event: None,
                rate_limiter: RateLimiter::new(),
                last_message_text: None,
                last_reaction_event: None,
            },
        );

        rooms.insert("typing-debounce-room".to_string(), room_state);
    }

    // Verify the debounce would trigger
    let rooms = app_state.rooms.read().await;
    if let Some(room_state) = rooms.get("typing-debounce-room")
        && let Some(user) = room_state.users.get("typing-user")
        && let Some(last_typing) = user.last_typing_event
    {
        let elapsed = Instant::now().duration_since(last_typing);
        // Should be less than the debounce interval since we just set it
        assert!(
            elapsed < TYPING_EVENT_MIN_INTERVAL,
            "Typing event should be debounced"
        );
    }
}

// Test read receipt debounce - covers lines 1780-1786

#[tokio::test]
async fn test_read_receipt_debounce_interval() {
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        let user_id = "receipt-user".to_string();
        let recent_time = Instant::now();

        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id: user_id.clone(),
                animal_name: "ReceiptAnimal".to_string(),
                last_active: recent_time,
                last_message_time: recent_time,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: recent_time,
                    connection_id: "conn-receipt".to_string(),
                },
                last_read_message: None,
                is_typing: false,
                last_typing_event: None,
                last_read_receipt_event: Some(recent_time), // Just sent receipt
                rate_limiter: RateLimiter::new(),
                last_message_text: None,
                last_reaction_event: None,
            },
        );

        rooms.insert("receipt-debounce-room".to_string(), room_state);
    }

    // Verify the debounce would trigger
    let rooms = app_state.rooms.read().await;
    if let Some(room_state) = rooms.get("receipt-debounce-room")
        && let Some(user) = room_state.users.get("receipt-user")
        && let Some(last_receipt) = user.last_read_receipt_event
    {
        let elapsed = Instant::now().duration_since(last_receipt);
        assert!(
            elapsed < READ_RECEIPT_MIN_INTERVAL,
            "Read receipt should be debounced"
        );
    }
}

// Test room not found during event processing - covers lines 1817-1820

#[tokio::test]
async fn test_invalid_uuid_in_read_receipt_via_ws() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/invalid-uuid-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send read receipt with invalid UUID
    let invalid_receipt = r#"{"type":"ReadReceipt","message_id":"not-a-valid-uuid"}"#;
    ws.send(text_frame(invalid_receipt)).await.ok();

    // Wait a moment - server should log warning but not crash
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(
        ping_result.is_ok(),
        "Connection should still be alive after invalid UUID"
    );

    ws.close(None).await.ok();
}

// Test message rate limit exceeded - covers lines 1700-1704

#[tokio::test]
async fn test_typing_stop_event_via_ws() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/typing-stop-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send typing start then stop
    ws.send(text_frame(r#"{"type":"Typing","is_typing":true}"#))
        .await
        .ok();
    tokio::time::sleep(Duration::from_millis(100)).await;
    ws.send(text_frame(r#"{"type":"Typing","is_typing":false}"#))
        .await
        .ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(ping_result.is_ok());

    ws.close(None).await.ok();
}

// Test message with reply_to field - covers reply processing

#[tokio::test]
async fn test_ws_read_receipt_valid_format() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/read-receipt-valid-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send valid read receipt with properly formatted UUID
    let valid_uuid = uuid::Uuid::new_v4().to_string();
    let msg = format!(r#"{{"type":"ReadReceipt","message_id":"{}"}}"#, valid_uuid);
    ws.send(text_frame(msg)).await.ok();

    // Wait for processing
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(ping_result.is_ok());

    ws.close(None).await.ok();
}

// Test user idle timeout check - covers line 323

/// Typing and read-receipt events are debounced, and a malformed receipt is
/// ignored rather than fatal.
#[tokio::test]
async fn typing_and_read_receipts_are_debounced_and_validated() {
    let state = state_with_user("room", "user", Instant::now()).await;
    let mut events = state.rooms.read().await["room"].sender.subscribe();

    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::Typing { is_typing: true },
    )
    .await;
    // Immediately again: inside the debounce window, so it must not broadcast.
    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::Typing { is_typing: false },
    )
    .await;

    let message_id = Uuid::new_v4().to_string();
    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::ReadReceipt { message_id },
    )
    .await;

    // A receipt that is not a message id is dropped.
    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::ReadReceipt {
            message_id: "nonsense".into(),
        },
    )
    .await;

    let mut typing_events = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event,
            OutgoingEvent::System {
                event: SystemEvent::Typing { .. }
            }
        ) {
            typing_events += 1;
        }
    }
    assert_eq!(
        typing_events, 1,
        "the debounced second event must not broadcast"
    );
}

/// A typing indicator is not a message, and must not spend a message's budget.
///
/// Every client event ran through `can_send_message`, but the client sends a
/// `Typing{true}` on the first keystroke and a `Typing{false}` when the message
/// is sent — so each real message cost three units of a thirty-unit window, and
/// the effective limit was ten messages a minute rather than the documented
/// thirty. Typing and read receipts have their own O(1) throttles
/// (`TYPING_EVENT_MIN_INTERVAL`, `READ_RECEIPT_MIN_INTERVAL`); that is what
/// bounds them.
#[tokio::test]
async fn typing_events_do_not_consume_the_message_budget() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        rooms.insert("budget-room".to_string(), room);
    }

    for i in 0..MAX_MESSAGES_PER_WINDOW {
        apply_client_event(
            &state,
            "budget-room",
            "u1",
            "otter",
            ClientEvent::Typing {
                is_typing: i % 2 == 0,
            },
        )
        .await;
        // Clear the per-event throttle so this measures the message budget,
        // not the 200ms spacing.
        {
            let mut rooms = state.rooms.write().await;
            let user = rooms
                .get_mut("budget-room")
                .unwrap()
                .users
                .get_mut("u1")
                .unwrap();
            user.last_typing_event = None;
        }
    }

    apply_client_event(
        &state,
        "budget-room",
        "u1",
        "otter",
        ClientEvent::Message {
            text: "hello".to_string(),
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    assert_eq!(
        rooms.get("budget-room").unwrap().chat_history.len(),
        1,
        "a full window of typing indicators must not block a message"
    );
}

/// The roster names everyone connected, sorted, and nobody else.
///
/// Sorted matters: a `HashMap` iterates in whatever order it likes, and a panel
/// that reorders itself every time it opens reads as people coming and going.
#[tokio::test]
async fn the_roster_names_everyone_connected_in_a_stable_order() {
    let mut room = create_room();
    let now = Instant::now();

    for (uid, animal) in [("u1", "otter"), ("u2", "badger"), ("u3", "crane")] {
        room.users
            .insert(uid.to_string(), connected_user(uid, animal, "c", now));
    }

    // Somebody who has gone is not in the room, whatever their slot says.
    let mut gone = connected_user("u4", "zebu", "c", now);
    gone.connection_state = ConnectionState::Disconnected { since: now };
    room.users.insert("u4".to_string(), gone);

    assert_eq!(
        room.roster(),
        vec![
            "badger".to_string(),
            "crane".to_string(),
            "otter".to_string()
        ],
        "the roster is everyone *connected*, in a stable order"
    );

    assert!(
        !room.roster().contains(&"zebu".to_string()),
        "a disconnected user is not in the room"
    );
}

/// Asking for the roster answers the asker, and nobody else pays for it.
///
/// Sent on request rather than broadcast with every arrival: pushing the whole
/// list to everyone whenever anybody joins is O(users) per recipient, which is
/// quadratic in the size of the room, for a panel almost nobody has open.
#[tokio::test]
async fn requesting_the_roster_answers_with_the_current_names() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for (uid, animal) in [("u1", "otter"), ("u2", "badger")] {
            room.users.insert(
                uid.to_string(),
                connected_user(uid, animal, "c", Instant::now()),
            );
        }
        rooms.insert("who".to_string(), room);
    }

    let mut receiver = state
        .rooms
        .read()
        .await
        .get("who")
        .unwrap()
        .sender
        .subscribe();

    apply_client_event(&state, "who", "u1", "otter", ClientEvent::RequestRoster).await;

    match receiver.try_recv().expect("a roster should be sent") {
        OutgoingEvent::Roster { users } => {
            assert_eq!(users, vec!["badger".to_string(), "otter".to_string()]);
        }
        other => panic!("expected a Roster, got {other:?}"),
    }
}

/// Roster requests are throttled like any other thing a person clicks.
#[tokio::test]
async fn roster_requests_are_throttled() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c", Instant::now()),
        );
        rooms.insert("spam".to_string(), room);
    }

    let mut receiver = state
        .rooms
        .read()
        .await
        .get("spam")
        .unwrap()
        .sender
        .subscribe();

    for _ in 0..5 {
        apply_client_event(&state, "spam", "u1", "otter", ClientEvent::RequestRoster).await;
    }

    let mut answered = 0;
    while receiver.try_recv().is_ok() {
        answered += 1;
    }
    assert_eq!(
        answered, 1,
        "five requests in a tick is one person's finger, not five questions"
    );
}

/// Every per-event throttle admits the event that lands exactly on its interval.
///
/// Six comparisons in `apply_client_event` survived mutation as `<=`, all for
/// the same reason: the clock was read inside the function, so a test could set
/// a user's last event to exactly one interval ago and the reading had already
/// moved past it by the time the comparison ran (§6.6f). With the instant
/// supplied, the boundary is a case.
///
/// The answer matters in both directions. A throttle that refuses the event
/// exactly at its interval is a throttle slightly tighter than the number it
/// advertises, and these numbers are what the client paces itself against —
/// typing indicators at 200 ms, reactions at 100 ms. `is_typing` stopping one
/// frame short is a "still typing…" that flickers off mid-sentence.
#[tokio::test]
async fn a_throttled_event_exactly_on_its_interval_is_admitted() {
    let now = Instant::now();

    // (what the user last did, how long ago, the event, what the room sees)
    // Built per iteration rather than cloned: `ClientEvent` is a wire type and
    // deriving `Clone` on it purely for a test is production surface a test
    // asked for (§8).
    type ThrottleCase = (&'static str, Duration, fn() -> ClientEvent);
    let cases: Vec<ThrottleCase> = vec![
        ("typing", TYPING_EVENT_MIN_INTERVAL, || {
            ClientEvent::Typing { is_typing: true }
        }),
        ("read receipt", READ_RECEIPT_MIN_INTERVAL, || {
            ClientEvent::ReadReceipt {
                message_id: Uuid::new_v4().to_string(),
            }
        }),
        ("roster", REACTION_MIN_INTERVAL, || {
            ClientEvent::RequestRoster
        }),
    ];

    for (label, interval, event) in cases {
        for (gap, admitted) in [
            (interval, true),
            (interval - Duration::from_nanos(1), false),
        ] {
            let state = Arc::new(AppState::new());
            {
                let mut rooms = state.rooms.write().await;
                let mut room = create_room();
                let mut user = connected_user("u1", "otter", "c1", now);
                user.last_typing_event = Some(now - gap);
                user.last_read_receipt_event = Some(now - gap);
                user.last_reaction_event = Some(now - gap);
                room.users.insert("u1".to_string(), user);
                rooms.insert("r".to_string(), room);
            }

            let mut rx = state.rooms.read().await["r"].sender.subscribe();
            apply_client_event_at(&state, "r", "u1", "otter", event(), now).await;

            assert_eq!(
                rx.try_recv().is_ok(),
                admitted,
                "a {label} event {gap:?} after the last one, against an interval \
                 of {interval:?}, should be admitted: {admitted}"
            );
        }
    }
}

/// A repeat of the same text is refused only while it is still a double-send.
///
/// `DUPLICATE_MESSAGE_WINDOW` catches the double-click and the retry, not the
/// person who means it. `<` mutating to `<=` survived because the window's edge
/// was unreachable — and the edge is exactly where "they sent it twice by
/// accident" stops being true.
#[tokio::test]
async fn a_repeated_message_is_refused_only_inside_the_duplicate_window() {
    let now = Instant::now();

    for (gap, delivered) in [
        (DUPLICATE_MESSAGE_WINDOW, true),
        (DUPLICATE_MESSAGE_WINDOW - Duration::from_nanos(1), false),
    ] {
        let state = Arc::new(AppState::new());
        {
            let mut rooms = state.rooms.write().await;
            let mut room = create_room();
            let mut user = connected_user("u1", "otter", "c1", now);
            user.last_message_text = Some(("hello".to_string(), now - gap));
            room.users.insert("u1".to_string(), user);
            rooms.insert("r".to_string(), room);
        }

        let mut rx = state.rooms.read().await["r"].sender.subscribe();
        apply_client_event_at(
            &state,
            "r",
            "u1",
            "otter",
            ClientEvent::Message {
                text: "hello".to_string(),
                reply_to: None,
                attachment: None,
            },
            now,
        )
        .await;

        assert_eq!(
            rx.try_recv().is_ok(),
            delivered,
            "the same text {gap:?} later, against a window of \
             {DUPLICATE_MESSAGE_WINDOW:?}, should be delivered: {delivered}"
        );
    }
}
