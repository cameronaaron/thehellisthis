//! Client events on the wire: deserialising them, and the session-level
//! contracts around identity, idle eviction, and the four raced connection
//! tasks tearing down when the client is gone.

use super::*;

#[tokio::test]
async fn test_malformed_client_event() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/malformed-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Send invalid JSON
    ws.send(text_frame(r#"{"type":"InvalidType"}"#.to_string()))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send malformed JSON
    ws.send(text_frame(r#"{not valid json}"#.to_string()))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
}

#[tokio::test]
async fn test_client_event_message_deserialization() {
    let json = r#"{"type":"Message","text":"Hello world"}"#;
    let event: ClientEvent = serde_json::from_str(json).unwrap();
    match event {
        ClientEvent::Message { text, reply_to, .. } => {
            assert_eq!(text, "Hello world");
            assert!(reply_to.is_none());
        }
        _ => panic!("Expected Message event"),
    }
}

#[tokio::test]
async fn test_client_event_typing_deserialization() {
    let json = r#"{"type":"Typing","is_typing":true}"#;
    let event: ClientEvent = serde_json::from_str(json).unwrap();
    match event {
        ClientEvent::Typing { is_typing } => assert!(is_typing),
        _ => panic!("Expected Typing event"),
    }
}

#[tokio::test]
async fn test_client_event_read_receipt_deserialization() {
    let msg_id = Uuid::new_v4().to_string();
    let json = format!(r#"{{"type":"ReadReceipt","message_id":"{}"}}"#, msg_id);
    let event: ClientEvent = serde_json::from_str(&json).unwrap();
    match event {
        ClientEvent::ReadReceipt { message_id } => assert_eq!(message_id, msg_id),
        _ => panic!("Expected ReadReceipt event"),
    }
}

#[tokio::test]
async fn test_client_event_unknown_type_handling() {
    // Unknown event types should fail to deserialize
    let json = r#"{"type":"UnknownEvent","data":"test"}"#;
    let result: Result<ClientEvent, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "Unknown event type should fail to deserialize"
    );
}

#[tokio::test]
async fn test_client_event_missing_fields() {
    // Message without text field
    let json = r#"{"type":"Message"}"#;
    let result: Result<ClientEvent, _> = serde_json::from_str(json);
    assert!(result.is_err(), "Message without text should fail");

    // Typing without is_typing
    let json = r#"{"type":"Typing"}"#;
    let result: Result<ClientEvent, _> = serde_json::from_str(json);
    assert!(result.is_err(), "Typing without is_typing should fail");
}

#[tokio::test]
async fn test_client_event_message_with_reply() {
    let json = r#"{"type":"Message","text":"Hello","reply_to":{"message_id":"abc","author_name":"Lion","preview_text":"Original"}}"#;
    let event: ClientEvent = serde_json::from_str(json).unwrap();
    match event {
        ClientEvent::Message { text, reply_to, .. } => {
            assert_eq!(text, "Hello");
            assert!(reply_to.is_some());
            let r = reply_to.unwrap();
            assert_eq!(r.message_id, "abc");
            assert_eq!(r.author_name, "Lion");
            assert_eq!(r.preview_text, "Original");
        }
        _ => panic!("Expected Message event"),
    }
}

#[tokio::test]
async fn test_client_event_deserialization() {
    // Message event
    let json = r#"{"type":"Message","text":"hello"}"#;
    let event: ClientEvent = serde_json::from_str(json).unwrap();
    matches!(event, ClientEvent::Message { .. });

    // Typing event
    let json = r#"{"type":"Typing","is_typing":true}"#;
    let event: ClientEvent = serde_json::from_str(json).unwrap();
    matches!(event, ClientEvent::Typing { .. });

    // ReadReceipt event
    let msg_id = uuid::Uuid::new_v4();
    let json = format!(r#"{{"type":"ReadReceipt","message_id":"{}"}}"#, msg_id);
    let event: ClientEvent = serde_json::from_str(&json).unwrap();
    matches!(event, ClientEvent::ReadReceipt { .. });
}

// Test SystemEvent serialization - covers lines 182-215

/// The client must take its identity from the Welcome frame, never from
/// `document.cookie`.
///
/// These are two halves of one decision. The identity cookies are `HttpOnly`,
/// so a client that reads `document.cookie` silently fails to recognise its own
/// messages — every message it sends renders as somebody else's. Making the
/// cookies `HttpOnly` and teaching the client to read the Welcome frame is a
/// single change; this test is what keeps either half from being reverted
/// alone.
#[test]
fn client_takes_identity_from_welcome_frame_not_cookies() {
    assert!(
        !SHIPPED_CLIENT.contains("document.cookie.split"),
        "client must not parse document.cookie: the identity cookies are HttpOnly"
    );
    assert!(
        SHIPPED_CLIENT.contains("case 'Welcome':"),
        "client must handle the Welcome frame to learn its own identity"
    );

    let (user_cookie, animal_cookie) = create_user_cookies("some-id", "otter", "thehellisthis.com");
    assert!(user_cookie.contains("HttpOnly"));
    assert!(animal_cookie.contains("HttpOnly"));
}

// ========== CLIENT SCROLL / LAYOUT STABILITY ==========

/// §3.5 — every map keyed by a client address has an eviction path.
///
/// `ConnectionPool::ip_counters` had one. The two maps in `SecurityManager` did
/// not: an expired ban was *tested* for expiry on read but never removed, and a
/// suspicion record was never removed at all. Both are keyed by client address,
/// so both grew for the life of the process with one entry per address ever
/// seen — the unbounded-map shape §3.5 exists to forbid.
#[tokio::test]
async fn every_client_keyed_map_is_swept_by_housekeeping() {
    let state = Arc::new(AppState::new());
    let stale = Instant::now() - IP_BAN_DURATION - Duration::from_secs(60);

    for i in 0..500 {
        let ip = format!("addr-{i}");
        state
            .security_manager
            .banned_ips
            .write()
            .await
            .insert(ip.clone(), stale);
        state
            .security_manager
            .suspicious_activity
            .write()
            .await
            .insert(ip, (1, stale));
    }

    state.cleanup().await;

    assert!(
        state.security_manager.banned_ips.read().await.is_empty(),
        "expired bans must be evicted, not merely ignored on read"
    );
    assert!(
        state
            .security_manager
            .suspicious_activity
            .read()
            .await
            .is_empty(),
        "stale suspicion records must be evicted"
    );
}

/// Constraint #12 — the client and server agree on the idle close code.
///
/// The number is the entire mechanism. If the client's copy drifts it stops
/// recognising the eviction, reconnects into the room it was just removed from,
/// and rooms silently stop fading again — with no error anywhere, because
/// reconnecting is exactly the right response to every *other* close.
#[test]
fn client_and_server_agree_on_the_idle_close_code() {
    assert!(
        EMBEDDED_JS.contains(&format!("const IDLE_CLOSE_CODE = {IDLE_CLOSE_CODE};")),
        "client.js must declare IDLE_CLOSE_CODE = {IDLE_CLOSE_CODE} to match config.rs"
    );

    // 4000-4999 is the range the protocol reserves for the application; a code
    // outside it is either reserved or rejected by the browser.
    assert!(
        (4000..=4999).contains(&IDLE_CLOSE_CODE),
        "an application close code must be in 4000-4999, not {IDLE_CLOSE_CODE}"
    );
}

/// §7 — the client does not undo the eviction that lets a room empty.
///
/// The server removes a user who has said nothing for ten minutes so the room
/// can empty and eventually fade. That only works if the client stays away.
/// `onclose` took no argument at all, so it could not see the code, and
/// `tryReconnect` ran on every close — one tab left open kept a room alive for
/// the life of the process, and rooms never faded.
#[test]
fn the_client_does_not_reconnect_after_an_idle_eviction() {
    let js = EMBEDDED_JS;

    assert!(
        js.contains("this.ws.onclose = (e) => this.handleWebSocketClose(e);"),
        "the close handler must receive the event, or it cannot read the code"
    );
    assert!(
        js.contains("event.code === IDLE_CLOSE_CODE"),
        "the close handler must branch on the eviction code"
    );

    // The eviction branch must return before reaching the reconnect.
    let handler = js
        .split_once("handleWebSocketClose(event) {")
        .and_then(|(_, rest)| rest.split_once("\n    }"))
        .map(|(body, _)| body)
        .expect("client.js should define handleWebSocketClose");

    let eviction = handler
        .find("IDLE_CLOSE_CODE")
        .expect("the handler should check the eviction code");
    let reconnect = handler
        .find("this.tryReconnect()")
        .expect("the handler should still reconnect for ordinary closes");

    assert!(
        eviction < reconnect,
        "the eviction check must come before the reconnect, or the client \
         reconnects anyway"
    );
    assert!(
        js.contains("rejoinAfterIdle"),
        "an evicted user must still have a way back"
    );
}

/// The idle eviction actually fires, and closes with the code the client reads.
///
/// This is the branch that stopped rooms fading, and until now nothing executed
/// it — the fix was asserted through the client's handling of the code and the
/// server's constant, but never by watching the server send it. Backdating the
/// user's `last_message_time` reaches it in one heartbeat tick rather than the
/// ten minutes the constant describes.
#[tokio::test]
#[ignore = "waits for a real heartbeat interval; run via scripts/slow-tests.sh"]
async fn an_idle_user_is_evicted_with_the_close_code_the_client_expects() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    let (mut ws, _) = connect_async(ws_request(addr, "idle-room", &[]))
        .await
        .expect("the handshake should be accepted");
    assert_eq!(recv_json_event(&mut ws).await["type"], "Welcome");

    // Make this user look like somebody who has said nothing for a long time.
    {
        let mut rooms = state.rooms.write().await;
        let room = rooms.get_mut("idle-room").expect("the room exists");
        for user in room.users.values_mut() {
            user.last_message_time =
                Instant::now() - USER_IDLE_MESSAGE_TIMEOUT - Duration::from_secs(5);
        }
    }

    // The heartbeat task checks on its own interval; wait for one tick.
    let close = timeout(HEARTBEAT_INTERVAL * 3, async {
        loop {
            match ws.next().await {
                Some(Ok(WsMessage::Close(frame))) => return frame,
                Some(Ok(_)) => continue,
                other => panic!("socket ended without a close frame: {other:?}"),
            }
        }
    })
    .await
    .expect("an idle user should be evicted within a few heartbeats");

    let frame = close.expect("the eviction must carry a close frame, not a bare close");
    assert_eq!(
        u16::from(frame.code),
        IDLE_CLOSE_CODE,
        "the eviction must use the code the client checks for; without it the \
         client reconnects and the room never empties"
    );

    handle.abort();
}

/// The category a client is told is the coarse one, and never empty.
///
/// Found by mutation testing: replacing `public_message` with `""` or with a
/// nonsense string survived. Nothing asserted what a client is actually shown —
/// the error taxonomy was tested for its *status codes* only, so the body could
/// have said anything, or nothing, and the suite would have agreed.
#[test]
fn every_error_tells_the_client_a_usable_category() {
    let cases = [
        (ChatError::RoomFull, "Room is full"),
        (
            ChatError::RateLimitError("internal detail".into()),
            "Rate limit exceeded",
        ),
        (
            ChatError::InvalidMessage("internal detail".into()),
            "Invalid message",
        ),
        (
            ChatError::ResourceLimit("internal detail".into()),
            "Server at capacity",
        ),
        (
            ChatError::SecurityError("internal detail".into()),
            "Access denied",
        ),
    ];

    for (error, expected) in cases {
        let body = format!("{:?}", error.public_message());
        assert_eq!(
            error.public_message(),
            expected,
            "the category shown to a client changed: {body}"
        );
        assert!(
            !error.public_message().is_empty(),
            "an error with no message tells a client nothing"
        );
    }
}

/// Forwarding stops the moment the client stops accepting frames.
///
/// If it did not, a dead socket would keep a broadcast receiver subscribed and
/// the session would never tear down — the connection slot leak, arrived at
/// from the other direction.
#[tokio::test]
async fn forwarding_stops_when_the_client_is_gone() {
    let room = create_room();
    let receiver = room.sender.subscribe();

    for i in 0..3 {
        let _ = room.sender.send(OutgoingEvent::UserCount { count: i });
    }
    drop(room);

    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::failing_after(1)));
    timeout(
        Duration::from_secs(5),
        crate::session::forward_broadcasts(receiver, sink.clone()),
    )
    .await
    .expect("a failing sink must end the forward task, not hang it");

    assert_eq!(
        sink.lock().await.sent.len(),
        1,
        "forwarding must stop at the first refused frame, not keep trying"
    );
}

/// Pinging stops when the client stops accepting frames.
#[tokio::test]
async fn pinging_stops_when_the_client_is_gone() {
    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::failing_immediately()));

    timeout(
        HEARTBEAT_INTERVAL * 3,
        crate::session::send_pings(sink.clone()),
    )
    .await
    .expect("a failing sink must end the ping task rather than loop forever");

    assert!(
        sink.lock().await.sent.is_empty(),
        "the first ping failed, so nothing was accepted"
    );
}

/// A client that leaves during the history replay ends the session cleanly.
///
/// Ordinary, not exceptional: people open a room and close the tab. What
/// matters is that the replay stops at the refused frame rather than working
/// through five hundred more, and that the caller learns to tear down.
#[tokio::test]
async fn a_client_that_leaves_mid_replay_stops_the_replay() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    for i in 0..5 {
        message_in(&mut room, &tracker, &format!("m{i}"));
    }
    let history = room.history_for("viewer");

    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::failing_after(2)));
    let completed = crate::session::send_history(&history, &sink, "viewer").await;

    assert!(!completed, "the caller must learn the client is gone");
    assert_eq!(
        sink.lock().await.sent.len(),
        2,
        "the replay must stop at the refused frame, not push the rest"
    );
}

/// The heartbeat stops when the client stops accepting frames.
#[tokio::test]
async fn the_heartbeat_stops_when_the_client_is_gone() {
    let state = Arc::new(AppState::new());
    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::failing_immediately()));

    timeout(
        HEARTBEAT_INTERVAL * 3,
        crate::session::beat_and_evict_idle(
            state,
            "gone-room".to_string(),
            "u1".to_string(),
            sink.clone(),
        ),
    )
    .await
    .expect("a failing sink must end the heartbeat rather than loop forever");

    assert!(sink.lock().await.sent.is_empty());
}

/// Every kind of frame a client can send, and what the session does with it.
///
/// Driving `run_session` directly makes this deterministic. Through a real
/// socket these arms are a race between four concurrent tasks, so a test can
/// only hope to reach them; here the frames are simply handed over in order.
///
/// What matters is that none of them ends the session except the one that
/// should: a client sending nonsense, or something the server does not
/// understand, is a client that stays connected.
#[tokio::test]
async fn the_session_handles_every_kind_of_client_frame() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        rooms.insert("frames".to_string(), create_room());
    }

    let oversized = "x".repeat(MAX_PAYLOAD_SIZE + 1);
    let incoming = futures::stream::iter(vec![
        // A frame the server has no use for, which must not end the session.
        Ok(crate::session::Message::Binary(bytes::Bytes::from_static(
            b"\x00\x01\x02",
        ))),
        // The client answering our ping.
        Ok(crate::session::Message::Pong(bytes::Bytes::from_static(
            b"p",
        ))),
        // Larger than the payload ceiling: dropped without parsing.
        Ok(crate::session::Message::Text(oversized.into())),
        // Not JSON at all.
        Ok(crate::session::Message::Text("{not json".into())),
        // Valid JSON, but not an event this server knows.
        Ok(crate::session::Message::Text(
            r#"{"type":"Nonsense"}"#.into(),
        )),
        // A real message, to prove the session is still working after all that.
        Ok(crate::session::Message::Text(
            r#"{"type":"Message","text":"still here"}"#.into(),
        )),
        // And the client says goodbye.
        Ok(crate::session::Message::Close(None)),
    ]);

    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::default()));
    timeout(
        Duration::from_secs(5),
        crate::session::run_session(
            "frames".to_string(),
            state.clone(),
            "u1".to_string(),
            "otter".to_string(),
            SharedSink(sink.clone()),
            incoming,
            "c1".to_string(),
        ),
    )
    .await
    .expect("the close frame ends the session");

    // The one real message got through everything before it.
    let rooms = state.rooms.read().await;
    let history = &rooms.get("frames").unwrap().chat_history;
    assert_eq!(
        history.len(),
        1,
        "exactly the one valid message should have been stored; the oversized, \
         unparseable and unknown frames are dropped, not stored and not fatal"
    );
    assert!(history[0].text.contains("still here"));
}

/// The header keeps every element the client reaches for.
///
/// It was restructured into the iMessage shape — a quiet back link, the room's
/// name in the centre, round actions on the right — and a restructure is
/// exactly when an id gets dropped by accident. The inventory catches that in
/// general; this says *why* each of these has to survive, which the inventory
/// cannot.
#[test]
fn the_navigation_bar_keeps_what_the_client_drives() {
    for (id, purpose) in [
        ("roomName", "which room you are in"),
        ("userCountNum", "how many people are here"),
        ("roomLifespan", "how long the room has left (§7)"),
        (
            "statusText",
            "what the connection is doing when it is not fine",
        ),
        ("muteBtn", "the notification toggle"),
        ("exploreLink", "the way to another room"),
        ("userCount", "the control that opens the roster"),
    ] {
        assert!(
            EMBEDDED_HTML.contains(&format!("id=\"{id}\"")),
            "the header lost `{id}`, which is {purpose}"
        );
    }
}

/// The roster is asked for, not pushed.
///
/// Broadcasting the whole list to everyone whenever anybody arrives is
/// O(users) per recipient — quadratic in the size of the room, for a panel
/// almost nobody has open. The client asks on open and keeps it current from
/// the join and leave events it already receives.
#[test]
fn the_client_asks_for_the_roster_rather_than_being_sent_it() {
    assert!(
        EMBEDDED_JS.contains("'RequestRoster'") || EMBEDDED_JS.contains("\"RequestRoster\""),
        "the client must ask for the roster"
    );
    assert!(
        EMBEDDED_JS.contains("updateRosterFrom"),
        "and keep an open list current from the events it already gets, rather \
         than asking again"
    );

    // Asking happens on open, not on a timer — a poll would be the quadratic
    // broadcast with extra steps.
    assert!(
        !EMBEDDED_JS.contains("setInterval(() => this.requestRoster"),
        "the roster must not be polled"
    );
}
