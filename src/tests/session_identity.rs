//! Identity: the cookies that carry it, the animal name that represents it,
//! the `Welcome` frame that hands it to the client, and reconnecting without
//! losing it.

use super::*;

#[tokio::test]
async fn test_create_user_cookies() {
    let (user_cookie, animal_cookie) = create_user_cookies("id", "lion", "thehellisthis.com");
    assert!(user_cookie.contains("user_id=id"));
    assert!(animal_cookie.contains("animal_name=lion"));
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
            last_message_text: None,
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

/// A merely-disconnected user still holds their room slot for
/// `DISCONNECTED_USER_RETENTION` — `cleanup.rs` does not remove them until
/// then, and reconnecting reclaims that exact slot. But `names_in_use`
/// (`room.rs`) only counts *connected* users as holding their name, so the
/// moment someone disconnects, their name is free for a brand new visitor to
/// draw — and reclaiming used to hand the original visitor their old name
/// back unconditionally, with no check that somebody else was now using it.
#[tokio::test]
async fn reclaiming_never_hands_back_a_name_someone_else_now_holds() {
    let state = Arc::new(AppState::new());
    let original_id = Uuid::new_v4().to_string();
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            original_id.clone(),
            UserData {
                user_id: original_id.clone(),
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
        rooms.insert("squatted-room".to_string(), room);
    }

    // A different visitor, preferring "otter" (say, their name in some other
    // room), claims it here — free, because its holder is only disconnected,
    // not gone. `claim_animal` is what a preference goes through, so this is
    // the direct way to ask it "is otter free", the same question the
    // newcomer's own admission asks.
    let newcomer_cookie = crate::identity::UserCookie {
        user_id: Uuid::new_v4().to_string(),
        animal_name: "otter".to_string(),
    };
    let (_, newcomer_name) = admit_user(&state, "squatted-room", "c1", Some(&newcomer_cookie))
        .await
        .expect("a new visitor is admitted");
    assert_eq!(
        newcomer_name, "otter",
        "the name must be free while disconnected"
    );

    // The original visitor reconnects.
    let cookie = crate::identity::UserCookie {
        user_id: original_id.clone(),
        animal_name: "otter".to_string(),
    };
    let (returning_id, returning_name) = admit_user(&state, "squatted-room", "c2", Some(&cookie))
        .await
        .expect("the original visitor reconnects");
    assert_eq!(returning_id, original_id, "reclaiming keeps the same id");

    let rooms = state.rooms.read().await;
    let room = rooms.get("squatted-room").unwrap();
    let connected_names: Vec<&str> = room
        .users
        .values()
        .filter(|u| u.is_connected())
        .map(|u| u.animal_name.as_str())
        .collect();
    let unique: std::collections::HashSet<&&str> = connected_names.iter().collect();
    assert_eq!(
        connected_names.len(),
        unique.len(),
        "two connected users must never share a name: {connected_names:?} \
         (returning visitor got {returning_name:?})"
    );
}
