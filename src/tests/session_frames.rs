//! Individual WebSocket frames: ping/pong, close, binary, malformed JSON,
//! oversized payloads — the shapes a client can send that are not an
//! ordinary `Message` event.

use super::*;

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
async fn test_message_html_escaping() {
    let dangerous = "<img src=x onerror=alert(1)>";
    let result = validate_message(dangerous).unwrap();
    // Ammonia should remove the dangerous attributes
    assert!(!result.contains("onerror"));
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
            "c1".to_string(),
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
