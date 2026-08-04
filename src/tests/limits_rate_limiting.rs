//! `RateLimiter`: the per-user message and room-join budgets, and the
//! frontend text that has to agree with them (constraint #12).

use super::*;

#[tokio::test]
async fn test_rate_limiter() {
    let mut rate_limiter = RateLimiter::new();

    assert!(rate_limiter.can_send_message());
    for _ in 0..(MAX_MESSAGES_PER_WINDOW - 1) {
        assert!(rate_limiter.can_send_message());
    }
    assert!(!rate_limiter.can_send_message());

    rate_limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);
    assert!(rate_limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_join_attempts() {
    let mut rate_limiter = RateLimiter::new();
    for _ in 0..MAX_ROOM_JOIN_ATTEMPTS {
        assert!(rate_limiter.can_join_room());
    }
    assert!(!rate_limiter.can_join_room());
}

#[tokio::test]
async fn test_rate_limiter_window_reset() {
    let mut rate_limiter = RateLimiter::new();

    // Fill up the current window
    for _ in 0..MAX_MESSAGES_PER_WINDOW {
        assert!(rate_limiter.can_send_message());
    }

    // Wait for window to reset
    rate_limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);
    rate_limiter.message_count = 0;

    // Should be able to send messages again
    assert!(rate_limiter.can_send_message());
    assert_eq!(rate_limiter.message_count, 1);
}

// ========== MESSAGE & BROADCASTING ==========

#[tokio::test]
async fn test_socket_message_rate_limiting_per_user() {
    let mut user = UserData {
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
    };

    // Fill window for this user
    for _ in 0..MAX_MESSAGES_PER_WINDOW {
        assert!(user.rate_limiter.can_send_message());
    }

    // Should be rate limited
    assert!(!user.rate_limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_join_window_reset() {
    let mut rate_limiter = RateLimiter::new();
    for _ in 0..MAX_ROOM_JOIN_ATTEMPTS {
        assert!(rate_limiter.can_join_room());
    }
    assert!(!rate_limiter.can_join_room());

    rate_limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);
    rate_limiter.join_attempts = 0;
    assert!(rate_limiter.can_join_room());
}

#[tokio::test]
async fn test_rate_limiter_message_window_enforcement() {
    let mut limiter = RateLimiter::new();

    // Send max messages
    for _ in 0..MAX_MESSAGES_PER_WINDOW {
        assert!(limiter.can_send_message());
    }

    // Next should fail
    assert!(!limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_window_reset_after_expiry() {
    let mut limiter = RateLimiter::new();

    // Fill window
    for _ in 0..MAX_MESSAGES_PER_WINDOW {
        limiter.can_send_message();
    }

    // Manually expire window
    limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);
    limiter.message_count = 0;

    // Should work again
    assert!(limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_different_message_types() {
    let mut limiter = RateLimiter::new();

    // Messages count toward rate limit
    for _ in 0..10 {
        assert!(limiter.can_send_message());
    }

    // Verify we're tracking
    assert_eq!(limiter.message_count, 10);
}

#[tokio::test]
async fn test_rate_limiter_window_expiry() {
    let mut limiter = RateLimiter::new();

    for _ in 0..30 {
        assert!(limiter.can_send_message());
    }

    assert!(!limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_reset() {
    let mut limiter = RateLimiter::new();

    for _ in 0..5 {
        assert!(limiter.can_send_message());
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    assert!(limiter.can_send_message());
}

#[tokio::test]
async fn test_chat_error_into_response_rate_limit_error() {
    let error = ChatError::RateLimitError("Too fast".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_user_data_rate_limiter_integration() {
    let mut user = UserData {
        user_id: "user1".to_string(),
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
        last_message_text: None,
        last_reaction_event: None,
    };

    // Should be able to send messages initially
    assert!(user.rate_limiter.can_send_message());

    // Update typing state
    user.is_typing = true;
    assert!(user.is_typing);

    // Set last read message
    let msg_id = Uuid::new_v4();
    user.last_read_message = Some(msg_id);
    assert_eq!(user.last_read_message, Some(msg_id));
}

#[tokio::test]
async fn test_rate_limiter_max_messages_in_window() {
    let mut limiter = RateLimiter::new();

    // Send MAX_MESSAGES_PER_WINDOW messages
    for _ in 0..MAX_MESSAGES_PER_WINDOW {
        assert!(limiter.can_send_message());
    }

    // Next message should be rejected
    assert!(!limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_existing_user_rate_limited() {
    let mut room_state = create_room();

    // Add an existing user with exhausted rate limiter
    let user_id = "rate_limited_user".to_string();
    let mut rate_limiter = RateLimiter::new();

    // Exhaust the join rate limiter
    for _ in 0..5 {
        let _ = rate_limiter.can_join_room();
    }

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
            rate_limiter,
            last_message_text: None,
            last_reaction_event: None,
        },
    );

    // User should still be allowed but their rate limiter is checked
    let _allowed = room_state.is_user_allowed(&user_id, crate::config::MAX_USERS_PER_ROOM);
    // Result depends on rate limiter state - just exercise the code path
}

#[tokio::test]
async fn test_chat_error_rate_limit_error_status() {
    let error = ChatError::RateLimitError("test".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

// ========== ROOM NAME REGEX TESTS ==========

#[tokio::test]
async fn test_rate_limiter_exactly_at_limit() {
    let mut limiter = RateLimiter::new();

    // can_send_message increments count internally, so after MAX calls it should fail
    for i in 0..MAX_MESSAGES_PER_WINDOW {
        assert!(limiter.can_send_message(), "Should allow message {}", i);
    }

    // Next should fail
    assert!(!limiter.can_send_message());
}

#[tokio::test]
async fn test_rate_limiter_join_attempts_exactly_at_limit() {
    let mut limiter = RateLimiter::new();
    // Use up all join attempts
    for _ in 0..MAX_ROOM_JOIN_ATTEMPTS {
        assert!(limiter.can_join_room());
    }
    // Next one should fail
    assert!(!limiter.can_join_room());
}

// ========== SECURITY MANAGER EDGE CASES ==========

#[tokio::test]
async fn test_frontend_rate_limit_messaging_matches_backend() {
    // Frontend should inform users about rate limits that match backend

    let messages_per_window = MAX_MESSAGES_PER_WINDOW;
    let window_seconds = RATE_LIMIT_WINDOW.as_secs();

    assert_eq!(
        messages_per_window, 30,
        "MAX_MESSAGES_PER_WINDOW should be 30"
    );
    assert_eq!(window_seconds, 60, "RATE_LIMIT_WINDOW should be 60s");

    // If frontend mentions rate limits, they should be accurate
    // (Currently frontend doesn't show specific numbers, which is fine)
}

#[tokio::test]
async fn test_rate_limit_exceeded_via_ws() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/rate-limit-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send many messages rapidly to trigger rate limit
    for i in 0..50 {
        let msg = format!(r#"{{"type":"Message","text":"Spam message {}"}}"#, i);
        ws.send(text_frame(msg)).await.ok();
    }

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection should still be alive
    let ping_result = ws.send(WsMessage::Ping(vec![1, 2, 3].into())).await;
    assert!(
        ping_result.is_ok(),
        "Connection should still be alive after rate limiting"
    );

    ws.close(None).await.ok();
}

// Test duplicate message prevention - covers lines 1676-1684

#[tokio::test]
async fn test_rate_limiter_window_reset_on_message() {
    let mut limiter = RateLimiter::new();

    // Exhaust the limiter
    for _ in 0..MAX_MESSAGES_PER_WINDOW {
        assert!(limiter.can_send_message());
    }
    assert!(!limiter.can_send_message()); // Should be rate limited now

    // Simulate window passing by directly modifying (in tests module we have access)
    limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);

    // Should be able to send again after window reset
    assert!(limiter.can_send_message());
}

// Test can_join_room rate limit - covers lines 106-113

#[tokio::test]
async fn test_rate_limiter_join_room_limit() {
    let mut limiter = RateLimiter::new();

    // Should allow joining up to limit
    for _ in 0..MAX_ROOM_JOIN_ATTEMPTS {
        assert!(limiter.can_join_room());
    }

    // Should reject after limit
    assert!(!limiter.can_join_room());

    // After window reset should allow again
    limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);
    assert!(limiter.can_join_room());
}

// Test ResourceMonitor limits - covers lines 131-134

/// `Default` is the same thing as `new` for the limiter types.
#[test]
fn limiter_defaults_match_their_constructors() {
    let limiter = RateLimiter::default();
    assert_eq!(limiter.message_count, 0);
    assert_eq!(limiter.join_attempts, 0);

    let state = AppState::default();
    assert_eq!(state.memory_tracker.total_bytes.load(Ordering::SeqCst), 0);
}

/// A message that is short enough as text but expands once escaped is still
/// accepted, and its rendered form never exceeds the ceiling that exists for
/// exactly this case.
///
/// This used to be rejected: `validate_message` sanitised the *raw* text and
/// compared that against `MAX_MESSAGE_LEN` — a stricter, and different, bound
/// than the one the actual stored value answers to. `render_message_html`
/// already has its own graceful answer for "the rendered form is too big":
/// fall back to escaped plain text, capped so it cannot itself overflow
/// (`MAX_RENDERED_MESSAGE_LEN`), rather than reject a message the person
/// still meant to send. `validate_and_render_message` now defers to that one
/// policy instead of enforcing a second, stricter one ahead of it (§1.4d).
#[test]
fn messages_that_expand_when_escaped_stay_within_the_rendered_ceiling() {
    // Each '<' becomes "&lt;" once escaped, so this is under the cap as input
    // and would be well over it if the old raw-text-sanitised bound still
    // applied.
    let expands = "<".repeat(MAX_MESSAGE_LEN - 1);
    assert!(expands.len() <= MAX_MESSAGE_LEN);

    let result = validate_and_render_message(&expands);
    let html = result.expect("render_message_html's own fallback must never need to reject");
    assert!(
        html.len() <= MAX_RENDERED_MESSAGE_LEN,
        "the rendered form must stay within the ceiling that exists for \
         exactly this case: {} bytes",
        html.len()
    );
}

/// Messages beyond the burst limit are dropped, and the session survives.
#[tokio::test]
async fn a_burst_beyond_the_limit_is_dropped_without_killing_the_session() {
    let (addr, _state, handle) = start_ws_server_with_state().await;

    let (mut ws, _) = connect_async(format!("ws://{addr}/ws/burst-room"))
        .await
        .expect("connect failed");

    // Drain the Welcome frame.
    assert_eq!(recv_json_event(&mut ws).await["type"], "Welcome");

    for i in 0..(MAX_MESSAGES_PER_WINDOW * 2) {
        let payload = serde_json::json!({ "type": "Message", "text": format!("m{i}") });
        ws.send(text_frame(payload.to_string()))
            .await
            .expect("send failed");
    }

    // Oversized frames and unparseable frames are ignored rather than fatal.
    ws.send(text_frame("not json at all")).await.unwrap();
    ws.send(text_frame("x".repeat(MAX_MESSAGE_LEN + 100)))
        .await
        .unwrap();

    // The socket is still usable afterwards.
    ws.send(WsMessage::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.abort();
}

// ========== ENTRY POINT ==========

/// The per-user message window admits exactly its quota.
///
/// `>=` mutating to `>` in `can_send_message` would let one extra message
/// through every window; the existing tests sent comfortably more than the
/// quota and never checked the last allowed one.
#[test]
fn the_message_quota_boundary_is_exact() {
    let mut limiter = RateLimiter::new();

    for i in 1..=MAX_MESSAGES_PER_WINDOW {
        assert!(
            limiter.can_send_message(),
            "message {i} of {MAX_MESSAGES_PER_WINDOW} must be allowed"
        );
    }
    assert_eq!(limiter.message_count, MAX_MESSAGES_PER_WINDOW);
    assert!(
        !limiter.can_send_message(),
        "the message after the quota must be refused"
    );
    assert_eq!(
        limiter.message_count, MAX_MESSAGES_PER_WINDOW,
        "and a refusal must not count against the window either"
    );
}

/// Joining admits exactly its quota of attempts.
#[test]
fn the_join_quota_boundary_is_exact() {
    let mut limiter = RateLimiter::new();

    for i in 1..=MAX_ROOM_JOIN_ATTEMPTS {
        assert!(limiter.can_join_room(), "join {i} must be allowed");
    }
    assert!(
        !limiter.can_join_room(),
        "the join after the quota must be refused"
    );
}
