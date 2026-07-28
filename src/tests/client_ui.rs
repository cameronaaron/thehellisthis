//! The shipped client — the bytes a visitor actually receives.
//!
//! Every test here reads `index.html` or `client.js` rather than a source of
//! truth that should have produced them (§9.1).

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
async fn test_reserved_path_robots_txt() {
    let app = Router::new().route("/robots.txt", get(robots_txt_handler));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_message_with_inline_code() {
    let text = "Use `cargo test` to run tests";
    let result = validate_message(text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_robots_txt_endpoint() {
    let app = Router::new().route("/robots.txt", get(robots_txt_handler));

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
}

#[tokio::test]
async fn test_extract_client_ip_from_x_forwarded_for() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        "203.0.113.195, 70.41.3.18, 150.172.238.178"
            .parse()
            .unwrap(),
    );

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("203.0.113.195".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_from_x_real_ip() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "192.168.1.100".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("192.168.1.100".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_empty_x_forwarded_for() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    // Should fallback to None since no conn_info provided
    assert!(ip.is_none());
}

#[tokio::test]
async fn test_extract_client_ip_priority() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "10.0.0.1".parse().unwrap());
    headers.insert("x-real-ip", "10.0.0.2".parse().unwrap());

    // x-forwarded-for should take priority
    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("10.0.0.1".to_string()));
}

#[tokio::test]
async fn test_robots_txt_content() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/robots.txt", get(robots_txt_handler))
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

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body.contains("User-agent:"));
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
async fn test_extract_client_ip_with_multiple_x_forwarded_for() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        "10.0.0.1, 10.0.0.2, 10.0.0.3".parse().unwrap(),
    );

    let ip = extract_client_ip(&headers, None);

    // Should return the first IP
    assert_eq!(ip, Some("10.0.0.1".to_string()));
}

#[tokio::test]
async fn test_message_alignment_with_multiple_users() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/alignment-test", addr);

    // Connect user 1
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();

    // Get user 1's identity
    let mut user1_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            user1_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user1_id.is_empty() {
                break;
            }
        }
    }

    assert!(!user1_id.is_empty(), "User 1 should have a user_id");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect user 2
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Get user 2's identity
    let mut user2_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws2).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            user2_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user2_id.is_empty() {
                break;
            }
        }
    }

    assert!(!user2_id.is_empty(), "User 2 should have a user_id");
    assert_ne!(user1_id, user2_id, "Users should have different user_ids");

    // Drain events
    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(200), recv_json_event(&mut ws1)).await;
        let _ = timeout(Duration::from_millis(200), recv_json_event(&mut ws2)).await;
    }

    // User 1 sends message
    let msg1 = r#"{"type":"Message","text":"From user 1"}"#;
    ws1.send(text_frame(msg1.to_string())).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // User 2 sends message
    let msg2 = r#"{"type":"Message","text":"From user 2"}"#;
    ws2.send(text_frame(msg2.to_string())).await.unwrap();

    // Both users should receive both messages with correct user_ids
    let mut messages_received = 0;
    for _ in 0..4 {
        if let Ok(data) = timeout(Duration::from_secs(2), recv_json_event(&mut ws1)).await
            && data["type"] == "Message"
        {
            let received_user_id = data["message"]["user_id"].as_str().unwrap_or("");
            assert!(
                !received_user_id.is_empty()
                    && (received_user_id == user1_id || received_user_id == user2_id),
                "Message should have valid user_id, got: {}",
                received_user_id
            );
            messages_received += 1;
        }
    }

    assert!(
        messages_received >= 2,
        "Should have received at least 2 messages"
    );

    handle.abort();
}

#[tokio::test]
async fn test_message_contains_user_id_for_alignment() {
    // Outgoing messages MUST contain user_id for client-side alignment
    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "alignment-test-user".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Test message</p>".to_string(),
        timestamp: "1234567890".to_string(),
        reply_to: None,
        attachment: None,
    };

    // user_id must be present and non-empty
    assert!(
        !msg.user_id.is_empty(),
        "OutgoingMessage must have user_id for alignment"
    );
    assert_eq!(msg.user_id, "alignment-test-user");

    // Serialize to JSON and verify user_id is included
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("user_id"), "JSON must include user_id field");
    assert!(
        json.contains("alignment-test-user"),
        "JSON must include actual user_id value"
    );
}

#[tokio::test]
async fn test_message_alignment_after_reconnect() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/alignment-reconnect", addr);

    // First connection
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();

    let mut user_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
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

    // Drain events and send message
    for _ in 0..3 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
    }

    ws1.send(text_frame(
        r#"{"type":"Message","text":"Pre-reconnect message"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Wait for message to be stored
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(ws1);

    // Reconnect with new connection
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Should receive history with original user_id intact
    let mut found_historical_msg = false;
    for _ in 0..15 {
        if let Ok(val) = timeout(Duration::from_secs(2), recv_json_event(&mut ws2)).await
            && val.get("type") == Some(&JsonValue::String("Message".to_string()))
        {
            let msg_user_id = val["message"]["user_id"].as_str().unwrap_or("");
            let text = val["message"]["text"].as_str().unwrap_or("");
            if text.contains("Pre-reconnect") {
                assert_eq!(
                    msg_user_id, user_id,
                    "Historical message must preserve original user_id for alignment"
                );
                found_historical_msg = true;
                break;
            }
        }
    }

    assert!(
        found_historical_msg,
        "Should receive historical message with preserved user_id"
    );
    handle.abort();
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
async fn test_extract_client_ip_all_sources() {
    use axum::extract::ConnectInfo;
    use axum::http::HeaderMap;
    use std::net::SocketAddr;

    // Test X-Forwarded-For priority
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
    headers.insert("x-real-ip", "9.10.11.12".parse().unwrap());
    let conn_info = ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345)));

    let ip = extract_client_ip(&headers, Some(&conn_info));
    assert_eq!(
        ip,
        Some("1.2.3.4".to_string()),
        "Should prefer X-Forwarded-For"
    );

    // Test X-Real-IP when no X-Forwarded-For
    let mut headers2 = HeaderMap::new();
    headers2.insert("x-real-ip", "9.10.11.12".parse().unwrap());
    let ip2 = extract_client_ip(&headers2, Some(&conn_info));
    assert_eq!(ip2, Some("9.10.11.12".to_string()), "Should use X-Real-IP");

    // Test fallback to ConnectInfo
    let headers3 = HeaderMap::new();
    let ip3 = extract_client_ip(&headers3, Some(&conn_info));
    assert_eq!(
        ip3,
        Some("127.0.0.1".to_string()),
        "Should fall back to ConnectInfo"
    );

    // Test no IP available
    let ip4 = extract_client_ip(&headers3, None);
    assert!(
        ip4.is_none(),
        "Should return None when no IP source available"
    );
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
async fn test_extract_client_ip_x_forwarded_for() {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "192.168.1.1, 10.0.0.1".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("192.168.1.1".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_x_real_ip() {
    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "192.168.2.2".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("192.168.2.2".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_fallback_to_conn_info() {
    let headers = HeaderMap::new();

    // When no headers are present, it falls back to connection info
    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, None); // No connection info provided

    // With connection info
    let addr: SocketAddr = "192.168.3.3:12345".parse().unwrap();
    let conn_info = ConnectInfo(addr);
    let ip = extract_client_ip(&headers, Some(&conn_info));
    assert_eq!(ip, Some("192.168.3.3".to_string()));
}

// Test ConnectionPool cleanup_stale behavior - covers lines 430-432

#[tokio::test]
async fn test_robots_txt_handler_content() {
    use axum::response::IntoResponse;

    let response = robots_txt_handler().await.into_response();
    assert_eq!(response.status(), StatusCode::OK);

    // Check content type header
    let headers = response.headers();
    assert!(headers.get("content-type").is_some());
}

// Test health_handler returns valid JSON - covers lines 2203-2230

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

/// The chat column must not change its own alignment when it stops being empty.
///
/// The empty state used to be `#chat:empty { justify-content: center;
/// align-items: center }`. The instant the first history message arrived
/// `:empty` stopped matching and the entire column snapped from centred to
/// top-aligned — a visible reorientation on every page load of a room that had
/// any history. The placeholder is now an absolutely-positioned overlay, which
/// cannot affect the layout of the messages that replace it.
#[test]
fn empty_chat_placeholder_does_not_alter_container_layout() {
    // Scan selectors, not raw substrings: prose in a comment can mention the
    // old rule (this file's own comments do), and a test that a comment can
    // fail is a test that will be silenced rather than fixed.
    let opens_a_rule_on_the_container = SHIPPED_CLIENT
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("#chat:empty") && !line.contains("::") && line.ends_with('{'));

    assert!(
        !opens_a_rule_on_the_container,
        "#chat:empty must not restyle the container itself; \
         use the absolutely-positioned ::before/::after overlay"
    );
    assert!(
        SHIPPED_CLIENT.contains("#chat:empty::before"),
        "the empty-state placeholder must still exist"
    );
}

/// Every response carries the security headers, and the CSP forbids inline
/// script.
///
/// The server previously sent none at all. Cloudflare adds none of its own for
/// a Worker-proxied origin, so a page that renders user-submitted Markdown as
/// HTML was protected by nothing but browser defaults.
#[tokio::test]
async fn every_response_carries_security_headers() {
    for path in ["/main", "/health", "/app.js", "/robots.txt"] {
        let app = build_router(Arc::new(AppState::new()));
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        let headers = response.headers();
        for required in [
            "content-security-policy",
            "x-frame-options",
            "x-content-type-options",
            "referrer-policy",
            "permissions-policy",
            "strict-transport-security",
        ] {
            assert!(
                headers.contains_key(required),
                "{path} is missing the {required} header"
            );
        }

        let csp = headers
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();

        // The whole point of moving the client script out of the page.
        assert!(
            csp.contains("script-src 'self'") && !csp.contains("script-src 'self' 'unsafe-inline'"),
            "CSP must not permit inline script: {csp}"
        );
        assert!(
            csp.contains("frame-ancestors 'none'"),
            "CSP must forbid framing: {csp}"
        );
        assert!(
            csp.contains("object-src 'none'"),
            "CSP must forbid plugins: {csp}"
        );
    }
}

/// The client script must not be inline, or the CSP above cannot hold.
#[test]
fn client_script_is_external_so_csp_can_forbid_inline() {
    assert!(
        !EMBEDDED_HTML.contains("<script>"),
        "the page must not contain an inline script block"
    );
    assert!(
        EMBEDDED_HTML.contains("src=\"/app.js\""),
        "the page must load its script from /app.js"
    );
    assert!(
        !EMBEDDED_HTML.contains(" onclick=") && !EMBEDDED_HTML.contains(" onload="),
        "inline event handlers are inline script and would need 'unsafe-inline'"
    );
}

/// `/app.js` is content-addressed and cacheable, and revalidates cheaply.
#[tokio::test]
async fn app_js_is_immutably_cacheable_and_revalidates() {
    let app = build_router(Arc::new(AppState::new()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let cache_control = response
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_control.contains("immutable"),
        "the script URL carries a content hash, so it can be immutable: {cache_control}"
    );

    let etag = response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // A conditional request for the same content must be answered 304.
    let app = build_router(Arc::new(AppState::new()));
    let conditional = app
        .oneshot(
            Request::builder()
                .uri("/app.js")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
}

/// The page must reference the script with its content-hash version, so a
/// deploy can never leave a browser running the previous script against a new
/// server.
#[tokio::test]
async fn page_references_the_versioned_script_url() {
    let app = build_router(Arc::new(AppState::new()));
    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&body);

    assert!(
        body.contains("src=\"/app.js?v="),
        "the served page must stamp the script version"
    );
}

// ========== SECURITY: WEBSOCKET ORIGIN ==========

/// The client address must be read from the header Cloudflare actually sets.
///
/// The server read only `X-Forwarded-For`, which Cloudflare does not set for a
/// Worker-proxied container request. Every visitor therefore arrived as the
/// same address, which quietly turned MAX_CONCURRENT_CONNECTIONS_PER_IP into a
/// global limit of 3 rather than a per-IP one — a correctness bug in both
/// directions: real users blocked each other, and one abuser was never isolated.
#[test]
fn client_address_prefers_the_header_cloudflare_sets() {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
    headers.insert("x-forwarded-for", "198.51.100.1, 10.0.0.1".parse().unwrap());

    assert_eq!(
        extract_client_ip(&headers, None).as_deref(),
        Some("203.0.113.7"),
        "CF-Connecting-IP is the trustworthy one behind Cloudflare"
    );

    // Without it, the X-Forwarded-For chain is still honoured, client first.
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "198.51.100.1, 10.0.0.1".parse().unwrap());
    assert_eq!(
        extract_client_ip(&headers, None).as_deref(),
        Some("198.51.100.1")
    );
}

/// The Worker must forward the client address to the container.
///
/// The Rust side reading the right header only helps if the Worker passes it
/// on; these two halves are one behaviour and neither is useful alone.
#[test]
fn worker_forwards_the_client_address() {
    const WORKER: &str = include_str!("../../cloudflare/src/index.ts");

    assert!(
        WORKER.contains("CF-Connecting-IP"),
        "the worker must forward the client address to the container"
    );
    assert!(
        WORKER.contains("X-Forwarded-For"),
        "the worker should also set X-Forwarded-For for the fallback path"
    );
    assert!(
        WORKER.contains("caches.default"),
        "the immutable script should be served from the edge cache"
    );
}

// ========== COVERAGE: PREVIOUSLY UNEXERCISED BRANCHES ==========
//
// Each test here targets a branch the suite never reached. They are grouped
// because they were written together, from a coverage report, but each asserts
// a real behaviour rather than merely visiting a line.

/// Trimming rendered history must be driven by the DOM, not by a counter.
///
/// The counter this replaced was incremented for system messages too, but those
/// remove themselves after eight seconds without decrementing it. On a busy
/// room the count drifted well above the number of nodes actually present and
/// began deleting live chat messages that were nowhere near the limit —
/// messages silently vanishing from a conversation, with the counter wrong and
/// the page fine.
#[test]
fn rendered_history_is_trimmed_from_the_dom_not_a_counter() {
    assert!(
        !EMBEDDED_JS.contains("this.messageCount"),
        "a hand-maintained message counter drifts; count the DOM instead"
    );
    assert!(
        EMBEDDED_JS.contains("pruneRenderedMessages()"),
        "trimming must be driven by the number of rendered messages"
    );
    assert!(
        EMBEDDED_JS.contains("querySelectorAll('.message')"),
        "the prune must measure the DOM it is trimming"
    );
}

/// The page must not block pinch-zoom.
///
/// `user-scalable=no` / `maximum-scale=1` fails WCAG 1.4.4. The usual reason to
/// set it is stopping iOS zooming when an input is focused, which is handled
/// instead by giving the message input a 16px font size.
#[test]
fn the_page_does_not_block_pinch_zoom() {
    // Inspect the viewport tag itself, not the whole file: the comment above it
    // names the settings it is explaining, and a test a comment can fail is a
    // test that gets silenced rather than fixed.
    let viewport = EMBEDDED_HTML
        .lines()
        .find(|line| line.contains("name=\"viewport\""))
        .expect("the page must declare a viewport");

    assert!(
        !viewport.contains("user-scalable=no"),
        "blocking zoom fails WCAG 1.4.4: {viewport}"
    );
    assert!(
        !viewport.contains("maximum-scale=1"),
        "capping zoom fails WCAG 1.4.4: {viewport}"
    );
    assert!(
        EMBEDDED_HTML.contains("font-size: 16px; /* Prevents iOS zoom */"),
        "the 16px input font is what makes blocking zoom unnecessary"
    );
}

// ========== BOUNDARY CONDITIONS ==========
//
// Written to kill mutants that survived `cargo mutants`: each one flipped a
// comparison at a threshold and no existing test noticed. A limit that is off
// by one at exactly the limit is the kind of bug that only appears under the
// load it was meant to protect against.

/// Client addresses are stored as opaque digests, never in the clear.
///
/// The rate limiter, connection pool and ban list only ever compare addresses
/// for equality, so none of them needs the real value. Hashing at the boundary
/// means a memory dump of a running server yields no visitor addresses — which
/// matters for a site whose entire premise is that you get an animal name
/// instead of an account.
#[test]
fn client_addresses_are_hashed_not_stored_in_the_clear() {
    let address = "203.0.113.42";
    let digest = hash_client_address(address);

    assert_ne!(digest, address);
    assert!(
        !digest.contains("203") && !digest.contains("113"),
        "the digest must not carry the address it came from: {digest}"
    );

    // Stable within a process, so it works as a map key.
    assert_eq!(digest, hash_client_address(address));
    // Distinct addresses stay distinct.
    assert_ne!(digest, hash_client_address("203.0.113.43"));
}

/// The client makes no third-party requests, and the policy says so.
///
/// The page used to pull its typeface and icon font from Google, which meant
/// every visitor's browser announced their address and the room they were
/// opening to a third party — on a site whose entire premise is that you get an
/// animal name instead of an account. Icons are an inline SVG sprite and text
/// uses the system stack, so there is nothing left to fetch.
#[test]
fn the_client_makes_no_third_party_requests() {
    // Match on the URL as it would actually be *fetched* — inside a src/href
    // attribute or a url() — rather than anywhere in the file. The comments
    // explaining why these origins were removed necessarily name them, and a
    // test that a comment can fail is a test that gets silenced rather than
    // fixed.
    let fetched = |source: &str, origin: &str| {
        [
            format!("src=\"https://{origin}"),
            format!("href=\"https://{origin}"),
            format!("url(https://{origin}"),
            format!("//{origin}/"),
        ]
        .iter()
        .any(|pattern| source.contains(pattern.as_str()))
    };

    for (name, source) in [("index.html", EMBEDDED_HTML), ("client.js", EMBEDDED_JS)] {
        for origin in [
            "fonts.googleapis.com",
            "fonts.gstatic.com",
            "unpkg.com",
            "cdn.jsdelivr.net",
            "www.googletagmanager.com",
            "www.google-analytics.com",
        ] {
            assert!(
                !fetched(source, origin),
                "{name} must not fetch from the third-party origin {origin}"
            );
        }
    }

    // Icons are local sprite references, not a downloaded font.
    assert!(
        EMBEDDED_HTML.contains("<use href=\"#i-"),
        "icons should be inline sprite references"
    );
    assert!(
        !EMBEDDED_HTML.contains("class=\"material-icons-round\""),
        "the icon font should be gone entirely"
    );
}

/// With no third-party assets, the policy can forbid outside origins outright.
#[tokio::test]
async fn the_policy_permits_no_third_party_origins() {
    let app = build_router(Arc::new(AppState::new()));
    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let csp = response
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert!(
        !csp.contains("https://"),
        "the policy should name no external origin: {csp}"
    );
    assert!(
        csp.contains("font-src 'none'"),
        "nothing should be loadable as a font: {csp}"
    );
}

// ========== INVARIANT SWEEPS ==========
//
// Each test here pins a law from ENGINEERING-STANDARDS.md as a property of the
// whole server rather than of one call site, so the *next* violation of the
// same class fails here instead of reaching production.

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

/// Every element the client asks for by id exists in the page it ships with.
///
/// The client is one HTML file and one JS file with no build step and no type
/// checker, so a `getElementById` for an id that is not in the markup is not a
/// compile error — it is `null`, and the first method that touches it throws at
/// runtime, usually somewhere far from the typo. Nothing else in the pipeline
/// would notice, which is exactly why this belongs in the gate (§9.1: verify
/// against the artifact, not the source that should have produced it).
#[test]
fn every_element_the_client_looks_up_exists_in_the_page() {
    let mut missing: Vec<&str> = Vec::new();

    for (index, _) in EMBEDDED_JS.match_indices("getElementById('") {
        let rest = &EMBEDDED_JS[index + "getElementById('".len()..];
        let Some(end) = rest.find('\'') else { continue };
        let id = &rest[..end];

        // Ids are quoted in the markup, so this cannot match a prefix of a
        // longer id the way a bare substring search would.
        if !EMBEDDED_HTML.contains(&format!("id=\"{id}\"")) {
            missing.push(id);
        }
    }

    assert!(
        missing.is_empty(),
        "client.js looks up ids that index.html does not define: {missing:?}"
    );
}

/// Constraint #12 — the client re-encodes to the ceiling the server enforces.
///
/// The client shrinks an image until it fits `MAX_ATTACHMENT_BYTES`. If its
/// copy of that number is larger than the server's, every photograph is
/// rejected *after* the user has waited for it to encode; if smaller, images
/// are needlessly degraded. Neither shows up as an error anywhere.
#[test]
fn client_attachment_ceiling_matches_the_server() {
    assert!(
        EMBEDDED_JS.contains(&format!(
            "const MAX_ATTACHMENT_BYTES = {MAX_ATTACHMENT_BYTES};"
        )),
        "client.js must declare MAX_ATTACHMENT_BYTES = {MAX_ATTACHMENT_BYTES} to match config.rs"
    );
}

/// Constraint #12 — every reaction the client offers, the server accepts.
///
/// Compared as sets: the server keeps its roster sorted by code point because
/// `is_reaction_emoji` binary-searches it, and that is not an order to show
/// anybody. A reaction button the server rejects is a button that silently does
/// nothing, which is the drift this catches.
#[test]
fn client_reaction_roster_matches_the_server() {
    let list = EMBEDDED_JS
        .split_once("const REACTION_EMOJI = [")
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(list, _)| list)
        .expect("client.js should declare a REACTION_EMOJI list");

    let client: std::collections::HashSet<&str> = list
        .split(',')
        .map(|entry| entry.trim().trim_matches('\'').trim())
        .filter(|entry| !entry.is_empty())
        .collect();

    let server: std::collections::HashSet<&str> = REACTION_EMOJI.iter().copied().collect();

    assert_eq!(
        client, server,
        "the client's reaction roster must be exactly the server's"
    );

    // The one-click bar is a subset of the same set, so no quick reaction can
    // be one the server refuses.
    let quick = EMBEDDED_JS
        .split_once("const QUICK_REACTIONS = [")
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(list, _)| list)
        .expect("client.js should declare QUICK_REACTIONS");

    for emoji in quick
        .split(',')
        .map(|e| e.trim().trim_matches('\'').trim())
        .filter(|e| !e.is_empty())
    {
        assert!(
            is_reaction_emoji(emoji),
            "quick reaction {emoji:?} is not on the server's roster"
        );
    }
}

/// §5.7 / constraint #13 — the new UI adds no inline handler and no outside
/// origin.
///
/// The emoji picker, the lightbox and the drag-and-drop overlay are all new
/// interactive surfaces, and every one of them is the sort of thing that
/// usually arrives with an `onclick=` or a CDN icon set. Either would quietly
/// undo the CSP that the whole message pipeline leans on.
#[test]
fn the_new_client_surfaces_keep_the_policy_intact() {
    for handler in [
        "onclick=",
        "onload=",
        "onchange=",
        "oninput=",
        "ondrop=",
        "ondragover=",
        "onkeydown=",
    ] {
        assert!(
            !EMBEDDED_HTML.contains(handler),
            "index.html must not carry the inline handler {handler}"
        );
    }

    assert!(
        !EMBEDDED_HTML.contains("<script>"),
        "the page must have no inline script block"
    );

    // The picker is emoji from the system font, not an icon set from a CDN.
    for origin in [
        "cdn.",
        "unpkg.com",
        "googleapis.com",
        "gstatic.com",
        "jsdelivr",
    ] {
        assert!(
            !EMBEDDED_JS.contains(origin),
            "client.js must not reference the third-party origin {origin}"
        );
    }
}

/// §5.7 — every font the page names is one the machine already has.
///
/// Removing the webfonts left two declarations behind that referred to fonts
/// nobody downloads any more. One was cosmetic: `'Roboto Mono', monospace` had
/// been quietly falling back to the generic for months. The other was not —
/// `#chat:empty::before` set `content: 'chat_bubble'` in `'Material Icons
/// Round'`, a *ligature*, so with the font gone every visitor who opened an
/// empty room was shown the literal word "chat_bubble" at 80px.
///
/// That is the trap in deleting a dependency: the code that referenced it still
/// parses, still applies, and fails only in the rendering — which no test that
/// checks for network requests can see. This checks the other half: that no
/// declaration names a family the browser cannot possibly have.
#[test]
fn the_page_names_no_font_it_does_not_ship_with() {
    // Families a browser has without downloading anything: the generic
    // keywords, the system-UI aliases, and the handful of faces that ship with
    // desktop and mobile operating systems.
    const AVAILABLE: &[&str] = &[
        "-apple-system",
        "arial",
        "blinkmacsystemfont",
        "consolas",
        "cursive",
        "fantasy",
        "helvetica",
        "helvetica neue",
        "inherit",
        "liberation mono",
        "menlo",
        "monospace",
        "sans-serif",
        "serif",
        "sf mono",
        "sfmono-regular",
        "segoe ui",
        "system-ui",
        "ui-monospace",
        "ui-sans-serif",
    ];

    let mut offenders: Vec<String> = Vec::new();

    let css = embedded_html_without_comments();

    for (index, _) in css.match_indices("font-family:") {
        let rest = &css[index + "font-family:".len()..];
        let Some(end) = rest.find(';') else { continue };
        let value = &rest[..end];

        // A custom property is checked where it is defined, not where used.
        if value.contains("var(--font-") {
            continue;
        }

        for family in value.split(',') {
            let name = family.trim().trim_matches('\'').trim_matches('"').trim();
            if name.is_empty() {
                continue;
            }
            if !AVAILABLE.contains(&name.to_ascii_lowercase().as_str()) {
                offenders.push(name.to_string());
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "index.html names fonts that are never downloaded and will not resolve: \
         {offenders:?}"
    );
}

/// No `content:` string is a leftover icon-font ligature.
///
/// The bug above rendered as text because a ligature name *is* text: with the
/// font present it draws a picture, and without it the browser shows the word.
/// Icons are inline SVG here, so any `content` holding a bare identifier-like
/// word is that mistake coming back.
#[test]
fn no_css_content_string_is_an_icon_ligature() {
    let mut suspicious: Vec<String> = Vec::new();

    let css = embedded_html_without_comments();

    for (index, _) in css.match_indices("content: '") {
        let rest = &css[index + "content: '".len()..];
        let Some(end) = rest.find('\'') else { continue };
        let value = &rest[..end];

        // Ligature names are lowercase identifiers with underscores and no
        // spaces — `chat_bubble`, `arrow_downward`. Real prose has spaces, and
        // decorative content is punctuation or empty.
        let looks_like_a_ligature = !value.is_empty()
            && value.contains('_')
            && value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit());

        if looks_like_a_ligature {
            suspicious.push(value.to_string());
        }
    }

    assert!(
        suspicious.is_empty(),
        "these CSS `content` strings look like icon-font ligatures, which render \
         as literal words once the font is gone: {suspicious:?}"
    );
}

// ========== RATCHETS: THINGS THAT MUST NOT GET WORSE ==========
//
// A coverage floor is a ratchet on how much is tested. These are ratchets on
// the properties that coverage cannot see: that the memory budget still closes,
// that the join path still shares instead of copying, and that per-message work
// is still independent of how much history there is.
//
// None of them assert a wall-clock threshold. The registry rules those out as
// flaky (§6.4, and the "injectable clock" entry), and a number tuned to this
// machine says nothing about CI. Each of these is either arithmetic over the
// constants or a structural fact, so it fails for a reason rather than for a
// bad afternoon on a shared runner.

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

/// The reaction bar survives the pointer travelling to it.
///
/// It is `position: fixed` and a sibling of `#chat`, so moving towards it
/// *leaves* the chat. The first version hid the bar on that event, which meant
/// the buttons appeared and vanished the instant you aimed at them — visible,
/// documented, and completely unclickable with a mouse.
///
/// Three things make it reachable, and all three are asserted because any one
/// of them silently restores the bug:
///   1. Hiding is delayed, not immediate.
///   2. Entering the bar cancels the pending hide.
///   3. Leaving the chat *onto the bar* is not treated as leaving.
#[test]
fn the_reaction_bar_can_actually_be_clicked() {
    let js = EMBEDDED_JS;

    assert!(
        js.contains("const REACTION_BAR_GRACE_MS"),
        "hiding the bar must be delayed; an immediate hide is unreachable by a \
         pointer that has to travel to it"
    );
    assert!(
        js.contains("scheduleReactionBarHide") && js.contains("cancelReactionBarHide"),
        "the delayed hide must be cancellable, or the delay only postpones the bug"
    );
    assert!(
        js.contains("this.reactionBar.addEventListener('pointerenter'"),
        "arriving on the bar must cancel the hide"
    );
    assert!(
        js.contains("this.reactionBar.contains(e.relatedTarget)"),
        "leaving the chat onto the bar is not leaving"
    );

    // The chat's own pointerleave must not hide the bar outright.
    let handler = js
        .split_once("this.chat.addEventListener('pointerleave'")
        .and_then(|(_, rest)| rest.split_once("});"))
        .map(|(body, _)| body)
        .expect("client.js should handle pointerleave on the chat");

    assert!(
        !handler.contains("this.hideReactionBar()"),
        "leaving the chat must schedule the hide, not perform it — performing \
         it is what made the buttons unclickable"
    );

    // Rebuilding the bar under the cursor replaced the button between
    // pointerdown and pointerup, so the click landed on nothing.
    assert!(
        js.contains("this.reactionBarFor === messageId"),
        "the bar must not be rebuilt while it is already showing for the same \
         message"
    );
}

/// §10.3 — the conversation is a column, not the whole window.
///
/// `#chat` spanned the full width with bubbles capped at 600px, so on a wide
/// monitor one message sat against the left edge and the next against the
/// right, with a metre of nothing between them. Reading it meant tracking
/// across the screen line by line.
#[test]
fn the_conversation_is_a_readable_centred_column() {
    let css = embedded_html_without_comments();

    assert!(
        css.contains("--conversation-width"),
        "the column's width should be one named value, not repeated literals"
    );

    // The composer and the messages must share it, or the input floats free of
    // the conversation it belongs to.
    for surface in ["#chat", ".input-container"] {
        let block = css
            .split_once(&format!("\n{surface} {{"))
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(body, _)| body.to_string())
            .unwrap_or_default();
        assert!(
            block.contains("--conversation-width"),
            "{surface} must be laid out against the conversation column, or it \
             drifts away from the messages"
        );
    }
}

/// No selector is styled in two places.
///
/// A second rule for the same selector wins by being later, which is invisible
/// in either block: the first says one thing, the second quietly overrides it,
/// and the file reads as though both apply. `#chat`, `.message`,
/// `.input-container` and `.reaction-bar` all ended up defined twice while the
/// layout was being reworked, and "weirdly proportioned" is exactly what that
/// produces — a value fixed in one place and undone thirteen hundred lines
/// later.
#[test]
fn no_css_selector_is_defined_twice() {
    let css = embedded_html_without_comments();
    let Some(start) = css.find("<style>") else {
        panic!("the page should carry its stylesheet");
    };
    let sheet = &css[start..];

    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

    for line in sheet.lines() {
        // Top-level rules only: nested at-rule bodies are indented, and a
        // selector may legitimately repeat inside a different media query.
        if line.starts_with(char::is_whitespace) || !line.ends_with(" {") {
            continue;
        }
        let selector = line.trim_end_matches(" {").trim();
        if selector.is_empty()
            || selector.starts_with('@')
            || selector.starts_with(':')
            || selector.contains(',')
        {
            continue;
        }
        *counts.entry(selector).or_default() += 1;
    }

    let mut duplicated: Vec<(&str, usize)> = counts.into_iter().filter(|(_, n)| *n > 1).collect();
    duplicated.sort();

    assert!(
        duplicated.is_empty(),
        "these selectors are styled in more than one place, so one block \
         silently overrides the other: {duplicated:?}"
    );
}

/// Every class the page uses has a style — in the markup and in the renderer.
///
/// A class with no rule renders as an unstyled box: no error, no warning,
/// nothing in the console. It simply looks wrong. Both halves have now failed
/// this way during one stylesheet rework:
///
///   - `.message-image-faded`, set by the renderer, lost its box when a sweep
///     over `.message*` matched it too (`\b` matches at a hyphen).
///   - `.reply-preview`, in the markup, lost the base rule carrying
///     `display: none`, leaving only its `.active` variant — so the
///     placeholder text sat above the composer permanently, which is what a
///     reader actually reported.
#[test]
fn every_class_the_client_renders_is_styled() {
    let css = embedded_html_without_comments();
    let js = EMBEDDED_JS;

    let mut rendered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Classes in the page's own markup, too. Checking only the ones JavaScript
    // sets is how `.reply-preview` lost its base rule unnoticed: the bulk sweep
    // that removed the old `.message*` styles took it with it, leaving only
    // `.reply-preview.active { display: flex }` — so with nothing hiding it,
    // the placeholder text in the markup sat above the composer permanently.
    for (index, _) in EMBEDDED_HTML.match_indices("class=\"") {
        let rest = &EMBEDDED_HTML[index + "class=\"".len()..];
        let Some(end) = rest.find('"') else { continue };
        for word in rest[..end].split_whitespace() {
            if word.starts_with(|c: char| c.is_ascii_lowercase()) {
                rendered.insert(word.to_string());
            }
        }
    }

    // `className = 'a b'` and `` className = `a ${x} b` `` both appear. A
    // static word immediately followed by a token that is *purely* `${…}` —
    // nothing glued before or after it — marks that word as a base class
    // combined with an independently varying modifier: `` `system-message
    // ${type}` `` means every render is "system-message" plus whichever of
    // `type`'s values won, never "system-message" with nothing else. That
    // word needs a rule that names it *alone*, not merely a rule that
    // mentions it — see `bare_base` below.
    let mut bare_base: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for marker in ["className = '", "className = `"] {
        for (index, _) in js.match_indices(marker) {
            let rest = &js[index + marker.len()..];
            let end = rest.find(['\'', '`']).unwrap_or(0);
            let words: Vec<&str> = rest[..end].split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                let word = word.trim();
                if word.starts_with(|c: char| c.is_ascii_lowercase()) && !word.contains('$') {
                    rendered.insert(word.to_string());
                    let next_is_bare_placeholder = words
                        .get(i + 1)
                        .is_some_and(|w| w.starts_with("${") && w.ends_with('}'));
                    if next_is_bare_placeholder {
                        bare_base.insert(word.to_string());
                    }
                }
            }
        }
    }

    assert!(
        rendered.len() > 15,
        "expected to find the renderer's classes; found {rendered:?}"
    );

    // Every class token that appears in *any* selector, not "does the
    // selector's raw text contain this substring" — `.icon-sprite` used to
    // make a plain `css.contains(".icon")` check pass for `.icon` too.
    let styled: std::collections::HashSet<String> = css_rule_selectors(&css)
        .iter()
        .flat_map(|s| classes_in_selector(s))
        .collect();

    // `bare_base` needs more than mention: `.system-message.error {}` mentions
    // `system-message` as a token, same as a real base rule would, but only
    // ever matches an element that *also* carries `error` — it does not style
    // `system-message` on its own. `addSystemMessage`'s default `type =
    // 'info'` case rendered with nothing applied at all — no padding, no
    // radius, no centring — because no selector was ever just `.system-message`.
    let bareless: Vec<&String> = bare_base
        .iter()
        .filter(|class| {
            !css_rule_selectors(&css)
                .iter()
                .any(|s| classes_in_selector(s) == [(*class).clone()])
        })
        .collect();

    assert!(
        bareless.is_empty(),
        "these are rendered alone, with no other literal class glued to them \
         — `` `{{class}} ${{var}}` `` — so each needs a rule that is just \
         `.{{class}}`, not merely a rule that mentions it alongside a \
         modifier: {bareless:?}"
    );

    let unstyled: Vec<&String> = rendered
        .iter()
        .filter(|class| !styled.contains(class.as_str()))
        .collect();

    assert!(
        unstyled.is_empty(),
        "the client renders these classes and nothing styles them, so they \
         appear unstyled with no error anywhere: {unstyled:?}"
    );
}

/// A rule that *shows* something implies a rule that hides it.
///
/// `.reply-preview.active { display: flex }` says "visible when active", which
/// only means anything if the base `.reply-preview` is hidden. The base rule
/// was removed by a bulk sweep and nothing noticed: the class was still
/// mentioned all over the stylesheet, the page still parsed, and the reply
/// preview — with the placeholder text baked into its markup — simply sat above
/// the composer forever, reading "Replying to / Message text…" to every
/// visitor.
///
/// This is the §10.3 rule stated the other way round. That one says a
/// sometimes-present thing must not change the layout; this says a
/// sometimes-*visible* thing must actually start invisible.
#[test]
fn every_conditional_display_rule_has_a_base_that_hides_it() {
    let css = embedded_html_without_comments();

    // Rules of the shape `.thing.state { display: … }`, where the state is a
    // class the client toggles.
    let mut missing: Vec<String> = Vec::new();

    for (index, _) in css.match_indices(".active {") {
        let head = &css[..index];
        let Some(line_start) = head.rfind('\n') else {
            continue;
        };
        let selector = css[line_start + 1..index].trim();

        // Only simple `.base.active` selectors; a descendant selector is a
        // different shape and hides for different reasons.
        if !selector.starts_with('.') || selector.contains(' ') || selector.contains(',') {
            continue;
        }

        // The declaration must actually be about visibility.
        let body_end = css[index..].find('}').map_or(index, |e| index + e);
        if !css[index..body_end].contains("display:") {
            continue;
        }

        let base = selector.to_string();
        let base_rule = css
            .split_once(&format!("\n{base} {{"))
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(body, _)| body.to_string());

        match base_rule {
            Some(body) if body.contains("display: none") => {}
            _ => missing.push(base),
        }
    }

    assert!(
        missing.is_empty(),
        "these have a rule making them visible in one state but no base rule \
         hiding them, so they are visible in every state: {missing:?}"
    );
}

/// Nothing the shipped client is made of has quietly disappeared.
///
/// This is the contract the other client sweeps kept turning out to need. Each
/// of them catches one *kind* of loss after it has happened once —
/// `.message-image-faded` losing its box, `.reply-preview` losing the rule that
/// hid it, `.reaction-bar::before` losing the bridge that made a hover target
/// reachable. All three came from the same bulk edit over the stylesheet, none
/// was caught by the gate, and a visitor reported one of them.
///
/// The common factor is not any of those classes. It is that **nothing knew
/// those rules were supposed to be there.** A regex over a stylesheet does not
/// know what a rule is for, and neither does a reviewer reading a 4000-line
/// diff.
///
/// So the page's parts are written down. `scripts/client-inventory.toml` lists
/// every element id, every stylesheet rule and every client method that exists
/// today; this fails when one of them stops existing. Adding things is free —
/// only removal is a decision, and making it a decision is the whole point.
///
/// **When this fails, answer the question rather than silencing it.** If the
/// removal was an accident, restore the rule. If it was deliberate, delete the
/// line from the manifest in the same commit. `scripts/client-inventory.sh`
/// prints both sides of the difference.
#[test]
fn the_shipped_client_still_contains_everything_it_did() {
    const MANIFEST: &str = include_str!("../../scripts/client-inventory.toml");

    /// The entries of one `name = [ … ]` list in the manifest.
    fn listed(manifest: &str, name: &str) -> Vec<String> {
        let body = manifest
            .split_once(&format!("{name} = ["))
            .and_then(|(_, rest)| rest.split_once("\n]"))
            .map(|(body, _)| body)
            .unwrap_or_else(|| panic!("the manifest should define `{name}`"));

        body.lines()
            .map(str::trim)
            .filter(|line| line.starts_with('\'') || line.starts_with('"'))
            .map(|line| {
                let quote = line.chars().next().expect("just checked it is quoted");
                line.trim_start_matches(quote)
                    .rsplit_once(quote)
                    .map(|(value, _)| value.to_string())
                    .unwrap_or_default()
            })
            .collect()
    }

    let css = embedded_html_without_comments();
    let sheet = css
        .split_once("<style>")
        .map(|(_, rest)| rest)
        .expect("the page carries its stylesheet");

    let present_selectors: std::collections::HashSet<&str> = sheet
        .lines()
        .filter(|line| line.ends_with(" {") && !line.starts_with([' ', '\t', '@']))
        .map(|line| line.trim_end_matches(" {").trim())
        .collect();

    let mut gone: Vec<String> = Vec::new();

    for id in listed(MANIFEST, "ids") {
        if !EMBEDDED_HTML.contains(&format!("id=\"{id}\"")) {
            gone.push(format!("element id: {id}"));
        }
    }

    for selector in listed(MANIFEST, "selectors") {
        if !present_selectors.contains(selector.as_str()) {
            gone.push(format!("stylesheet rule: {selector}"));
        }
    }

    for method in listed(MANIFEST, "methods") {
        if !EMBEDDED_JS.contains(&format!("\n    {method}(")) {
            gone.push(format!("client method: {method}"));
        }
    }

    assert!(
        gone.is_empty(),
        "these are listed in scripts/client-inventory.toml and are no longer in \
         the shipped client. Either the removal was an accident — restore it — \
         or it was deliberate, in which case delete the line from the manifest \
         in the same commit. `scripts/client-inventory.sh` shows both sides.\n\n{gone:#?}"
    );

    // A manifest that has quietly emptied catches nothing.
    assert!(
        listed(MANIFEST, "selectors").len() > 150
            && listed(MANIFEST, "ids").len() > 40
            && listed(MANIFEST, "methods").len() > 50,
        "the manifest has shrunk to the point of not being a record of anything"
    );
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

/// Every class the page renders, from markup, assignments and template strings.
fn classes_the_page_can_render() -> std::collections::HashSet<String> {
    let mut alive: std::collections::HashSet<String> = std::collections::HashSet::new();

    for source in [EMBEDDED_HTML, EMBEDDED_JS] {
        for (index, _) in source.match_indices("class=\"") {
            let rest = &source[index + "class=\"".len()..];
            if let Some(end) = rest.find('"') {
                alive.extend(rest[..end].split_whitespace().map(str::to_string));
            }
        }
    }

    for marker in [
        "className = '",
        "className = `",
        "classList.add('",
        "classList.toggle('",
        "className: '",
    ] {
        for (index, _) in EMBEDDED_JS.match_indices(marker) {
            let rest = &EMBEDDED_JS[index + marker.len()..];
            let Some(end) = rest.find(['\'', '`']) else {
                continue;
            };

            // `reaction-pill${mine ? ' mine' : ''}` is one class name and one
            // expression. Cutting each `${…}` out leaves the literal parts;
            // splitting on whitespace first would throw `reaction-pill` away
            // along with the expression glued to it.
            let mut literal = String::new();
            let mut depth = 0usize;
            let mut chars = rest[..end].chars().peekable();
            while let Some(c) = chars.next() {
                if depth == 0 && c == '$' && chars.peek() == Some(&'{') {
                    chars.next();
                    depth = 1;
                    literal.push(' ');
                } else if depth > 0 {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                    }
                } else {
                    literal.push(c);
                }
            }
            alive.extend(literal.split_whitespace().map(str::to_string));
        }
    }

    // `row.className = \`message ${isSent ? 'sent' : 'received'} run-end\`;`
    // and `updateConnectionStatus('connected')` both put a class-shaped word
    // where the parsing above cannot reach it: one is inside a `${…}`
    // ternary, the other is a value passed to a function that assembles the
    // class somewhere else entirely. Neither is a pattern worth chasing
    // individually — the general shape is "a bare, lowercase, hyphenated word
    // in quotes", which in this file is overwhelmingly a class or state name.
    // Any single-quoted JS string literal of that shape counts as alive,
    // rather than trying to trace which ones a stylesheet selector consumes.
    let is_class_shaped = |s: &str| {
        !s.is_empty()
            && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let mut rest = EMBEDDED_JS;
    while let Some(start) = rest.find('\'') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('\'') else { break };
        let literal = &rest[..end];
        if is_class_shaped(literal) {
            alive.insert(literal.to_string());
        }
        rest = &rest[end + 1..];
    }

    alive
}

/// No rule styles something the page never renders.
///
/// Dead CSS is not merely clutter. When the header was restructured, the rules
/// for the design it replaced — `.brand`, `.room-info`, `.room-badge` — stayed
/// behind. A rule for something that no longer exists is a rule nobody reads,
/// and the next person to reuse that name inherits it.
#[test]
fn no_rule_styles_something_the_page_never_renders() {
    let alive = classes_the_page_can_render();
    assert!(alive.len() > 40, "expected to find the page's classes");

    let css = embedded_html_without_comments();
    let mut dead: Vec<String> = Vec::new();

    for selector in css_rule_selectors(&css) {
        if !selector.starts_with('.') {
            continue;
        }

        let mentioned = classes_in_selector(&selector);

        // Every class named in the selector must be alive, not merely one of
        // them. `.old-container .icon` only ever matches an `.icon` that is a
        // descendant of `.old-container` — if the container never renders,
        // the rule is entirely dead regardless of how common `.icon` is
        // elsewhere. Requiring only one match let `.logo-icon .icon` survive
        // the header rewrite that deleted `.logo-icon`: `.icon` alone is used
        // on every SVG in the page, so the rule read as "alive" while
        // matching nothing a browser could ever select.
        if !mentioned.is_empty() && !mentioned.iter().all(|c| alive.contains(c)) {
            dead.push(selector.to_string());
        }
    }

    assert!(
        dead.is_empty(),
        "these rules style classes the page never renders — leftovers from a \
         design that was replaced, which the next person to reuse the name will \
         inherit: {dead:?}"
    );
}

/// Ids carry no layout, so a class can always win.
///
/// `#userCount` styled a small inline count. The header was restructured and
/// that id moved onto the button in the centre — a column with a name over a
/// subtitle — and the old rule beat every class on it, because an id selector
/// outranks any number of classes. The centre of the navigation bar laid itself
/// out as a row of chips inside a column: icons stacked, nothing errored, and
/// it was reported as "the icons are really not showing up right".
///
/// Ids identify; classes style. The exceptions are structural containers that
/// exist exactly once and are never restyled by role.
#[test]
fn stylesheet_ids_do_not_carry_layout() {
    const STRUCTURAL: &[&str] = &[
        "#chat",
        "#typingIndicator",
        "#replyPreview",
        "#emojiPanel",
        "#participantsSheet",
        "#lightbox",
        "#dropOverlay",
        "#jumpLatest",
        "#reactionBar",
        "#attachmentTray",
        "#welcomeBanner",
        "#messageInput",
        "#emojiSearch",
        "#emojiGrid",
        "#emojiTabs",
        "#emojiEmpty",
        "#sendBtn",
        "#charCount",
        "#participantsList",
        "#attachmentPreview",
        "#roomAvatar",
        "#lightboxImage",
        "#lightboxClose",
        "#participantsClose",
        "#jumpLatestCount",
        "#typingText",
        "#statusChip",
        "#roomHeartbeat",
    ];

    let css = embedded_html_without_comments();
    let mut offenders: Vec<String> = Vec::new();

    for selector in css_rule_selectors(&css) {
        if !selector.starts_with('#') {
            continue;
        }
        let id = selector
            .split([' ', ':', '.', '>'])
            .next()
            .unwrap_or(&selector);
        if !STRUCTURAL.contains(&id) {
            offenders.push(selector.to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these style an element by id. An id beats every class, so the next time \
         that id is reused for a different control the old layout wins silently \
         — which is how the navigation bar ended up stacked. Style by class: \
         {offenders:?}"
    );

    // Self-cleaning: an exemption must not outlive the element it names.
    for id in STRUCTURAL {
        let bare = id.trim_start_matches('#');
        assert!(
            EMBEDDED_HTML.contains(&format!("id=\"{bare}\"")),
            "{id} is exempted here but is no longer in the page — delete the entry"
        );
    }
}
