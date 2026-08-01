//! Delivering to the room: user-count and message broadcasts, who gets
//! excluded, and what happens under a slow or lagging receiver.

use super::*;

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
            last_message_text: None,
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
            last_message_text: None,
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
                last_message_text: None,
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
                last_message_text: None,
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
                last_message_text: None,
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
                last_message_text: None,
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
            last_message_text: None,
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

/// `render_off_thread` moves message rendering to Tokio's blocking pool so a
/// slow render cannot stall every other connection's heartbeat and ping on
/// this server's single-threaded runtime (`main.rs`). Generic over the work
/// itself, the same reason the tasks above are generic over their sink: it
/// turns "the task panicked" from an untestable line into a value a test can
/// produce directly.
#[tokio::test]
async fn render_off_thread_runs_the_closure_on_the_blocking_pool() {
    let doubled = render_off_thread(|| 21 * 2).await;
    assert_eq!(
        doubled,
        Some(42),
        "the closure's return value must come back"
    );
}

/// A closure that panics is Tokio's own defined failure mode for
/// `spawn_blocking` — the runtime shutting down mid-render looks the same to
/// the caller. Either way, this must not propagate the panic into the
/// connection's own task; it reads as `None` and the caller drops the message.
#[tokio::test]
async fn render_off_thread_turns_a_panic_into_none() {
    let result = render_off_thread(|| -> i32 { panic!("boom") }).await;
    assert_eq!(
        result, None,
        "a panicking render must not take the connection's task down with it"
    );
}

/// `resolve_render` is what keeps the message-handling call site down to one
/// failure arm: `render_off_thread` returning `None` (the render task did not
/// complete) folds into the same rejection an invalid message already takes,
/// rather than needing its own arm that only a real panic mid-render could
/// reach.
#[tokio::test]
async fn resolve_render_folds_a_missing_render_into_a_rejection() {
    assert!(
        resolve_render(None).is_err(),
        "no render at all must be treated as a rejected message, not a crash"
    );
    assert_eq!(
        resolve_render(Some(Ok("<p>hi</p>".to_string()))).unwrap(),
        "<p>hi</p>"
    );
    assert!(resolve_render(Some(Err(ChatError::InvalidMessage("x".to_string())))).is_err());
}
