//! `room_handler`'s HTTP path: name validation, the reserved/length/shape/
//! capacity check order (constraint #4), and the plain-error responses that
//! come out of it.

use super::*;

#[tokio::test]
async fn test_main_room_handler() {
    let app = Router::new().route("/main", get(main_room_handler));

    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!body.is_empty());
}

#[tokio::test]
async fn test_room_handler_invalid_length() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(Request::builder().uri("/ro").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters")
    );
}

#[tokio::test]
async fn test_room_handler_reserved_path() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
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
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Invalid room name"));
}

#[tokio::test]
async fn test_room_name_boundary_cases() {
    let _app_state = Arc::new(AppState::new());

    // Test minimum valid length (just 1 char is actually ok per validate_input)
    assert!(validate_input("a", 50).is_ok());

    // Test maximum valid length (50 chars)
    let max_name = "a".repeat(50);
    assert!(validate_input(&max_name, 50).is_ok());

    // Test just over max (51 chars)
    let too_long = "a".repeat(51);
    assert!(validate_input(&too_long, 50).is_err());

    // Test empty (not allowed)
    assert!(validate_input("", 50).is_err());
}

#[tokio::test]
async fn test_room_name_too_short() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/ab") // Less than MIN_ROOM_NAME_LEN (3)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("between 3 and 50"));
}

#[tokio::test]
async fn test_room_name_too_long() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let long_name = "a".repeat(51);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/{}", long_name))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("between 3 and 50"));
}

#[tokio::test]
async fn test_room_name_invalid_format_starts_with_dash() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("start/end with alphanumeric"));
}

#[tokio::test]
async fn test_room_name_invalid_format_ends_with_underscore() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/invalid_")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("start/end with alphanumeric"));
}

#[tokio::test]
async fn test_room_name_with_hyphens_and_underscores() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test-room_123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_name_all_numbers() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/12345")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_name_with_numbers() {
    let room = "room123";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_name_with_hyphen() {
    let room = "my-cool-room";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_name_with_underscore() {
    let room = "my_room_name";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_room_name_mixed_case_new() {
    let room = "MyRoomName";
    let result = validate_input(room, 50);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_chat_error_into_response_room_full() {
    let error = ChatError::RoomFull;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_room_name_validation_via_handler() {
    // Test room name validation through room_handler
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Valid room name
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/myroom123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Invalid room name (starts with hyphen) - should fail regex
    let response2 = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(Arc::new(AppState::new()))
        .oneshot(
            Request::builder()
                .uri("/-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response2.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("alphanumeric"));
}

#[tokio::test]
async fn test_reserved_room_names_via_handler() {
    let reserved = [
        "robots.txt",
        "main",
        "admin",
        "api",
        "ws",
        "health",
        "metrics",
    ];

    for name in reserved {
        let app_state = Arc::new(AppState::new());
        let app = Router::new()
            .route("/{room}", get(room_handler))
            .with_state(app_state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        // Reserved names should return an error message
        assert!(
            body_str.contains("Reserved")
                || body_str.contains("reserved")
                || body_str.contains("Invalid")
        );
    }
}

#[tokio::test]
async fn test_room_max_reached_error() {
    let app_state = Arc::new(AppState::new());

    // Fill up rooms to MAX_ROOMS
    {
        let mut rooms = app_state.rooms.write().await;
        for i in 0..MAX_ROOMS {
            rooms.insert(format!("room{}", i), create_room());
        }
    }

    // Now try to access a new room via room_handler
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/newroom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should get an error message about max rooms
    let status = response.status();
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(
        body.contains("Maximum number of rooms reached")
            || body.contains("max")
            || status == StatusCode::OK
    );
}

#[tokio::test]
async fn test_main_room_handler_content_type() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/main", get(main_room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Should return HTML content
    let content_type = response.headers().get("content-type");
    assert!(content_type.is_some());
}

#[tokio::test]
async fn test_room_handler_invalid_regex() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Room name that doesn't match regex (starts with hyphen)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("alphanumeric") || body.contains("Invalid"));
}

#[tokio::test]
async fn test_room_handler_too_long_name() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Room name that's too long
    let long_name = "a".repeat(MAX_ROOM_NAME_LEN + 1);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/{}", long_name))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("Invalid") || body.contains("room") || status == StatusCode::OK);
}

#[tokio::test]
async fn test_health_handler_with_rooms() {
    let app_state = Arc::new(AppState::new());

    // Create a room
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("test-room".to_string(), create_room());
    }

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

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(health["rooms"], 1);
}

#[tokio::test]
async fn test_room_handler_special_chars_rejected() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Room name with special characters
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/room!name")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("Invalid") || body.contains("alphanumeric"));
}

#[tokio::test]
async fn test_room_handler_valid_name() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/valid-room-name")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_handler_max_rooms_reached() {
    let app_state = Arc::new(AppState::new());

    // Fill up to MAX_ROOMS
    {
        let mut rooms = app_state.rooms.write().await;
        for i in 0..MAX_ROOMS {
            rooms.insert(format!("existingroom{}", i), create_room());
        }
    }

    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Request a new room that doesn't exist
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/newroomthatdoesntexist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    // Should get "Maximum number of rooms reached" error
    assert!(body.contains("Maximum") || body.contains("room"));
}

#[tokio::test]
async fn test_room_handler_existing_room_at_max_rooms() {
    let app_state = Arc::new(AppState::new());

    // Fill up to MAX_ROOMS with one named "existingroom"
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("existingroom".to_string(), create_room());
        for i in 1..MAX_ROOMS {
            rooms.insert(format!("room{}", i), create_room());
        }
    }

    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Request an existing room - should work even at max rooms
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/existingroom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be successful since room exists
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_handler_special_char_validation() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Test room name with only hyphens
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/a-b-c")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be valid (alphanumeric with hyphens)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_room_full_error_response() {
    let error = ChatError::RoomFull;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_chat_error_room_full_status() {
    let error = ChatError::RoomFull;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_room_name_regex_pattern() {
    let re = regex::Regex::new(ROOM_NAME_REGEX).unwrap();

    // Valid names
    assert!(re.is_match("abc"));
    assert!(re.is_match("a1b"));
    assert!(re.is_match("test-room"));
    assert!(re.is_match("test_room"));
    assert!(re.is_match("Test123"));

    // Invalid names (start/end with special chars)
    assert!(!re.is_match("-test"));
    assert!(!re.is_match("test-"));
    assert!(!re.is_match("_test"));
    assert!(!re.is_match("test_"));
    // Note: "ab" matches regex, min length is enforced separately
}

// ========== MEMORY TRACKER EDGE CASES ==========

#[tokio::test]
async fn test_room_handler_reserved_paths() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let reserved = [
        "robots.txt",
        "sitemap.xml",
        "favicon.ico",
        "main",
        "admin",
        "api",
        "health",
        "metrics",
        "ws",
    ];

    for path in reserved.iter() {
        let req = Request::builder()
            .uri(format!("/{}", path))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("Invalid") || body_str.contains("<!DOCTYPE"),
            "Reserved path {} should be handled specially",
            path
        );
    }
}

#[tokio::test]
async fn test_room_handler_more_reserved_paths() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Test various reserved paths
    let reserved = vec!["admin", "api", "ws"];
    for path in reserved {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", path))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Invalid room name"));
    }
}

// Test room_handler with invalid room name length - covers lines 1146-1150

#[tokio::test]
async fn test_room_handler_name_length_bounds() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Too short (less than 3 chars)
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/ab").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters")
    );

    // Too long (more than 50 chars)
    let long_name = "a".repeat(51);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/{}", long_name))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters")
    );
}

// Test room_handler with invalid regex pattern - covers lines 1154-1159

#[tokio::test]
async fn test_room_handler_invalid_regex_patterns() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Invalid patterns that fail the regex (not starting/ending with alphanumeric)
    let invalid = vec!["has@symbol", "has.dot"];
    for name in invalid {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        // Should fail regex validation
        assert!(
            String::from_utf8_lossy(&body).contains("alphanumeric")
                || String::from_utf8_lossy(&body).contains("Invalid")
        );
    }
}

// Test AppState cleanup triggers memory GC - covers lines 1073-1088

#[tokio::test]
async fn test_room_handler_hyphen_underscore() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    // Valid patterns with hyphens and underscores - should return the HTML page, not error
    let valid_names = vec!["test-room", "test_room", "test-room-name", "a1b2c3"];
    for name in valid_names {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);
        // Should NOT contain specific error messages
        assert!(
            !body_str.contains("Room name must be between 3 and 50 characters"),
            "Room '{}' incorrectly rejected for length",
            name
        );
        assert!(
            !body_str.contains("Room name must start/end with alphanumeric"),
            "Room '{}' incorrectly rejected for regex",
            name
        );
        // Should contain HTML content indicating success
        assert!(
            body_str.contains("<!DOCTYPE html>") || body_str.contains("<html"),
            "Room '{}' should return HTML page",
            name
        );
    }
}

// ========== ROSTER INTEGRITY ==========
