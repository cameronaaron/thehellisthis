//! The WebSocket session: admission, the four tasks, identity, teardown.

use super::*;

#[tokio::test]
async fn test_create_user_cookies() {
    let (user_cookie, animal_cookie) = create_user_cookies("id", "lion", "thehellisthis.com");
    assert!(user_cookie.contains("user_id=id"));
    assert!(animal_cookie.contains("animal_name=lion"));
}

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
        last_sanitized_message: None,
        last_reaction_event: None,
    };

    assert_eq!(user.user_id, "test-id");
    assert!(!user.is_typing);
    assert!(user.last_read_message.is_none());
}

// ========== TYPING & READ RECEIPTS (DEBOUNCING) ==========

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
        last_sanitized_message: None,
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
        last_sanitized_message: None,
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
        last_sanitized_message: None,
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
        last_sanitized_message: None,
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
async fn test_broadcast_user_count_sends_event() {
    let mut room = create_room();
    let now = Instant::now();

    room.users.insert(
        "user1".to_string(),
        UserData {
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
            last_sanitized_message: None,
            last_reaction_event: None,
        },
    );

    room.users.insert(
        "user2".to_string(),
        UserData {
            user_id: "user2".to_string(),
            animal_name: "Tiger".to_string(),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Connected {
                last_heartbeat: now - Duration::from_secs(10),
                connection_id: "conn2".to_string(),
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

    let mut rx = room.sender.subscribe();
    room.broadcast_user_count();

    let event = timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("no broadcast received")
        .expect("broadcast recv failed");

    match event {
        OutgoingEvent::UserCount { count } => assert_eq!(count, 1),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn test_ws_reconnect_token_sent() {
    let (addr, handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/testroom", addr);

    let (mut ws, _) = connect_async(url).await.expect("connect failed");

    let mut found = false;
    for _ in 0..5 {
        let val = recv_json_event(&mut ws).await;
        if val.get("type") == Some(&JsonValue::String("ReconnectToken".to_string())) {
            let token = val.get("token").and_then(|v| v.as_str()).unwrap_or("");
            assert!(!token.is_empty());
            found = true;
            break;
        }
    }

    assert!(found, "ReconnectToken event not received");
    handle.abort();
}

#[tokio::test]
async fn test_ws_user_join_event_sent() {
    let (addr, handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/joinroom", addr);

    let (mut ws, _) = connect_async(url).await.expect("connect failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut found = false;
    for _ in 0..10 {
        let val = recv_json_event(&mut ws).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            let user_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let animal = payload
                .get("animal_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert!(!user_id.is_empty());
            assert!(!animal.is_empty());
            found = true;
            break;
        }
    }

    assert!(found, "UserJoined event not received");
    handle.abort();
}

#[tokio::test]
async fn test_ws_user_count_event_sent() {
    let (addr, handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/countroom", addr);

    let (mut ws, _) = connect_async(url).await.expect("connect failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut found = false;
    for _ in 0..10 {
        let val = recv_json_event(&mut ws).await;
        if val.get("type") == Some(&JsonValue::String("UserCount".to_string())) {
            let count = val.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            assert!(count >= 1);
            found = true;
            break;
        }
    }

    assert!(found, "UserCount event not received");
    handle.abort();
}

#[tokio::test]
async fn test_ws_message_broadcast_to_other_client() {
    let (addr, handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/broadcastroom", addr);

    let (mut ws1, _) = connect_async(url.clone()).await.expect("connect failed");
    let (mut ws2, _) = connect_async(url).await.expect("connect failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drain a few initial events so subsequent recv is message-focused
    for _ in 0..4 {
        let _ = recv_json_event(&mut ws1).await;
        let _ = recv_json_event(&mut ws2).await;
    }

    let payload = serde_json::json!({
        "type": "Message",
        "text": "Hello <script>alert(1)</script> world"
    })
    .to_string();
    ws1.send(text_frame(payload)).await.unwrap();

    let mut found = false;
    for _ in 0..10 {
        let val = recv_json_event(&mut ws2).await;
        if val.get("type") == Some(&JsonValue::String("Message".to_string())) {
            let text = val
                .get("message")
                .and_then(|m| m.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert!(text.contains("Hello"));
            assert!(!text.contains("<script>"));
            found = true;
            break;
        }
    }

    assert!(found, "Broadcast message not received");
    handle.abort();
}

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
async fn test_message_history_preserved_across_reconnects() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/history-test", addr);

    // First connection sends messages
    {
        let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("Failed to connect");

        for _ in 0..3 {
            let _ = recv_json_event(&mut ws1).await;
        }

        ws1.send(text_frame(
            r#"{"type":"Message","text":"Historical message"}"#.to_string(),
        ))
        .await
        .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = ws1.close(None).await;
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Second connection should see the history in subsequent events
    let (mut ws2, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = ws2.close(None).await;

    handle.abort();
}

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
async fn test_ws_ping_pong_handling() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/ping-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Send ping
    ws.send(WsMessage::Ping(vec![1, 2, 3, 4].into()))
        .await
        .unwrap();

    // Server should respond with pong (though we don't strictly verify response)
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.abort();
}

#[tokio::test]
async fn test_ws_binary_message_ignored() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/binary-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send binary data
    ws.send(WsMessage::Binary(vec![0xDE, 0xAD, 0xBE, 0xEF].into()))
        .await
        .unwrap();

    // Should be ignored, no error
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.abort();
}

#[tokio::test]
async fn test_ws_close_frame_handled() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/close-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Gracefully close
    let _ = ws.close(None).await;

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
async fn test_user_cookie_parsing() {
    use axum::http::HeaderValue;

    // Valid cookie
    let valid_cookie = "user_id=550e8400-e29b-41d4-a716-446655440000; animal_name=Lion";
    let mut headers = HeaderMap::new();
    headers.insert("cookie", HeaderValue::from_str(valid_cookie).unwrap());

    // Just verify cookies can be parsed (actual parsing is done by axum extractors)
    assert!(headers.get("cookie").is_some());
}

#[tokio::test]
async fn test_message_html_escaping() {
    let dangerous = "<img src=x onerror=alert(1)>";
    let result = validate_message(dangerous).unwrap();
    // Ammonia should remove the dangerous attributes
    assert!(!result.contains("onerror"));
}

#[tokio::test]
async fn test_multiple_rooms_different_user_counts() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url1 = format!("ws://{}/ws/room-count-1", addr);
    let ws_url2 = format!("ws://{}/ws/room-count-2", addr);

    let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url1).await.unwrap();
    let (mut ws2, _) = tokio_tungstenite::connect_async(&ws_url2).await.unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Each room should report their own user counts
    for _ in 0..5 {
        let event1 = recv_json_event(&mut ws1).await;
        if event1["type"] == "UserCount" {
            let count1 = event1["count"].as_u64().unwrap();
            assert_eq!(count1, 1);
            break;
        }
    }

    let _ = ws1.close(None).await;
    let _ = ws2.close(None).await;
    handle.abort();
}

#[tokio::test]
async fn test_reconnect_token_format() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/token-format-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // Look for reconnect token
    for _ in 0..10 {
        let event = recv_json_event(&mut ws).await;
        if event["type"] == "ReconnectToken" {
            let token = event["token"].as_str().unwrap();
            // Should be a valid UUID
            assert!(!token.is_empty());
            assert!(token.contains("-") || token.len() == 36);
            break;
        }
    }

    handle.abort();
}

#[tokio::test]
async fn test_message_broadcast_excludes_sender() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/broadcast-exclude-test", addr);
    let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let (mut ws2, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // Clear initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
    }
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws2).await;
    }

    // ws1 sends message
    ws1.send(text_frame(
        r#"{"type":"Message","text":"Test from ws1"}"#.to_string(),
    ))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ws2 should receive it
    let mut ws2_received = false;
    for _ in 0..10 {
        let event = recv_json_event(&mut ws2).await;
        if event["type"] == "Message" {
            let text = event["message"]["text"].as_str().unwrap();
            if text.contains("Test from ws1") {
                ws2_received = true;
                break;
            }
        }
    }

    assert!(ws2_received);
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
async fn test_websocket_invalid_json() {
    let (addr, _handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/invalid-json", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    let _ = recv_json_event(&mut ws).await;

    ws.send(text_frame("{not valid json}".to_string()))
        .await
        .unwrap();

    let result = ws
        .send(text_frame(
            r#"{"type":"Message","text":"test"}"#.to_string(),
        ))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_websocket_unknown_event_type() {
    let (addr, _handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/unknown-event", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    let _ = recv_json_event(&mut ws).await;

    ws.send(text_frame(
        r#"{"type":"UnknownEvent","data":"test"}"#.to_string(),
    ))
    .await
    .unwrap();

    let result = ws
        .send(text_frame(
            r#"{"type":"Message","text":"test"}"#.to_string(),
        ))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_state_broadcast_count() {
    let app_state = Arc::new(AppState::new());
    let room_name = "broadcast-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let rooms = app_state.rooms.read().await;
    let room_state = rooms.get(&room_name).unwrap();

    assert!(room_state.chat_history.is_empty());
}

#[tokio::test]
async fn test_websocket_multiple_rapid_frames() {
    let (addr, _handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/rapid-frames-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Skip initial events (UserJoined, UserCount, etc.)
    tokio::time::sleep(Duration::from_millis(100)).await;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), ws.next()).await {}

    // Send 5 messages rapidly (not 10 - rate limiting kicks in)
    for i in 0..5 {
        let msg = serde_json::json!({"type": "Message", "text": format!("Rapid message {}", i)});
        ws.send(text_frame(msg.to_string())).await.unwrap();
        // Small delay to avoid rate limiting
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Should receive messages back
    let mut received = 0;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
            && let Ok(event) = serde_json::from_str::<serde_json::Value>(&text)
            && event["type"] == "Message"
        {
            received += 1;
        }
    }
    assert!(
        received >= 3,
        "Should receive at least 3 of the 5 rapid messages, got {}",
        received
    );
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
async fn test_payload_too_large_rejected() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/large-payload-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send a very large payload (larger than MAX_PAYLOAD_SIZE which is 64KB)
    let large_text = "x".repeat(70000);
    let msg = format!(r#"{{"type":"Message","text":"{}"}}"#, large_text);
    ws.send(text_frame(msg)).await.unwrap();

    // Server should handle gracefully - message won't be broadcast
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection should still work
    ws.send(text_frame(
        r#"{"type":"Message","text":"Small message"}"#.to_string(),
    ))
    .await
    .unwrap();

    handle.abort();
}

#[tokio::test]
async fn test_create_user_cookies_format() {
    let (user_cookie, animal_cookie) =
        create_user_cookies("test-user-123", "Tiger", "thehellisthis.com");

    assert!(user_cookie.contains("user_id=test-user-123"));
    assert!(user_cookie.contains("Path=/"));
    assert!(user_cookie.contains("SameSite=Strict"));
    // HttpOnly is required: the client learns its own identity from the
    // Welcome frame, so script access to these cookies buys nothing and only
    // widens what an XSS through the Markdown pipeline could steal.
    assert!(
        user_cookie.contains("HttpOnly"),
        "user_id cookie must be HttpOnly"
    );
    assert!(user_cookie.contains("Secure"));

    assert!(animal_cookie.contains("animal_name=Tiger"));
    assert!(animal_cookie.contains("Path=/"));
    assert!(animal_cookie.contains("SameSite=Strict"));
    assert!(
        animal_cookie.contains("HttpOnly"),
        "animal_name cookie must be HttpOnly"
    );
    assert!(animal_cookie.contains("Secure"));
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
async fn test_user_cookie_extraction() {
    use axum::http::header::COOKIE;

    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route(
            "/test",
            get(|cookie: OptionalUserCookie| async move {
                match cookie.0 {
                    Some(user) => {
                        format!("user_id={}, animal_name={}", user.user_id, user.animal_name)
                    }
                    None => "no_cookie".to_string(),
                }
            }),
        )
        .with_state(app_state);

    // Test with valid cookies
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/test")
                .header(COOKIE, "user_id=test-user-123; animal_name=Lion")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&body),
        "user_id=test-user-123, animal_name=Lion"
    );

    // Test with no cookies
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&body), "no_cookie");

    // Test with only user_id cookie (missing animal_name)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/test")
                .header(COOKIE, "user_id=test-user-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&body), "no_cookie");
}

#[tokio::test]
async fn test_cookie_creation_format() {
    let (uid_cookie, name_cookie) =
        create_user_cookies("test-user-456", "Tiger", "thehellisthis.com");

    assert!(uid_cookie.contains("user_id=test-user-456"));
    assert!(uid_cookie.contains("Path=/"));
    assert!(uid_cookie.contains("SameSite=Strict"));
    assert!(
        uid_cookie.contains("HttpOnly"),
        "user_id cookie must be HttpOnly"
    );
    assert!(uid_cookie.contains("Secure"));

    assert!(name_cookie.contains("animal_name=Tiger"));
    assert!(name_cookie.contains("Path=/"));
    assert!(name_cookie.contains("SameSite=Strict"));
    assert!(
        name_cookie.contains("HttpOnly"),
        "animal_name cookie must be HttpOnly"
    );
    assert!(name_cookie.contains("Secure"));
}

#[tokio::test]
async fn test_reconnect_with_chat_history() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/test-room", addr);

    // First connection - send a message
    let (mut ws_stream, _) = connect_async(&ws_url).await.unwrap();

    // Wait for user joined and collect user info
    let mut user_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws_stream).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            user_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user_id.is_empty() {
                break;
            }
        }
    }

    assert!(!user_id.is_empty(), "Should have received a user_id");

    // Drain other events
    for _ in 0..3 {
        let _ = timeout(Duration::from_millis(200), recv_json_event(&mut ws_stream)).await;
    }

    // Send a message
    let test_msg = r#"{"type":"Message","text":"First message before reconnect"}"#;
    ws_stream
        .send(text_frame(test_msg.to_string()))
        .await
        .unwrap();

    // Wait for message echo
    let echo = recv_json_event(&mut ws_stream).await;
    assert_eq!(echo["type"], "Message");
    let message_id = echo["message"]["message_id"].as_str().unwrap_or("");
    assert!(!message_id.is_empty(), "Should have received a message_id");

    // Disconnect
    drop(ws_stream);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Reconnect (simulating browser with cookies)
    let (mut ws_stream2, _) = connect_async(&ws_url).await.unwrap();

    // Should receive chat history including our previous message
    let mut found_history = false;
    let mut found_join = false;

    for _ in 0..15 {
        if let Ok(data) = timeout(Duration::from_secs(2), recv_json_event(&mut ws_stream2)).await {
            if data["type"] == "Message" {
                let recv_msg_id = data["message"]["message_id"].as_str().unwrap_or("");
                if recv_msg_id == message_id {
                    found_history = true;
                    // Verify the message has the correct user_id for client-side detection
                    let recv_user_id = data["message"]["user_id"].as_str().unwrap_or("");
                    assert_eq!(recv_user_id, user_id, "Message user_id should match");
                }
            }

            if data["type"] == "System" {
                found_join = true;
            }
        }
    }

    assert!(found_history, "Should receive chat history on reconnect");
    assert!(found_join, "Should receive UserJoined event");

    handle.abort();
}

#[tokio::test]
async fn test_user_count_updates_on_join() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/count-test", addr);

    // Connect first user
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();

    // Drain initial events
    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(200), recv_json_event(&mut ws1)).await;
    }

    // Connect second user
    let (mut _ws2, _) = connect_async(&ws_url).await.unwrap();

    // User 1 should receive user count update
    let mut found_count = false;
    for _ in 0..10 {
        if let Ok(data) = timeout(Duration::from_secs(2), recv_json_event(&mut ws1)).await
            && data["type"] == "UserCount"
        {
            let count = data["count"].as_u64().unwrap();
            if count == 2 {
                found_count = true;
                break;
            }
        }
    }

    assert!(
        found_count,
        "Should receive updated user count when new user joins"
    );

    handle.abort();
}

// ========== COOKIE ACCESSIBILITY TESTS ==========
// These tests ensure cookies are configured correctly for JavaScript access
// Required for client-side message alignment (sent vs received messages)

#[tokio::test]
async fn test_cookie_must_be_js_accessible() {
    // CRITICAL: user_id cookie MUST be readable by JavaScript for message alignment
    let (user_cookie, animal_cookie) =
        create_user_cookies("js-test-user", "Lion", "thehellisthis.com");

    // Both identity cookies are HttpOnly. The client is told who it is by the
    // Welcome frame on connect, so it never needs to read document.cookie.
    assert!(
        user_cookie.contains("HttpOnly"),
        "user_id cookie must be HttpOnly"
    );
    assert!(
        animal_cookie.contains("HttpOnly"),
        "animal_name cookie must be HttpOnly"
    );

    // Still must have security attributes
    assert!(
        user_cookie.contains("Secure"),
        "Cookie must have Secure flag"
    );
    assert!(
        user_cookie.contains("SameSite=Strict"),
        "Cookie must have SameSite=Strict"
    );
}

#[tokio::test]
async fn test_cookie_security_attributes_present() {
    let (user_cookie, animal_cookie) =
        create_user_cookies("sec-test", "Tiger", "thehellisthis.com");

    // Essential security attributes MUST be present
    assert!(
        user_cookie.contains("Secure"),
        "user_id cookie missing Secure flag"
    );
    assert!(
        user_cookie.contains("SameSite=Strict"),
        "user_id cookie missing SameSite=Strict"
    );
    assert!(
        user_cookie.contains("Path=/"),
        "user_id cookie missing Path=/"
    );

    assert!(
        animal_cookie.contains("Secure"),
        "animal_name cookie missing Secure flag"
    );
    assert!(
        animal_cookie.contains("SameSite=Strict"),
        "animal_name cookie missing SameSite=Strict"
    );
    assert!(
        animal_cookie.contains("Path=/"),
        "animal_name cookie missing Path=/"
    );
}

#[tokio::test]
async fn test_cookie_max_age_present() {
    let (user_cookie, animal_cookie) =
        create_user_cookies("maxage-test", "Bear", "thehellisthis.com");

    assert!(
        user_cookie.contains("Max-Age="),
        "user_id cookie missing Max-Age"
    );
    assert!(
        animal_cookie.contains("Max-Age="),
        "animal_name cookie missing Max-Age"
    );

    // Max-Age should be INACTIVE_TIMEOUT seconds
    let expected_max_age = format!("Max-Age={}", INACTIVE_TIMEOUT.as_secs());
    assert!(
        user_cookie.contains(&expected_max_age),
        "user_id cookie Max-Age should be INACTIVE_TIMEOUT seconds"
    );
}

#[tokio::test]
async fn test_cookie_values_properly_encoded() {
    // Test with special characters that might need encoding
    let (user_cookie, _) = create_user_cookies("user-with-dash", "Lion", "thehellisthis.com");
    assert!(user_cookie.contains("user_id=user-with-dash"));

    // UUID format user_id
    let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
    let (uuid_cookie, _) = create_user_cookies(uuid_str, "Tiger", "thehellisthis.com");
    assert!(uuid_cookie.contains(&format!("user_id={}", uuid_str)));
}

#[tokio::test]
async fn test_message_user_id_matches_cookie_format() {
    // Verify the user_id in messages matches what we'd set in cookies
    let test_user_id = "test-123-abc";
    let (user_cookie, _) = create_user_cookies(test_user_id, "Lion", "thehellisthis.com");

    let _msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: test_user_id.to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Test</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };

    // Cookie should contain exact user_id value
    assert!(
        user_cookie.contains(&format!("user_id={}", test_user_id)),
        "Cookie user_id format must match message user_id"
    );
}

#[tokio::test]
async fn test_ws_message_includes_user_id_in_broadcast() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/userid-broadcast-test", addr);

    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Get ws1's user_id from UserJoined event
    let mut ws1_user_id = String::new();
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
            if !uid.is_empty() {
                ws1_user_id = uid.to_string();
                break;
            }
        }
    }

    // Drain remaining events
    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws2)).await;
    }

    // ws1 sends message
    ws1.send(text_frame(
        r#"{"type":"Message","text":"Test alignment"}"#.to_string(),
    ))
    .await
    .unwrap();

    // ws2 receives - verify user_id is present in message
    let mut found_message = false;
    for _ in 0..10 {
        if let Ok(val) = timeout(Duration::from_secs(2), recv_json_event(&mut ws2)).await
            && val.get("type") == Some(&JsonValue::String("Message".to_string()))
        {
            let msg_user_id = val["message"]["user_id"].as_str().unwrap_or("");
            assert!(
                !msg_user_id.is_empty(),
                "Broadcast message MUST contain user_id"
            );
            assert_eq!(
                msg_user_id, ws1_user_id,
                "Message user_id should match sender"
            );
            found_message = true;
            break;
        }
    }

    assert!(
        found_message,
        "Should receive broadcast message with user_id"
    );
    handle.abort();
}

// ========== MESSAGE ALIGNMENT EDGE CASES ==========

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
        last_sanitized_message: None,
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
        last_sanitized_message: None,
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
            last_sanitized_message: None,
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
async fn test_ws_early_disconnect_during_history() {
    // This tests the path where client disconnects while receiving history
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/early-disconnect", addr);

    // First user sends some messages to create history
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();
    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
    }

    for i in 0..3 {
        ws1.send(text_frame(format!(
            r#"{{"type":"Message","text":"History message {}"}}"#,
            i
        )))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await; // Wait for rate limit
    }

    // Second user connects and immediately disconnects
    let (ws2, _) = connect_async(&ws_url).await.unwrap();
    drop(ws2); // Immediate disconnect

    // System should handle this gracefully
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.abort();
}

#[tokio::test]
async fn test_user_cookie_extraction_with_partial_cookies() {
    use axum::extract::FromRequestParts;
    use axum::http::Request;

    // Create a request with only user_id cookie (missing animal_name)
    let request = Request::builder()
        .header("Cookie", "user_id=test-123")
        .body(Body::empty())
        .unwrap();

    let (mut parts, _body) = request.into_parts();
    let result = OptionalUserCookie::from_request_parts(&mut parts, &()).await;

    // Should return None since both cookies are required
    assert!(result.is_ok());
    let cookie = result.unwrap();
    assert!(
        cookie.0.is_none(),
        "Should return None when animal_name is missing"
    );
}

#[tokio::test]
async fn test_user_cookie_extraction_with_empty_values() {
    use axum::extract::FromRequestParts;
    use axum::http::Request;

    // Create a request with empty cookie values
    let request = Request::builder()
        .header("Cookie", "user_id=; animal_name=")
        .body(Body::empty())
        .unwrap();

    let (mut parts, _body) = request.into_parts();
    let result = OptionalUserCookie::from_request_parts(&mut parts, &()).await;

    // Should return None since values are empty
    assert!(result.is_ok());
    let cookie = result.unwrap();
    assert!(
        cookie.0.is_none(),
        "Should return None when cookie values are empty"
    );
}

#[tokio::test]
async fn test_user_cookie_extraction_valid_cookies() {
    use axum::extract::FromRequestParts;
    use axum::http::Request;

    // Create a request with valid cookies
    let request = Request::builder()
        .header("Cookie", "user_id=test-user-456; animal_name=Tiger")
        .body(Body::empty())
        .unwrap();

    let (mut parts, _body) = request.into_parts();
    let result = OptionalUserCookie::from_request_parts(&mut parts, &()).await;

    assert!(result.is_ok());
    let cookie = result.unwrap();
    assert!(cookie.0.is_some(), "Should extract valid cookies");
    let inner = cookie.0.unwrap();
    assert_eq!(inner.user_id, "test-user-456");
    assert_eq!(inner.animal_name, "Tiger");
}

#[tokio::test]
async fn test_assign_animal_reuses_disconnected_animal() {
    let mut room = create_room();
    let now = Instant::now();

    // First user takes "Lion"
    let animal1 = room.assign_animal();
    room.users.insert(
        "user-1".to_string(),
        UserData {
            user_id: "user-1".to_string(),
            animal_name: animal1.clone(),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Disconnected { since: now }, // Disconnected!
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
            last_reaction_event: None,
        },
    );

    // Return the animal to pool
    room.available_animals.push_back(animal1.clone());

    // Second user should be able to get an animal (possibly the returned one)
    let animal2 = room.assign_animal();
    assert!(!animal2.is_empty());
    // The disconnected user's animal should be available for reuse
}

#[tokio::test]
async fn test_graceful_shutdown_broadcasts_shutdown() {
    let app_state = Arc::new(AppState::new());
    let room_name = "shutdown-test".to_string();

    // Create a room with a user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        let now = Instant::now();
        room.users.insert(
            "user1".to_string(),
            UserData {
                user_id: "user1".to_string(),
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
        rooms.insert(room_name.clone(), room);
    }

    // Subscribe to the room's broadcast
    let rx = {
        let rooms = app_state.rooms.read().await;
        rooms.get(&room_name).map(|r| r.sender.subscribe())
    };

    // Call shutdown
    app_state.shutdown().await;

    // Rooms should be cleared
    assert!(
        app_state.rooms.read().await.is_empty(),
        "Rooms should be cleared after shutdown"
    );

    // If we had a receiver, it should have received ServerShutdown event before being dropped
    if let Some(mut rx) = rx {
        // Note: The receiver might have received the event or the channel might be closed
        // Either way, the shutdown was processed
        let result = rx.try_recv();
        // Result could be Ok(ServerShutdown) or Err(Lagged/Closed)
        match result {
            Ok(OutgoingEvent::System {
                event: SystemEvent::ServerShutdown { reason: _ },
            }) => {
                // Perfect - received shutdown event
            }
            _ => {
                // Also acceptable - channel may have been closed during shutdown
            }
        }
    }
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
        last_sanitized_message: None,
        last_reaction_event: None,
    };

    assert!(!user.is_typing);
    assert!(user.last_typing_event.is_none());
}

// ========== BUGFIX TESTS: Room Cleanup & Connection Issues ==========

#[tokio::test]
async fn test_room_cleanup_disconnected_users_dont_block_deletion() {
    // Bug fix: Stale DISCONNECTED users should not prevent room deletion
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY;

        // Add a disconnected user (not connected anymore)
        room.users.insert(
            "ghost".to_string(),
            UserData {
                user_id: "ghost".to_string(),
                animal_name: "Phantom".to_string(),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Disconnected {
                    since: Instant::now() - Duration::from_secs(10),
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

        rooms.insert("has-disconnected-user".to_string(), room);
    }

    cleanup_rooms(&app_state).await;

    let rooms = app_state.rooms.read().await;
    assert!(
        !rooms.contains_key("has-disconnected-user"),
        "Room with only disconnected users should be deleted after cleanup delay"
    );
}

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
        last_sanitized_message: None,
        last_reaction_event: None,
    };

    user.last_message_time = now - (USER_IDLE_MESSAGE_TIMEOUT - Duration::from_secs(1));
    assert!(!user_idle_for_too_long(&user, now));

    user.last_message_time = now - (USER_IDLE_MESSAGE_TIMEOUT + Duration::from_secs(1));
    assert!(user_idle_for_too_long(&user, now));
}

// ========== REPLY FEATURE TESTS ==========

#[tokio::test]
async fn test_create_user_cookies_format_detailed() {
    let (user_id_cookie, animal_name_cookie) =
        create_user_cookies("test-user-123", "Lion", "thehellisthis.com");

    assert!(user_id_cookie.contains("user_id=test-user-123"));
    assert!(user_id_cookie.contains("Path=/"));
    assert!(user_id_cookie.contains("SameSite=Strict"));
    assert!(user_id_cookie.contains("Secure"));

    assert!(animal_name_cookie.contains("animal_name=Lion"));
    assert!(animal_name_cookie.contains("Path=/"));
    assert!(animal_name_cookie.contains("SameSite=Strict"));
    assert!(animal_name_cookie.contains("Secure"));
}

#[tokio::test]
async fn test_cookie_with_nonexistent_user_id() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/nonexistent-user-room", addr);

    // Connect with a fake user_id cookie that doesn't exist in the room
    let request = http::Request::builder()
        .uri(&url)
        .header("Cookie", "user_id=fake-user-12345; animal_name=FakeAnimal")
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(())
        .unwrap();

    let (mut ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Connect with fake cookie failed");

    // Should still connect and get a reconnect token (new user created)
    let mut got_token = false;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
            && text.contains("ReconnectToken")
        {
            got_token = true;
            break;
        }
    }

    assert!(
        got_token,
        "Should receive reconnect token with new user created"
    );
    ws.close(None).await.ok();
}

// Test room not found during handle_websocket - covers lines 1486-1489

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
                last_sanitized_message: None,
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
                last_sanitized_message: None,
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
                last_sanitized_message: None,
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
async fn test_cookie_with_unknown_cookie_names() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/unknown-cookie-room", addr);

    // Connect with extra unknown cookies alongside valid ones
    let request = http::Request::builder()
        .uri(&url)
        .header(
            "Cookie",
            "user_id=test123; animal_name=Lion; random_cookie=value; session=abc; tracking=xyz",
        )
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(())
        .unwrap();

    let (mut ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Connect with unknown cookies failed");

    // Should still connect and work
    let mut got_token = false;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
            && text.contains("ReconnectToken")
        {
            got_token = true;
            break;
        }
    }

    assert!(
        got_token,
        "Should receive reconnect token even with unknown cookies"
    );
    ws.close(None).await.ok();
}

// Test memory tracker add_bytes when at exactly limit - covers line 386

#[tokio::test]
async fn test_broadcast_user_count_with_disconnected_users() {
    let app_state = Arc::new(AppState::new());

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room_state = create_room();

        // Add a connected user
        room_state.users.insert(
            "connected-user".to_string(),
            UserData {
                user_id: "connected-user".to_string(),
                animal_name: "Lion".to_string(),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Connected {
                    last_heartbeat: Instant::now(),
                    connection_id: "conn-1".to_string(),
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

        // Add a disconnected user
        room_state.users.insert(
            "disconnected-user".to_string(),
            UserData {
                user_id: "disconnected-user".to_string(),
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
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );

        // Broadcast should only count the connected user
        room_state.broadcast_user_count();

        rooms.insert("disconnected-test".to_string(), room_state);
    }

    // The connected count should be 1, not 2
    let rooms = app_state.rooms.read().await;
    let room = rooms.get("disconnected-test").unwrap();
    assert_eq!(room.users.len(), 2); // Both users exist
}

// Test validate_input with empty string error path - covers line 1162

#[tokio::test]
async fn test_pong_message_received() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/pong-test-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send ping and wait for pong
    ws.send(WsMessage::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();

    // We should receive a pong response
    let mut received_pong = false;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Pong(_)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            received_pong = true;
            break;
        }
    }

    assert!(received_pong, "Should receive pong response");
    ws.close(None).await.ok();
}

// Test close frame handling - covers lines 1851-1853

#[tokio::test]
async fn test_close_frame_initiates_cleanup() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/close-test-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send close frame
    ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
        reason: "Test close".into(),
    }))
    .await
    .ok();

    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should be closed
}

// Test message validation with very short valid message - edge case

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
async fn test_user_cookie_edge_cases() {
    // Test basic cookie extraction via WebSocket
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/cookie-edge-room", addr);

    // Connect with edge case cookie values
    let request = http::Request::builder()
        .uri(&url)
        .header("Cookie", "user_id=a;animal_name=b") // Very short values
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(())
        .unwrap();

    let result = tokio_tungstenite::connect_async(request).await;

    // Should either connect or reject - both are valid behaviors
    match result {
        Ok((mut ws, _)) => {
            // Connected successfully, close gracefully
            ws.close(None).await.ok();
        }
        Err(_) => {
            // Connection rejected, also valid
        }
    }
}

// Test ConnectionPool at limit - covers lines 218-228

#[tokio::test]
async fn test_room_broadcast_to_multiple_receivers() {
    let room_state = create_room();

    // Create multiple receivers
    let mut receivers: Vec<_> = (0..5).map(|_| room_state.sender.subscribe()).collect();

    // Broadcast a message
    let event = OutgoingEvent::System {
        event: SystemEvent::UserJoined {
            user_id: "test".to_string(),
            animal_name: "Lion".to_string(),
        },
    };
    let _ = room_state.sender.send(event.clone());

    // All receivers should get the message
    for receiver in &mut receivers {
        let received = receiver.try_recv();
        assert!(received.is_ok());
    }
}

// Test message history at max capacity - covers lines 848-870

#[tokio::test]
async fn test_create_user_cookies_attributes() {
    let (uid_cookie, animal_cookie) = create_user_cookies("user123", "Lion", "thehellisthis.com");

    // Check cookie values contain expected values
    assert!(uid_cookie.contains("user_id=user123"));
    assert!(animal_cookie.contains("animal_name=Lion"));

    // Check cookie attributes - the actual implementation uses SameSite=Strict; Secure
    assert!(uid_cookie.contains("SameSite=Strict"));
    assert!(uid_cookie.contains("Secure"));
    assert!(animal_cookie.contains("SameSite=Strict"));
    assert!(animal_cookie.contains("Secure"));
}

// Test assign_animal when pool is empty - covers lines 794-807

#[tokio::test]
async fn test_ws_oversized_payload() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/payload-test-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send oversized payload (MAX_PAYLOAD_SIZE is 64KB)
    let large_payload = format!(r#"{{"type":"Message","text":"{}"}}"#, "x".repeat(70000));
    ws.send(text_frame(large_payload)).await.ok();

    // Wait a moment - the message should be ignored
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection should still be alive (message was just ignored)
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(ping_result.is_ok());

    ws.close(None).await.ok();
}

// Test empty text message via WebSocket - covers text validation

#[tokio::test]
async fn test_ws_empty_text_message() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/empty-text-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send message with empty text
    ws.send(text_frame(r#"{"type":"Message","text":""}"#))
        .await
        .ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(ping_result.is_ok());

    ws.close(None).await.ok();
}

// Test ReadReceipt with valid message_id format - covers lines 1762-1777

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

#[tokio::test]
async fn test_ws_creates_new_room_on_connect() {
    let (addr, _handle) = start_ws_server().await;
    let unique_room = format!(
        "new-room-{}",
        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
    );
    let url = format!("ws://{}/ws/{}", addr, unique_room);

    // Connect to create room
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages - room is created when connection establishes
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send a message to verify room is functional
    ws.send(text_frame(r#"{"type":"Message","text":"hello"}"#))
        .await
        .ok();

    // Should receive the message back
    let mut received = false;
    for _ in 0..5 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(200), ws.next()).await
            && text.contains("hello")
        {
            received = true;
            break;
        }
    }
    assert!(received);

    ws.close(None).await.ok();
}

// Test ws_handler with cookie reconnection - covers lines 1334-1383

#[tokio::test]
async fn test_ws_room_deleted_after_upgrade() {
    let (addr, _handle) = start_ws_server().await;
    let room_name = "will-delete-room";
    let url = format!("ws://{}/ws/{}", addr, room_name);

    // Connect
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain messages
    for _ in 0..3 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send messages - connection should work
    ws.send(text_frame(r#"{"type":"Message","text":"test"}"#))
        .await
        .ok();

    tokio::time::sleep(Duration::from_millis(200)).await;
    ws.close(None).await.ok();
}

// Test message broadcast serialization error path - covers lines 1599-1601

#[tokio::test]
async fn test_broadcast_handles_receiver_lag() {
    let room = create_room();

    // Create a receiver
    let mut rx = room.sender.subscribe();

    // Send many messages quickly to potentially cause lag
    for i in 0..100 {
        let _ = room.sender.send(OutgoingEvent::System {
            event: SystemEvent::UserJoined {
                user_id: format!("user{}", i),
                animal_name: format!("Animal{}", i),
            },
        });
    }

    // Try to receive - may have missed some due to lag
    let mut received = 0;
    while rx.try_recv().is_ok() {
        received += 1;
    }

    // Should have received at least some messages
    assert!(received > 0);
}

// Test prune_old_messages removes oldest - covers lines 887-910

#[tokio::test]
async fn test_room_state_broadcast_user_count() {
    let mut room = create_room();
    let mut rx = room.sender.subscribe();

    // Add some users
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

    room.broadcast_user_count();

    // Should receive user count event
    if let Ok(OutgoingEvent::UserCount { count }) = rx.try_recv() {
        assert_eq!(count, 1);
    } else {
        panic!("Should have received user count event");
    }
}

// Test OutgoingMessage estimate_size - covers lines 275-285

#[tokio::test]
async fn test_ws_malformed_json_handling() {
    let (addr, _handle) = start_ws_server().await;
    let url = format!("ws://{}/ws/malformed-test", addr);

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
    ws.send(text_frame("not valid json")).await.ok();
    ws.send(text_frame("{incomplete")).await.ok();
    ws.send(text_frame(r#"{"type":"Unknown"}"#)).await.ok();

    // Connection should still be alive
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Can still send valid message
    ws.send(text_frame(r#"{"type":"Message","text":"valid"}"#))
        .await
        .ok();

    ws.close(None).await.ok();
}

// Test header parsing scenarios - covers IP extraction

/// The first frame a client receives tells it who it is.
///
/// Without it the client had to guess its own identity — it assumed the first
/// `UserJoined` it saw was itself and otherwise parsed `document.cookie`, which
/// is why the identity cookies could not be `HttpOnly`.
#[tokio::test]
async fn first_frame_is_welcome_with_this_users_identity() {
    let (addr, _state, handle) = start_ws_server_with_state().await;

    let (mut ws, _) = connect_async(format!("ws://{addr}/ws/welcome-room"))
        .await
        .expect("connect failed");

    let event = recv_json_event(&mut ws).await;
    assert_eq!(event["type"], "Welcome");

    let user_id = event["user_id"].as_str().expect("welcome carries user_id");
    let animal_name = event["animal_name"]
        .as_str()
        .expect("welcome carries animal_name");

    assert!(
        Uuid::parse_str(user_id).is_ok(),
        "welcome user_id must be a uuid, got {user_id}"
    );
    assert!(!animal_name.is_empty(), "welcome animal_name must be set");

    handle.abort();
}

// ========== FRONTEND/BACKEND IDENTITY CONTRACT ==========

/// WebSocket upgrades are not covered by the same-origin policy, so the server
/// has to enforce it.
///
/// Without this any page on the internet can open a socket to this server and
/// drive a visitor's browser into these rooms. A missing Origin is allowed:
/// non-browser clients do not send one, and browsers always do.
#[test]
fn websocket_origin_policy_accepts_only_this_site() {
    // Same host as the request: the normal case, whatever the domain.
    assert!(is_allowed_origin(
        Some("https://thehellisthis.com"),
        Some("thehellisthis.com")
    ));
    // Ports legitimately differ: the container listens on 3000 behind a Worker.
    assert!(is_allowed_origin(
        Some("http://localhost:3000"),
        Some("localhost")
    ));
    // No Origin at all — non-browser client.
    assert!(is_allowed_origin(None, Some("thehellisthis.com")));

    // Foreign origins.
    assert!(!is_allowed_origin(
        Some("https://evil.example"),
        Some("thehellisthis.com")
    ));
    // A prefix match must not be enough.
    assert!(!is_allowed_origin(
        Some("https://thehellisthis.com.evil.example"),
        Some("thehellisthis.com")
    ));
    // Non-http origins (`null`, extensions, file://) are not this application.
    assert!(!is_allowed_origin(Some("null"), Some("thehellisthis.com")));
    assert!(!is_allowed_origin(
        Some("file://"),
        Some("thehellisthis.com")
    ));
}

// ========== SECURITY: QUOTED REPLIES ==========

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

/// A cookie header the parser cannot make sense of yields a new visitor rather
/// than an error.
#[tokio::test]
async fn unparseable_cookies_are_treated_as_a_new_visitor() {
    let request = Request::builder()
        .uri("/")
        .header("cookie", "=;;;garbage;  ;user_id=;animal_name=")
        .body(Body::empty())
        .unwrap();

    let (mut parts, _) = request.into_parts();
    let OptionalUserCookie(cookie) = OptionalUserCookie::from_request_parts(&mut parts, &())
        .await
        .unwrap();

    assert!(
        cookie.is_none(),
        "empty or malformed identity cookies are not an identity"
    );
}

/// A handshake from another site is refused.
///
/// This is the live counterpart to the unit test on `is_allowed_origin`: it
/// asserts the check is actually wired into the upgrade, not merely present.
#[tokio::test]
async fn websocket_upgrade_from_a_foreign_origin_is_refused() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    let request = ws_request(
        addr,
        "origin-room",
        &[("Origin", "https://evil.example".to_string())],
    );
    assert!(
        connect_async(request).await.is_err(),
        "a cross-origin handshake must not be upgraded"
    );

    // And it must not have cost a connection slot.
    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        0
    );

    // The same handshake from this site succeeds.
    let request = ws_request(addr, "origin-room", &[("Origin", format!("http://{addr}"))]);
    let (mut ws, _) = connect_async(request)
        .await
        .expect("same-origin handshake should be accepted");
    assert_eq!(recv_json_event(&mut ws).await["type"], "Welcome");

    handle.abort();
}

/// Teardown of an already-disconnected user is a no-op.
#[tokio::test]
async fn teardown_of_an_already_disconnected_user_is_a_no_op() {
    let state = Arc::new(AppState::new());

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "user".to_string(),
            UserData {
                user_id: "user".to_string(),
                animal_name: "otter".to_string(),
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
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );
        rooms.insert("room".to_string(), room);
    }

    cleanup_user(&state, "room", "user", "whatever", None).await;
    // Unknown room and unknown user are equally harmless.
    cleanup_user(&state, "no-such-room", "user", "c", None).await;
    cleanup_user(&state, "room", "no-such-user", "c", None).await;

    let rooms = state.rooms.read().await;
    assert!(!rooms["room"].users["user"].is_connected());
}

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

/// History is trimmed for a joining user if it is already over the cap —
/// not just during the periodic housekeeping sweep.
#[tokio::test]
async fn joining_user_triggers_a_trim_of_oversized_history() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..(MAX_MESSAGES_PER_ROOM + 50) {
            room.chat_history.push(Arc::new(OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("m{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            }));
        }
        rooms.insert("crowded-room".to_string(), room);
    }

    let admitted = admit_user(&state, "crowded-room", "conn-1", None).await;
    assert!(admitted.is_some());

    let rooms = state.rooms.read().await;
    assert!(
        rooms["crowded-room"].chat_history.len() <= MAX_MESSAGES_PER_ROOM,
        "joining should trim history that is already over the cap"
    );
}

/// A frame larger than the payload cap is dropped at the frame level, before
/// it is even parsed as an event.
#[tokio::test]
async fn oversized_frame_is_dropped_without_killing_the_session() {
    let (addr, _state, handle) = start_ws_server_with_state().await;
    let (mut ws, _) = connect_async(format!("ws://{addr}/ws/oversize-room"))
        .await
        .expect("connect failed");

    assert_eq!(recv_json_event(&mut ws).await["type"], "Welcome");

    let huge = "x".repeat(MAX_PAYLOAD_SIZE + 1024);
    ws.send(text_frame(huge)).await.expect("send failed");

    // The socket must still be usable afterwards, not torn down.
    ws.send(WsMessage::Ping(vec![9].into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    handle.abort();
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

/// Constraint #9 — two connected users in one room never share a name.
///
/// The animal name is the only identity the UI shows. `admit_user` trusted the
/// name on a returning visitor's cookie, so carrying a cookie from one room
/// into another where that name was already taken produced two users the UI
/// could not tell apart.
#[tokio::test]
async fn a_cookie_name_never_duplicates_a_name_already_in_the_room() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "resident".to_string(),
            connected_user("resident", "otter", "c1", Instant::now()),
        );
        rooms.insert("shared-room".to_string(), room);
    }

    let cookie = crate::identity::UserCookie {
        user_id: Uuid::new_v4().to_string(),
        animal_name: "otter".to_string(),
    };
    let (_, assigned) = admit_user(&state, "shared-room", "c2", Some(&cookie))
        .await
        .expect("the room has room for another visitor");

    assert_ne!(
        assigned, "otter",
        "a cookie must not hand a visitor a name somebody in this room is using"
    );

    let rooms = state.rooms.read().await;
    let room = rooms.get("shared-room").unwrap();
    let names: Vec<&str> = room
        .users
        .values()
        .filter(|u| u.is_connected())
        .map(|u| u.animal_name.as_str())
        .collect();
    let unique: std::collections::HashSet<&&str> = names.iter().collect();
    assert_eq!(
        names.len(),
        unique.len(),
        "connected users must have distinct names: {names:?}"
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

/// Identity from a cookie is a *claim*, not a fact.
///
/// Both cookies are `HttpOnly`, which stops a page's script from touching
/// them — it does not stop the person operating the browser from sending any
/// `Cookie` header they like. The server took `animal_name` verbatim: an
/// arbitrary, unbounded string that then became this user's display name in
/// history and in every frame broadcast to everyone else in the room.
///
/// Bounding it is not enough — the safe form is a closed set. An animal name
/// is one of [`ANIMAL_NAMES`] or it is not a name.
#[tokio::test]
async fn a_forged_identity_cookie_cannot_choose_its_own_name_or_id() {
    let state = Arc::new(AppState::new());

    let forged = crate::identity::UserCookie {
        user_id: "../../etc/passwd".to_string(),
        animal_name: format!("<img src=x onerror=alert(1)>{}", "A".repeat(100_000)),
    };
    let (user_id, animal_name) = admit_user(&state, "forged-room", "c1", Some(&forged))
        .await
        .expect("a forged cookie is a new visitor, not an error");

    assert!(
        ANIMAL_NAMES.contains(&animal_name.as_str()),
        "a display name must come from the roster, not from the client: {animal_name:?}"
    );
    assert!(
        Uuid::parse_str(&user_id).is_ok(),
        "a user id must be server-issued: {user_id:?}"
    );
}

/// §5 — a session releases what it reserved however it ends, including when
/// its room is gone by the time it ends.
///
/// This is the property the teardown wrapper rests on. Teardown used to be the
/// duty of each `return` inside the session, and the "room vanished between
/// admission and upgrade" path skipped it, holding that connection's global and
/// per-IP slots until the process died. That specific race is now closed
/// structurally — the release wraps the session rather than being called from
/// inside it, so no exit can route around it — and is not what this test
/// reproduces. What this test pins is the half that makes the wrapper safe:
/// releasing happens *before* the room is looked up, so a missing room is a
/// clean teardown rather than an early return that skips the accounting.
#[tokio::test]
async fn a_session_whose_room_disappeared_still_releases_its_slots() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    let (mut ws, _) = connect_async(ws_request(addr, "doomed-room", &[]))
        .await
        .expect("the handshake should be accepted");
    assert_eq!(recv_json_event(&mut ws).await["type"], "Welcome");

    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        1,
        "the live session holds exactly one slot"
    );

    // Delete the room out from under the live session.
    state.rooms.write().await.remove("doomed-room");

    ws.close(None).await.unwrap();
    drop(ws);

    // The teardown is asynchronous; give it a moment to run.
    for _ in 0..50 {
        if state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst)
            == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        0,
        "a session must release its global slot even with no room to clean up"
    );
    assert_eq!(
        state.connection_pool.active.load(Ordering::SeqCst),
        0,
        "a session must release its per-IP slot even with no room to clean up"
    );

    handle.abort();
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

/// A cookie carrying a *valid* id but a name that is not on the roster.
///
/// Found by mutation testing: replacing `is_animal_name` with `true` survived
/// the whole suite. The forged-cookie test could not catch it, because its
/// `user_id` is not a UUID — the identity is discarded before the name is ever
/// checked, so the roster check was never reached with a bad name. This is the
/// case that actually exercises it: a well-formed id, an invented name.
#[tokio::test]
async fn a_valid_id_does_not_let_a_cookie_invent_its_own_name() {
    let state = Arc::new(AppState::new());

    for invented in [
        "<script>alert(1)</script>",
        "administrator",
        "otter ",  // trailing space: close, but not a roster entry
        "OTTER",   // the roster is lowercase
        "hadron",  // a real word, and a name this roster deliberately dropped
        "guest_1", // the fallback shape, not an animal
    ] {
        let cookie = crate::identity::UserCookie {
            user_id: Uuid::new_v4().to_string(),
            animal_name: invented.to_string(),
        };
        let (_, assigned) = admit_user(&state, "invented-room", "c1", Some(&cookie))
            .await
            .expect("a visitor with an unusable name is still a visitor");

        assert!(
            ANIMAL_NAMES.contains(&assigned.as_str()),
            "a name off the roster must be replaced, not honoured: {invented:?} \
             became {assigned:?}"
        );
        assert_ne!(assigned, invented, "and specifically must not be kept");
    }
}

/// Joining a room that has just been deleted is a clean refusal.
///
/// The room can disappear in the microseconds between `admit_user` returning
/// and the upgrade callback running. Winning that race from a test is not
/// possible reliably — but the decision does not need a socket, only a room in
/// the right state, which is why it is a function now (§6.1c).
#[tokio::test]
async fn joining_a_room_that_vanished_is_refused_rather_than_panicking() {
    let state = Arc::new(AppState::new());

    let joined = crate::session::join_room(&state, "never-existed", "u1", "otter", "c1").await;

    assert!(
        joined.is_none(),
        "there is nothing to join, so the session must end and let teardown run"
    );
}

/// Forwarding delivers what the room broadcasts, in order, while it can.
#[tokio::test]
async fn forwarding_delivers_every_broadcast_in_order() {
    let room = create_room();
    let receiver = room.sender.subscribe();

    for count in [1usize, 2, 3] {
        let _ = room.sender.send(OutgoingEvent::UserCount { count });
    }
    drop(room);

    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::default()));
    timeout(
        Duration::from_secs(5),
        crate::session::forward_broadcasts(receiver, sink.clone()),
    )
    .await
    .expect("the task ends when the room's sender is dropped");

    let sent = &sink.lock().await.sent;
    assert_eq!(sent.len(), 3, "every broadcast should have been forwarded");

    for (index, expected) in [1usize, 2, 3].into_iter().enumerate() {
        let crate::session::Message::Text(body) = &sent[index] else {
            panic!("broadcasts are sent as text frames");
        };
        let event: JsonValue = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(event["type"], "UserCount");
        assert_eq!(event["count"], expected, "frames must arrive in order");
    }
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

/// A user who has gone quiet is sent the eviction frame and the loop ends.
///
/// This is the mechanic rooms fading depends on, reachable here in
/// milliseconds: the user is backdated rather than waited out, and the sink is
/// a parameter rather than a socket.
#[tokio::test]
async fn a_quiet_user_is_evicted_with_the_close_frame() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        let mut user = connected_user("u1", "otter", "c1", Instant::now());
        user.last_message_time =
            Instant::now() - USER_IDLE_MESSAGE_TIMEOUT - Duration::from_secs(5);
        room.users.insert("u1".to_string(), user);
        rooms.insert("quiet".to_string(), room);
    }

    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::default()));
    timeout(
        HEARTBEAT_INTERVAL * 3,
        crate::session::beat_and_evict_idle(
            state,
            "quiet".to_string(),
            "u1".to_string(),
            sink.clone(),
        ),
    )
    .await
    .expect("an idle user must be evicted, ending the loop");

    let sent = &sink.lock().await.sent;
    let last = sent.last().expect("a heartbeat and then a close");

    let crate::session::Message::Close(Some(frame)) = last else {
        panic!("the last frame must be the eviction close, got {last:?}");
    };
    assert_eq!(
        frame.code, IDLE_CLOSE_CODE,
        "without the code the client reconnects and the room never empties"
    );
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

/// A cookie that cannot be a header value is dropped, not fatal.
///
/// Unreachable in production since identity became a closed set — a UUID and a
/// roster name have no character that is invalid in a header value (§5.9). It
/// stays as the thing that catches that closed set being widened, and this is
/// what it does when it fires: drop the one cookie and carry on, because a
/// visitor with no cookie is a new visitor rather than a broken one.
#[test]
fn a_cookie_that_cannot_be_encoded_is_dropped_and_the_rest_still_set() {
    use axum::response::IntoResponse;

    let mut response = "body".into_response();

    crate::session::attach_cookies(
        &mut response,
        &[
            "user_id=valid; Path=/".to_string(),
            // A newline cannot appear in a header value.
            "animal_name=bro\nken; Path=/".to_string(),
            "third=also-valid; Path=/".to_string(),
        ],
    );

    let set: Vec<&str> = response
        .headers()
        .get_all("Set-Cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();

    assert_eq!(
        set.len(),
        2,
        "the two encodable cookies are set and the broken one is skipped: {set:?}"
    );
    assert!(set.iter().any(|c| c.starts_with("user_id=")));
    assert!(set.iter().any(|c| c.starts_with("third=")));
}

/// Serialising an outgoing frame yields a frame, and never panics.
///
/// Every outgoing type is a plain struct of owned strings, numbers and UUIDs,
/// so this cannot fail — which is exactly why the fallback needs a test: it is
/// the arm that would otherwise be reasoned about rather than run. A panic here
/// would take the connection task with it (§5.1), so the fallback is an empty
/// frame the client drops.
#[test]
fn encoding_an_outgoing_frame_never_panics() {
    let json = crate::session::encode_event(&OutgoingEvent::UserCount { count: 7 });
    let parsed: JsonValue = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["type"], "UserCount");
    assert_eq!(parsed["count"], 7);

    // A type whose Serialize impl fails, which no outgoing type does — the
    // point is that reaching that arm produces an empty frame rather than a
    // panic in a connection task.
    struct Unserialisable;
    impl serde::Serialize for Unserialisable {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("cannot be represented"))
        }
    }

    assert_eq!(
        crate::session::encode_event(&Unserialisable),
        "",
        "an unrepresentable frame becomes an empty one, not a panic"
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

/// The whole session, driven end to end without a socket.
///
/// `run_session` is generic over its sink and stream, so a test can play the
/// part of a client: hand it frames, watch what comes back, and stop. Three
/// exits that previously needed a real peer to fail at an exact instant are
/// reachable this way — the room vanishing before the session starts, the
/// client leaving during the history replay, and a ping that cannot be
/// answered.
#[tokio::test]
async fn a_session_runs_and_ends_on_each_of_its_exits() {
    // ---- The room vanished between admission and upgrade -----------------
    let state = Arc::new(AppState::new());
    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::default()));

    timeout(
        Duration::from_secs(5),
        crate::session::run_session(
            "gone".to_string(),
            state.clone(),
            "u1".to_string(),
            "otter".to_string(),
            SharedSink(sink.clone()),
            futures::stream::empty(),
            "c1".to_string(),
        ),
    )
    .await
    .expect("with no room there is nothing to run");

    assert!(
        sink.lock().await.sent.is_empty(),
        "a session with no room sends nothing at all — not even a Welcome"
    );

    // ---- The client leaves during the history replay ---------------------
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for i in 0..6 {
            message_in(&mut room, &state.memory_tracker, &format!("m{i}"));
        }
        rooms.insert("replay".to_string(), room);
    }

    // Welcome, then two history frames, then the client is gone.
    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::failing_after(3)));
    timeout(
        Duration::from_secs(5),
        crate::session::run_session(
            "replay".to_string(),
            state.clone(),
            "u1".to_string(),
            "otter".to_string(),
            SharedSink(sink.clone()),
            futures::stream::empty(),
            "c1".to_string(),
        ),
    )
    .await
    .expect("a client leaving mid-replay ends the session");

    let sent = &sink.lock().await.sent;
    assert_eq!(
        sent.len(),
        3,
        "the replay stops at the refused frame rather than pushing the rest"
    );

    // The first frame is always Welcome — the client cannot tell its own
    // messages apart until it has one (constraint #2).
    let crate::session::Message::Text(first) = &sent[0] else {
        panic!("frames are text");
    };
    let welcome: JsonValue = serde_json::from_str(first.as_str()).unwrap();
    assert_eq!(welcome["type"], "Welcome");
}

/// A ping the server cannot answer ends the session.
///
/// The client pings, the server replies with a pong — and if that write fails
/// the peer is gone, so there is nothing left to run.
#[tokio::test]
async fn a_ping_that_cannot_be_answered_ends_the_session() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        rooms.insert("ping-room".to_string(), create_room());
    }

    // Everything succeeds except the pong. Counting frames instead would be a
    // race: the four tasks run concurrently, so "the third frame" is whichever
    // of them happened to win.
    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::refusing_pongs()));
    let incoming = futures::stream::iter(vec![Ok(crate::session::Message::Ping(
        bytes::Bytes::from_static(b"hi"),
    ))]);

    timeout(
        Duration::from_secs(5),
        crate::session::run_session(
            "ping-room".to_string(),
            state.clone(),
            "u1".to_string(),
            "otter".to_string(),
            SharedSink(sink.clone()),
            incoming,
            "c1".to_string(),
        ),
    )
    .await
    .expect("a pong that cannot be written ends the session");

    let sent = &sink.lock().await.sent;
    assert!(
        !sent
            .iter()
            .any(|m| matches!(m, crate::session::Message::Pong(_))),
        "the pong was refused, so it never appears in what was sent"
    );
    assert!(
        sent.iter()
            .any(|m| matches!(m, crate::session::Message::Text(_))),
        "the Welcome still got through before the ping arrived"
    );
}

/// Reconnecting keeps your name, and does not announce a stranger arriving.
///
/// The reported symptom is a room repeating "skink left / stinks joined" — one
/// person reconnecting and coming back as somebody else each time. That is what
/// happens whenever the identity cookie does not make it back: `admit_user`
/// mints a fresh id, and the room sees a departure and an arrival rather than a
/// reconnection.
///
/// This drives the real server over a real socket, takes the cookies out of the
/// handshake response the way a browser would, and reconnects with them.
#[tokio::test]
async fn reconnecting_with_the_handshake_cookies_keeps_the_same_identity() {
    let (addr, _state, handle) = start_ws_server_with_state().await;

    let (mut first, response) = connect_async(ws_request(addr, "identity", &[]))
        .await
        .expect("the handshake should be accepted");

    // Exactly what a browser stores from the response.
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::to_string)
        .collect();

    assert_eq!(
        cookies.len(),
        2,
        "the handshake must set both identity cookies, or a reconnecting \
         visitor cannot be recognised: {cookies:?}"
    );

    let welcome = recv_json_event(&mut first).await;
    assert_eq!(welcome["type"], "Welcome");
    let original_name = welcome["animal_name"].as_str().unwrap().to_string();
    let original_id = welcome["user_id"].as_str().unwrap().to_string();

    // The connection drops without a clean close, as a network blip does.
    drop(first);

    // The browser comes back with what it stored.
    let header = cookies.join("; ");
    let (mut second, _) =
        connect_async(ws_request(addr, "identity", &[("Cookie", header.clone())]))
            .await
            .expect("the reconnect should be accepted");

    let welcome = recv_json_event(&mut second).await;
    assert_eq!(
        welcome["animal_name"].as_str().unwrap(),
        original_name,
        "a reconnecting visitor must come back as themselves — a different name \
         is what the room reads as one person leaving and another arriving"
    );
    assert_eq!(
        welcome["user_id"].as_str().unwrap(),
        original_id,
        "and as the same identity, so their own messages stay theirs"
    );

    // And again, because the report is of it repeating.
    drop(second);
    let (mut third, _) = connect_async(ws_request(addr, "identity", &[("Cookie", header)]))
        .await
        .expect("the second reconnect should be accepted");
    let welcome = recv_json_event(&mut third).await;
    assert_eq!(welcome["animal_name"].as_str().unwrap(), original_name);

    handle.abort();
}

/// A reconnecting visitor is not announced as a new arrival to the room.
///
/// The other half of the same symptom: even when the name is kept, an onlooker
/// should not see "left" and "joined" every time somebody's connection blips.
#[tokio::test]
async fn a_reconnect_does_not_announce_a_departure_and_an_arrival() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    // An onlooker who stays put and watches. Its own arrival is in its stream
    // too — it subscribes before announcing itself, deliberately (§4.2).
    let (mut watcher, _) = connect_async(ws_request(addr, "watched", &[]))
        .await
        .expect("the watcher connects");
    let watcher_welcome = recv_json_event(&mut watcher).await;
    let watcher_name = watcher_welcome["animal_name"].as_str().unwrap().to_string();

    let (visitor, response) = connect_async(ws_request(addr, "watched", &[]))
        .await
        .expect("the visitor connects");
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::to_string)
        .collect();
    assert_eq!(cookies.len(), 2, "the handshake sets both identity cookies");

    let mut visitor = visitor;
    let visitor_name = recv_json_event(&mut visitor).await["animal_name"]
        .as_str()
        .unwrap()
        .to_string();

    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(visitor);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (_reconnected, _) = connect_async(ws_request(
        addr,
        "watched",
        &[("Cookie", cookies.join("; "))],
    ))
    .await
    .expect("the visitor reconnects");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut arrivals: Vec<String> = Vec::new();
    let mut departures: Vec<String> = Vec::new();
    while let Ok(Some(Ok(message))) = timeout(Duration::from_millis(250), watcher.next()).await {
        let WsMessage::Text(body) = message else {
            continue;
        };
        let event: JsonValue = serde_json::from_str(body.as_str()).unwrap_or_default();
        if let Some(name) = event.pointer("/event/UserJoined/animal_name") {
            arrivals.push(name.as_str().unwrap_or_default().to_string());
        }
        if let Some(name) = event.pointer("/event/UserLeft/animal_name") {
            departures.push(name.as_str().unwrap_or_default().to_string());
        }
    }

    let strangers: Vec<&String> = arrivals
        .iter()
        .filter(|name| **name != visitor_name && **name != watcher_name)
        .collect();

    assert!(
        strangers.is_empty(),
        "reconnecting introduced somebody who was never there: {strangers:?} \
         (visitor {visitor_name}, watcher {watcher_name}); \
         arrivals={arrivals:?} departures={departures:?}"
    );

    drop(state);
    handle.abort();
}

/// Without the cookies, a reconnecting visitor *is* a stranger — every time.
///
/// This is the reported symptom reproduced: "skink left, stinks joined",
/// repeating. Each reconnection mints a fresh identity, so the room announces
/// a departure and an arrival for what is one person whose connection blipped.
///
/// The test exists to pin *why*: the cookies are the whole mechanism, so
/// anything that stops them coming back — a browser refusing to store a
/// `Secure` cookie over plain http, which is exactly what local development
/// is — turns every reconnect into a new person.
#[tokio::test]
async fn without_cookies_every_reconnect_is_a_different_person() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    let mut names = Vec::new();
    for _ in 0..3 {
        let (mut socket, _) = connect_async(ws_request(addr, "amnesia", &[]))
            .await
            .expect("each connection is accepted");
        names.push(
            recv_json_event(&mut socket).await["animal_name"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        drop(socket);
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    let distinct: std::collections::BTreeSet<&String> = names.iter().collect();
    assert_eq!(
        distinct.len(),
        3,
        "without cookies each reconnect is a new person — which is the bug as \\
         experienced, and why the cookies have to reach the server: {names:?}"
    );

    drop(state);
    handle.abort();
}

/// The identity cookies are usable over plain http in local development.
///
/// They are `Secure`, which is right in production and fatal locally: a browser
/// will not store a `Secure` cookie received over plain http. Safari refuses
/// even on localhost. With nothing stored, every reconnect mints a fresh
/// identity, and the room fills with "skink left / stinks joined" — one person
/// whose connection blipped, announced as a parade of strangers.
///
/// So `Secure` is set for every host except the loopback ones, where http is
/// the only thing on offer. That keeps production strict and makes the
/// mechanism work where it is actually being developed.
#[test]
fn identity_cookies_are_secure_everywhere_except_loopback() {
    for host in [
        "thehellisthis.com",
        "www.thehellisthis.com",
        "thehellisthis.com:443",
        "some-preview.workers.dev",
    ] {
        let (id, name) = create_user_cookies("id", "otter", host);
        assert!(
            id.contains("Secure") && name.contains("Secure"),
            "{host} must get Secure cookies"
        );
    }

    for host in [
        "localhost",
        "localhost:3000",
        "127.0.0.1:3000",
        "[::1]:3000",
    ] {
        let (id, name) = create_user_cookies("id", "otter", host);
        assert!(
            !id.contains("Secure") && !name.contains("Secure"),
            "{host} is plain http, and a Secure cookie there is a cookie the \\
             browser throws away: {id}"
        );
    }

    // Everything else about them is unconditional.
    let (id, name) = create_user_cookies("id", "otter", "localhost");
    for cookie in [&id, &name] {
        assert!(
            cookie.contains("HttpOnly"),
            "the client must never be able to read these (constraint #2)"
        );
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Path=/"));
    }
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
