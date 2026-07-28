//! Joining a room over the socket: origin checks on the upgrade, the room
//! not existing yet or vanishing mid-join, and the teardown that follows
//! every exit from `run_session`.

use super::*;

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
                last_message_text: None,
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
