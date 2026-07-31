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

/// `/metrics` discloses operational counts to nobody who cannot produce the
/// configured token — see `security::is_authorized_for_metrics`. Every case
/// this handler can reach: no token configured, a missing header, a wrong
/// token, and the one correct request, which is also where the response body
/// itself is checked, since there is no point asserting the content twice
/// under two different names for the same handler.
#[tokio::test]
async fn metrics_requires_the_configured_bearer_token() {
    let _env = METRICS_TOKEN_ENV.lock().await;

    let app_state = Arc::new(AppState::new());
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
                last_message_text: None,
                last_reaction_event: None,
            },
        );
        rooms.insert("test-room".to_string(), room);
    }

    let app = || {
        Router::new()
            .route("/metrics", get(metrics_handler))
            .with_state(Arc::clone(&app_state))
    };
    let request = |auth: Option<&str>| {
        let mut builder = Request::builder().uri("/metrics");
        if let Some(header) = auth {
            builder = builder.header("authorization", header);
        }
        builder.body(Body::empty()).unwrap()
    };

    // SAFETY: serialised by METRICS_TOKEN_ENV; cleared before the lock is
    // released, on every path below.
    unsafe { std::env::remove_var("METRICS_TOKEN") };

    // Not configured at all: fails closed, whatever the request carries.
    let unconfigured = app()
        .oneshot(request(Some("Bearer anything")))
        .await
        .unwrap();
    assert_eq!(unconfigured.status(), StatusCode::NOT_FOUND);

    // SAFETY: still within the lock.
    unsafe { std::env::set_var("METRICS_TOKEN", "correct-horse-battery-staple") };

    let no_header = app().oneshot(request(None)).await.unwrap();
    assert_eq!(no_header.status(), StatusCode::NOT_FOUND);

    let wrong_token = app().oneshot(request(Some("Bearer wrong"))).await.unwrap();
    assert_eq!(wrong_token.status(), StatusCode::NOT_FOUND);

    let not_bearer = app()
        .oneshot(request(Some("correct-horse-battery-staple")))
        .await
        .unwrap();
    assert_eq!(
        not_bearer.status(),
        StatusCode::NOT_FOUND,
        "the scheme matters, not just the token value"
    );

    let authorized = app()
        .oneshot(request(Some("Bearer correct-horse-battery-staple")))
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    assert_eq!(
        authorized.headers().get("content-type").unwrap(),
        "text/plain; version=0.0.4"
    );

    let body_bytes = to_bytes(authorized.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("chat_rooms_total"));
    assert!(body.contains("chat_connections_total"));
    assert!(body.contains("chat_users_total 1"));
    assert!(body.contains("chat_messages_total"));
    assert!(body.contains("chat_memory_bytes"));
    assert!(body.contains("chat_memory_peak_bytes"));

    // SAFETY: still within the lock; removed before it is released.
    unsafe { std::env::remove_var("METRICS_TOKEN") };
}

/// `/admin` names every open room, so it is gated on its own password
/// (`ADMIN_TOKEN`), checked via HTTP Basic — see
/// `security::is_authorized_for_admin`. Every case this handler can reach: no
/// token configured, no credentials, the wrong password, and the one correct
/// request, which is also where the room list itself is checked.
#[tokio::test]
async fn admin_dashboard_requires_the_configured_password() {
    let _env = ADMIN_TOKEN_ENV.lock().await;

    let app_state = Arc::new(AppState::new());
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("test-room".to_string(), create_room());
        rooms.insert("<script>".to_string(), create_room());
    }

    let app = || {
        Router::new()
            .route("/admin", get(admin_dashboard_handler))
            .with_state(Arc::clone(&app_state))
    };
    let request = |auth: Option<&str>| {
        let mut builder = Request::builder().uri("/admin");
        if let Some(header) = auth {
            builder = builder.header("authorization", header);
        }
        builder.body(Body::empty()).unwrap()
    };
    let basic = |password: &str| {
        use base64::Engine;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("admin:{password}"))
        )
    };

    // SAFETY: serialised by ADMIN_TOKEN_ENV; cleared before the lock is
    // released, on every path below.
    unsafe { std::env::remove_var("ADMIN_TOKEN") };

    // Not configured at all: fails closed, whatever the request carries.
    let unconfigured = app()
        .oneshot(request(Some(&basic("anything"))))
        .await
        .unwrap();
    assert_eq!(unconfigured.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unconfigured.headers().get("www-authenticate").unwrap(),
        "Basic realm=\"admin\""
    );

    // SAFETY: still within the lock.
    unsafe { std::env::set_var("ADMIN_TOKEN", "correct-horse-battery-staple") };

    let no_header = app().oneshot(request(None)).await.unwrap();
    assert_eq!(no_header.status(), StatusCode::UNAUTHORIZED);

    let wrong_password = app().oneshot(request(Some(&basic("wrong")))).await.unwrap();
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);

    let authorized = app()
        .oneshot(request(Some(&basic("correct-horse-battery-staple"))))
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);

    let body_bytes = to_bytes(authorized.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("test-room"));
    assert!(body.contains("href=\"/test-room\""));
    // A room name that could not exist through normal admission (rejected by
    // `matches_room_name_shape`) is still rendered escaped, not as a tag —
    // the defense in depth the handler's doc comment describes.
    assert!(!body.contains("<script>"));
    assert!(body.contains("&lt;script&gt;"));

    // SAFETY: still within the lock; removed before it is released.
    unsafe { std::env::remove_var("ADMIN_TOKEN") };
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
                last_message_text: None,
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
        // Mounted, but requires the bearer token this request does not carry
        // — see `metrics_requires_the_configured_bearer_token`, which is
        // where its real behaviour is checked. That 404 is deliberately
        // indistinguishable from a path this router never mounted at all, so
        // this row cannot tell the two apart by status code alone; it exists
        // to keep every path in this list, not to re-prove the route exists.
        ("/metrics", StatusCode::NOT_FOUND),
        // Mounted, but requires the Basic credentials this request does not
        // carry — see `admin_dashboard_requires_the_configured_password`,
        // which is where its real behaviour is checked. Unlike `/metrics`
        // this reads as `401`, not `404`: see
        // `security::is_authorized_for_admin` for why the two disagree on
        // purpose.
        ("/admin", StatusCode::UNAUTHORIZED),
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
#[tokio::test]
async fn a_server_error_is_reported_and_a_clean_stop_is_not_an_error() {
    // This used to call both arms and assert nothing, so replacing the whole
    // function with `()` survived mutation testing: an operator would have had
    // no way to tell a server that died from one that stopped. The log is the
    // only output this function has, which makes the log the assertion (§5.12).
    let clean = capturing_logs(|| async {
        crate::startup::log_server_result(Ok(()));
    })
    .await;
    assert!(
        clean.is_empty(),
        "a clean stop is not an error and must say nothing, got: {clean}"
    );

    let failed = capturing_logs(|| async {
        crate::startup::log_server_result(Err(std::io::Error::other("connection reset")));
    })
    .await;
    assert!(
        failed.contains("server error") && failed.contains("connection reset"),
        "a server that died must name itself and the reason, got: {failed}"
    );
}

/// The script version is a real FNV-1a hash, not merely *some* function of the
/// bytes.
///
/// Every mutation of `fnv1a` survived the suite: returning a constant, never
/// entering the loop, replacing the xor with an or. Nothing checked the value,
/// only that two different inputs disagreed — which a constant-free but wrong
/// hash satisfies just as well. A stamped URL that fails to change when the
/// script does serves a stale client for a year (`immutable`), so the function
/// being *this* hash is the property worth pinning.
///
/// The vectors are the published FNV-1a 64-bit ones.
#[test]
fn the_script_version_is_the_fnv1a_hash_it_claims_to_be() {
    assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325, "the offset basis");
    assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);

    // Order matters: a hash that only sums or ors its bytes cannot see this.
    assert_ne!(fnv1a(b"ab"), fnv1a(b"ba"));
}

/// A name exactly at each end of the allowed length is allowed.
///
/// `room.len() < MIN || room.len() > MAX` reads as inclusive bounds, and the
/// user-facing string promises "between 3 and 50 characters". Both comparisons
/// could be made strict without a test noticing, which would silently refuse
/// the shortest and longest names the page says are fine.
#[tokio::test]
async fn a_room_name_at_either_length_boundary_is_accepted() {
    let state = Arc::new(AppState::new());

    for len in [MIN_ROOM_NAME_LEN, MAX_ROOM_NAME_LEN] {
        let name = "a".repeat(len);
        let response = room_handler(Path(name.clone()), State(Arc::clone(&state))).await;
        let body = response.into_response();
        let bytes = axum::body::to_bytes(body.into_body(), usize::MAX)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&bytes);
        assert!(
            html.contains("<!DOCTYPE html>") || html.contains("<!doctype html>"),
            "a {len}-character name is within the advertised range and must be served, \
             got: {}",
            &html[..html.len().min(120)]
        );
    }

    for len in [MIN_ROOM_NAME_LEN - 1, MAX_ROOM_NAME_LEN + 1] {
        let name = "a".repeat(len);
        let response = room_handler(Path(name), State(Arc::clone(&state))).await;
        let bytes = axum::body::to_bytes(response.into_response().into_body(), usize::MAX)
            .await
            .expect("body");
        assert!(
            String::from_utf8_lossy(&bytes).contains("between 3 and 50"),
            "a {len}-character name is outside the advertised range and must be refused"
        );
    }
}

/// A room that already exists is served even when the server is at its cap.
///
/// The cap refuses *new* rooms; `!rooms.contains_key(&room)` is what makes it a
/// creation limit rather than an admission limit. Deleting that `!` survived,
/// because no test ever asked for an existing room on a full server — the case
/// where every person already in a conversation is locked out of it.
#[tokio::test]
async fn an_existing_room_is_served_even_when_the_server_is_at_its_room_cap() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        for i in 0..MAX_ROOMS {
            rooms.insert(format!("room{i:04}"), create_room());
        }
        assert_eq!(rooms.len(), MAX_ROOMS, "the server is exactly at its cap");
    }

    let response = room_handler(Path("room0000".to_string()), State(Arc::clone(&state))).await;
    let bytes = axum::body::to_bytes(response.into_response().into_body(), usize::MAX)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        !html.contains("Maximum number of rooms reached"),
        "an existing room must stay reachable at the cap, or a full server \
         locks out the people already talking in it"
    );

    let response = room_handler(Path("brandnewroom".to_string()), State(state)).await;
    let bytes = axum::body::to_bytes(response.into_response().into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(
        String::from_utf8_lossy(&bytes).contains("Maximum number of rooms reached"),
        "a room that does not exist yet must still be refused at the cap"
    );
}
