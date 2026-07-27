//! Plain HTTP: the route table, the handlers, and the error taxonomy.

use super::*;

#[tokio::test]
async fn test_root_redirect() {
    let app = Router::new().route("/", get(root_redirect));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    // Should be a permanent redirect (308) to /main
    let status = response.status();
    assert!(
        status == StatusCode::MOVED_PERMANENTLY || status == StatusCode::PERMANENT_REDIRECT,
        "Expected permanent redirect (301 or 308), got {}",
        status
    );
    assert_eq!(response.headers().get("location").unwrap(), "/main");
}

#[tokio::test]
async fn test_reserved_path_metrics_rejected() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Invalid room name"));
}

#[tokio::test]
async fn test_http_redirect_from_root() {
    let app = Router::new().route("/", get(root_redirect));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    // Root should redirect to /main with a permanent redirect (301 or 308)
    let status = response.status();
    assert!(
        status == StatusCode::MOVED_PERMANENTLY || status == StatusCode::PERMANENT_REDIRECT,
        "Expected permanent redirect (301 or 308), got {}",
        status
    );
    let location = response.headers().get("location").unwrap();
    assert_eq!(location, "/main");
}

#[tokio::test]
async fn test_chat_error_into_response_invalid_message() {
    let error = ChatError::InvalidMessage("Bad input".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chat_error_into_response_security_error() {
    let error = ChatError::SecurityError("Banned".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_health_endpoint() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/health", get(health_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(health["status"], "healthy");
    assert!(health["version"].is_string());
    assert!(health["connections"].is_number());
    assert!(health["rooms"].is_number());
    assert!(health["memory_bytes"].is_number());
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/plain; version=0.0.4"
    );

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body_str.contains("chat_rooms_total"));
    assert!(body_str.contains("chat_connections_total"));
    assert!(body_str.contains("chat_users_total"));
    assert!(body_str.contains("chat_messages_total"));
    assert!(body_str.contains("chat_memory_bytes"));
    assert!(body_str.contains("chat_memory_peak_bytes"));
}

#[tokio::test]
async fn test_metrics_handler_with_users() {
    let app_state = Arc::new(AppState::new());

    // Create a room with a user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "user1".to_string(),
            UserData {
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
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );
        rooms.insert("test-room".to_string(), room);
    }

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body.contains("chat_users_total 1"));
}

#[tokio::test]
async fn test_invalid_message_error_response() {
    let error = ChatError::InvalidMessage("Test error".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chat_error_invalid_message_status() {
    let error = ChatError::InvalidMessage("test".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chat_error_security_error_status() {
    let error = ChatError::SecurityError("test".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_health_handler_returns_ok() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/health", get(health_handler))
        .with_state(app_state);

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("ok") || body_str.contains("healthy"));
}

#[tokio::test]
async fn test_metrics_handler_returns_metrics() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(app_state);

    let req = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Check content type header
    let content_type = res.headers().get("content-type").unwrap();
    assert!(content_type.to_str().unwrap().contains("text/plain"));
}

#[tokio::test]
async fn test_all_chat_error_variants_into_response() {
    use axum::response::IntoResponse;

    // Test all ChatError variants
    let errors = vec![
        ChatError::RoomFull,
        ChatError::InvalidMessage("test".to_string()),
        ChatError::ResourceLimit("test".to_string()),
        ChatError::SecurityError("test".to_string()),
        ChatError::RateLimitError("test".to_string()),
    ];

    for error in errors {
        let response = error.into_response();
        // Verify we get a valid response
        assert!(response.status().as_u16() >= 400);
    }
}

// Test MemoryTracker peak_bytes tracking behavior - covers lines 362-370

#[tokio::test]
async fn test_health_handler_response() {
    let state = Arc::new(AppState::new());

    // Add a room with users
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
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
        rooms.insert("health-test".to_string(), room);
    }

    let response = health_handler(State(state)).await;
    let json_response = response.into_response();
    assert_eq!(json_response.status(), StatusCode::OK);
}

// Test metrics_handler returns prometheus format - covers lines 2234-2265

#[tokio::test]
async fn test_metrics_handler_format() {
    let state = Arc::new(AppState::new());

    // Add some data
    {
        let mut rooms = state.rooms.write().await;
        rooms.insert("metrics-test".to_string(), create_room());
    }

    let response = metrics_handler(State(state)).await;
    let http_response = response.into_response();
    assert_eq!(http_response.status(), StatusCode::OK);
}

// Test room_handler with reserved path - covers lines 1129-1145

#[tokio::test]
async fn test_chat_error_display() {
    let errors = vec![
        ChatError::InvalidMessage(String::from("test")),
        ChatError::RoomFull,
        ChatError::ResourceLimit(String::from("memory")),
        ChatError::SecurityError(String::from("banned")),
        ChatError::RateLimitError(String::from("too fast")),
    ];

    for err in errors {
        let display = format!("{}", err);
        assert!(!display.is_empty());
    }
}

// Test ClientEvent deserialization - covers lines 157-180

#[tokio::test]
async fn test_header_map_operations() {
    // Test that HeaderMap works correctly
    let mut h = HeaderMap::new();
    h.insert("X-Real-IP", "192.168.1.1".parse().unwrap());

    // Can retrieve header
    assert!(h.get("X-Real-IP").is_some());
    let ip_val = h.get("X-Real-IP").unwrap().to_str().unwrap();
    assert_eq!(ip_val, "192.168.1.1");
}

// Test ConnectionState transitions - covers lines 315-330

/// Every route the deployment depends on is actually mounted.
///
/// `build_router` exists so this asserts against the real table rather than a
/// hand-assembled copy that can silently drift from `main`.
#[tokio::test]
async fn router_mounts_every_public_route() {
    for (path, expected) in [
        ("/", StatusCode::PERMANENT_REDIRECT),
        ("/main", StatusCode::OK),
        ("/health", StatusCode::OK),
        ("/metrics", StatusCode::OK),
        ("/robots.txt", StatusCode::OK),
        ("/some-room", StatusCode::OK),
    ] {
        let app = build_router(Arc::new(AppState::new()));
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), expected, "unexpected status for {path}");
    }
}

// ========== CONNECTION ACCOUNTING ==========

/// With no Host header to compare against, the origin is checked against the
/// known domains.
#[test]
fn origin_check_falls_back_to_known_domains_without_a_host() {
    assert!(is_allowed_origin(Some("https://thehellisthis.com"), None));
    assert!(is_allowed_origin(
        Some("https://www.thehellisthis.com"),
        None
    ));
    assert!(is_allowed_origin(Some("http://localhost:3000"), None));
    assert!(is_allowed_origin(Some("http://127.0.0.1:3000"), None));

    assert!(!is_allowed_origin(Some("https://evil.example"), None));
}

/// Binding reports failure rather than panicking, and `run` propagates it.
///
/// The container sets `PORT`; a port already in use is the one thing that goes
/// wrong at startup, and it is the difference between a container that starts
/// and one that crash-loops with nothing useful in the log.
#[tokio::test]
async fn binding_a_port_already_in_use_is_an_error_not_a_panic() {
    // Hold the same address `bind_listener` uses. Holding `127.0.0.1:port`
    // instead does not conflict with `0.0.0.0:port` — the first version of this
    // test did exactly that, so the bind succeeded, `run` went on to serve, and
    // the test hung waiting for a SIGINT that was never coming.
    let occupied = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();

    let result = crate::startup::bind_listener(port).await;

    assert!(
        result.is_err(),
        "binding an occupied port must return an error for `main` to report,          not panic and not succeed"
    );

    // Released, the same port binds cleanly — so the failure was the conflict
    // rather than anything about the port itself.
    drop(occupied);
    assert!(crate::startup::bind_listener(port).await.is_ok());
}

/// How the server stopped is reported, either way.
#[test]
fn a_server_error_is_reported_and_a_clean_stop_is_not_an_error() {
    // Neither arm returns anything; what is asserted is that both are
    // executable and neither panics — the error arm in particular, which
    // otherwise needs a live server to fail mid-flight.
    crate::startup::log_server_result(Ok(()));
    crate::startup::log_server_result(Err(std::io::Error::other("connection reset")));
}
