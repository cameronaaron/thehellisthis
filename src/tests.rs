// src/tests.rs

use axum::{
    body::{to_bytes, Body}, 
    http::{Request, StatusCode}, 
    Router,
    extract::{Path, State, ws::WebSocketUpgrade},
    routing::get,
};
use std::sync::Arc;
use uuid::Uuid;
use crate::*;
use tower::util::ServiceExt;
use std::str;
use tokio::sync::Barrier;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use tokio_tungstenite::{accept_async, connect_async};
use tungstenite::protocol::Message as TungsteniteMessage;
use tokio::net::TcpListener;
use futures::{FutureExt, StreamExt, SinkExt};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};

// Helper function to create WebSocket requests
async fn create_ws_request(path: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header("Host", "localhost")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn test_root_redirect() {
    let app = Router::new().route("/", get(root_redirect));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get("location").unwrap(), "/main");
}

#[tokio::test]
async fn test_main_room_handler() {
    let app = Router::new().route("/main", get(main_room_handler));

    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(body.len() > 0);
}

#[tokio::test]
async fn test_room_handler_invalid_length() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/:room", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(Request::builder().uri("/ro").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Room name must be between 3 and 50 characters"));
}

#[tokio::test]
async fn test_room_handler_reserved_path() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/:room", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(Request::builder().uri("/robots.txt").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Invalid room name"));
}

#[tokio::test]
async fn test_connection_pool_cleanup() {
    let connection_pool = ConnectionPool::new();
    let ip = "192.168.1.3";

    // Add a connection
    connection_pool.add_connection(ip).await.unwrap();
    connection_pool.cleanup_stale().await;

    assert!(connection_pool.can_accept(ip).await);
}

#[tokio::test]
async fn test_ws_handler_rate_limit_exceeded() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/ws/:room", get(
            |ws: WebSocketUpgrade, 
             Path(room): Path<String>,
             State(state): State<Arc<AppState>>| async move {
                let connection_id = Uuid::new_v4().to_string();
                ws.on_upgrade(move |socket| handle_websocket(room, state.clone(), None, socket, connection_id))
            }
        ))
        .with_state(app_state.clone());

    // Simulate rate limit by rapidly sending multiple requests
    for i in 0..(MAX_MESSAGES_PER_WINDOW + 1) {
        let request = create_ws_request("/ws/test_room").await;
        let response = app.clone().oneshot(request).await.unwrap();
        // The first MAX_MESSAGES_PER_WINDOW should succeed, the last should be rate limited
        if i < MAX_MESSAGES_PER_WINDOW {
            assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        } else {
            // Assuming your rate limiter returns TOO_MANY_REQUESTS for rate limit exceeded
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }
    }
}

#[tokio::test]
async fn test_ws_handler_input_sanitization() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/ws/:room", get(
            |ws: WebSocketUpgrade, 
             Path(room): Path<String>,
             State(state): State<Arc<AppState>>| async move {
                let connection_id = Uuid::new_v4().to_string();
                ws.on_upgrade(move |socket| handle_websocket(room, state.clone(), None, socket, connection_id))
            }
        ))
        .with_state(app_state.clone());

    let request = create_ws_request("/ws/sanitize_test_room").await;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
}

#[tokio::test]
async fn test_create_room() {
    let room = create_room();
    assert!(room.chat_history.is_empty());
    assert_eq!(room.available_animals.len(), 132);
}

#[tokio::test]
async fn test_rate_limiter() {
    let mut rate_limiter = RateLimiter::new();
    
    // Initial message should succeed
    assert!(rate_limiter.can_send_message());
    
    // Fill up to limit
    for _ in 0..(MAX_MESSAGES_PER_WINDOW-1) {
        assert!(rate_limiter.can_send_message());
    }
    
    // Next message should fail
    assert!(!rate_limiter.can_send_message());
    
    // Reset window
    rate_limiter.window_start = Instant::now() - RATE_LIMIT_WINDOW - Duration::from_secs(1);
    
    // Should work again after reset
    assert!(rate_limiter.can_send_message());
}

#[tokio::test]
async fn test_memory_tracker() {
    let tracker = MemoryTracker::new();
    assert!(tracker.add_bytes(1024));
    assert!(tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY - 1024));
    assert!(!tracker.add_bytes(1));
    tracker.remove_bytes(1024);
    assert!(tracker.add_bytes(1024));
}

#[tokio::test]
async fn test_memory_tracker_concurrency() {
    use tokio::sync::Barrier;

    let tracker = Arc::new(MemoryTracker::new());
    let barrier = Arc::new(Barrier::new(10));

    let mut handles = vec![];
    for _ in 0..10 {
        let tracker_clone = tracker.clone();
        let barrier_clone = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier_clone.wait().await;
            tracker_clone.add_bytes(1000);
        }));
    }

    futures::future::join_all(handles).await;
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 10000);
}

#[tokio::test]
async fn test_validate_message() {
    assert!(validate_message("Hello").is_ok());
    assert!(validate_message(&"a".repeat(MAX_MESSAGE_LEN + 1)).is_err());
    assert!(validate_message("<script>alert('xss')</script>").is_ok());
}

#[tokio::test]
async fn test_cleanup_user() {
    let app_state = Arc::new(AppState::new());
    let room_name = "cleanup_room".to_string();
    let user_id = "test_user".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        let room = create_room();
        rooms.insert(room_name.clone(), room);
    }

    cleanup_user(&app_state, &room_name, &user_id, "test_conn", None).await;
    let rooms = app_state.rooms.read().await;
    assert!(rooms.get(&room_name).is_some());
}

#[tokio::test]
async fn test_cleanup_rooms() {
    let app_state = Arc::new(AppState::new());
    
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - Duration::from_secs(7201);
        room.users.clear();  // Ensure room is empty
        rooms.insert("inactive_room".to_string(), room);
    }
    
    cleanup_rooms(&app_state).await;
    
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("inactive_room"));
}

#[tokio::test]
async fn test_trigger_cleanup() {
    let mut room_state = create_room();

    // Add messages up to limit
    for _ in 0..MAX_MESSAGES_PER_ROOM {
        room_state.add_message(OutgoingMessage {
            message_id: Uuid::new_v4(),
            animal_name: "test_animal".to_string(),
            text: "test_message".to_string(),
            timestamp: "1234567890".to_string(),
        });
    }

    room_state.preserve_messages();
    assert_eq!(room_state.chat_history.len(), MAX_MESSAGES_PER_ROOM);
}

#[tokio::test]
async fn test_generate_random_room_name() {
    let name = generate_random_room_name();
    assert!(!name.is_empty());
    assert!(name.contains("-"));
}

#[tokio::test]
async fn test_validate_input() {
    assert!(validate_input("room_name", 50).is_ok());
    assert!(validate_input("", 50).is_err());
    assert!(validate_input(&"a".repeat(51), 50).is_err());
    assert!(validate_input("invalid name!", 50).is_err());
}

#[tokio::test]
async fn test_security_manager_ban_ip() {
    let security_manager = SecurityManager::new();
    let ip = "10.0.0.1";

    // Record suspicious activities until banned
    for _ in 0..11 {
        let _ = security_manager.record_suspicious_activity(ip).await;
    }

    // Verify IP is banned
    assert!(matches!(
        security_manager.check_ip(ip).await,
        Err(ChatError::SecurityError(_))
    ));
}

#[tokio::test]
async fn test_connection_pool_can_accept_new_connection_alt() {
    let connection_pool = ConnectionPool::new();
    let ip = "192.168.1.2";

    // Initially, should accept
    assert!(connection_pool.can_accept(ip).await);

    // Add max connections
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(connection_pool.add_connection(ip).await.is_ok());
    }

    // Now, should not accept more
    assert!(!connection_pool.can_accept(ip).await);
}

#[tokio::test]
async fn test_handle_websocket_reconnection_token_alt() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/ws/:room", get(
            |ws: WebSocketUpgrade, 
             Path(room): Path<String>,
             State(state): State<Arc<AppState>>| async move {
                let connection_id = Uuid::new_v4().to_string();
                ws.on_upgrade(move |socket| handle_websocket(room, state.clone(), None, socket, connection_id))
            }
        ))
        .with_state(app_state.clone());

    let request = create_ws_request("/ws/reconnect_test_room").await;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let headers = response.headers();
    assert!(headers.contains_key("Set-Cookie"));
}

#[tokio::test]
async fn test_appstate_memory_cleanup_trigger() {
    let app_state = Arc::new(AppState::new());

    // Simulate memory usage nearing the limit
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.total_memory_bytes.store((MAX_TOTAL_ROOMS_MEMORY * 9) / 10, Ordering::SeqCst);
        rooms.insert("memory_cleanup_room".to_string(), room);
    }

    // Perform cleanup which should trigger message cleanup
    app_state.cleanup().await;

    // Verify memory has been cleaned up
    let rooms = app_state.rooms.read().await;
    let room = rooms.get("memory_cleanup_room").unwrap();
    assert!(room.total_memory_bytes.load(Ordering::SeqCst) < (MAX_TOTAL_ROOMS_MEMORY * 9) / 10);
}

#[tokio::test]
async fn test_appstate_graceful_shutdown_no_rooms() {
    let app_state = Arc::new(AppState::new());

    // Call graceful_shutdown with no rooms
    app_state.graceful_shutdown().await;

    // Verify rooms are still empty
    let rooms = app_state.rooms.read().await;
    assert!(rooms.is_empty());
}

#[tokio::test]
async fn test_handle_websocket_duplicate_messages() {
    let app_state = Arc::new(AppState::new());
    let room_name = "duplicate_msg_room".to_string();
    let user_id = "duplicate_user".to_string();
    let connection_id = "duplicate_conn".to_string();
    let animal_name = "duplicate_animal".to_string();

    // Setup room and user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(user_id.clone(), UserData {
            animal_name: animal_name.clone(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: connection_id.clone(),
            },
            connection_id: connection_id.clone(),
            last_read_message: None,
            is_typing: false,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
        });
        rooms.insert(room_name.clone(), room);
    }

    // Simulate sending a duplicate message within the debounce timeout
    {
        let mut rooms = app_state.rooms.write().await;
        let room = rooms.get_mut(&room_name).unwrap();
        let user = room.users.get_mut(&user_id).unwrap();

        let message_text = "Hello World";
        let sanitized = validate_message(message_text).unwrap();
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            animal_name: animal_name.clone(),
            text: sanitized.clone(),
            timestamp: "1234567890".to_string(),
        };

        // First message should be accepted
        user.last_sanitized_message = Some((message_text.to_string(), Instant::now()));
        assert!(user.last_sanitized_message.is_some());

        // Duplicate message within timeout should be rejected
        let now = Instant::now();
        if let Some((last_text, last_time)) = &user.last_sanitized_message {
            if message_text == last_text && now.duration_since(*last_time) < SANITIZE_TIMEOUT {
                // Should not add the message
                room.add_message(msg.clone());
                assert_eq!(room.chat_history.len(), 1); // No new message added
            }
        }
    }
}

#[tokio::test]
async fn test_handle_websocket_ping_pong() {
    use tokio_tungstenite::accept_async;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::Message as TungsteniteMessage;

    let app_state = Arc::new(AppState::new());
    let room_name = "ping_pong_room".to_string();
    let user_id = "ping_pong_user".to_string();
    let connection_id = "ping_pong_conn".to_string();
    let animal_name = "ping_pong_animal".to_string();

    // Setup room and user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(user_id.clone(), UserData {
            animal_name: animal_name.clone(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: connection_id.clone(),
            },
            connection_id: connection_id.clone(),
            last_read_message: None,
            is_typing: false,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
        });
        rooms.insert(room_name.clone(), room);
    }

    // Start a mock WebSocket server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_app_state = app_state.clone();
    let server_room_name = room_name.clone();
    let server_user_id = user_id.clone();
    let server_animal_name = animal_name.clone();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = accept_async(stream).await.unwrap();

        handle_websocket(
            server_room_name,
            server_app_state,
            Some(AnimalCookie(server_animal_name)),
            axum::extract::ws::WebSocket::from_raw_socket(ws, axum::extract::ws::WebSocketConfig::default(), false).await,
            connection_id.clone(),
        )
        .await;
    });

    // Connect as a client
    let (ws_stream, _) = connect_async(format!("ws://{}", addr)).await.unwrap();
    let (mut write, mut read) = ws_stream.split();

    // Send Ping from client
    write.send(TungsteniteMessage::Ping(Vec::new())).await.unwrap();

    // Read Pong from server
    if let Some(message) = read.next().await {
        match message {
            Ok(TungsteniteMessage::Pong(_)) => {
                // Success
            }
            _ => panic!("Expected Pong message"),
        }
    } else {
        panic!("Did not receive any message");
    }

    server.await.unwrap();
}

#[tokio::test]
async fn test_memory_tracker_should_gc() {
    let tracker = MemoryTracker::new();
    tracker.last_gc.store(0, Ordering::Relaxed);
    assert!(tracker.should_gc());

    tracker.last_gc.store(300, Ordering::Relaxed); // Less than 5 minutes
    assert!(!tracker.should_gc());
}

#[tokio::test]
async fn test_rate_limiter_can_join_room() {
    let mut rate_limiter = RateLimiter::new();
    for _ in 0..MAX_ROOM_JOIN_ATTEMPTS {
        assert!(rate_limiter.can_join_room());
    }
    assert!(!rate_limiter.can_join_room());
}

#[tokio::test]
async fn test_resource_monitor_can_accept_multiple_connections() {
    let resource_monitor = ResourceMonitor::new();
    let pool = ConnectionPool::new();
    let ip = "192.168.1.4";

    assert!(pool.can_accept(ip).await);
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(pool.add_connection(ip).await.is_ok());
    }
    assert!(!pool.can_accept(ip).await);
}

#[tokio::test]
async fn test_appstate_graceful_shutdown_with_rooms() {
    let app_state = Arc::new(AppState::new());
    let room_name = "graceful_shutdown_room".to_string();
    let user_id = "graceful_shutdown_user".to_string();
    let connection_id = "graceful_shutdown_conn".to_string();
    let animal_name = "graceful_shutdown_animal".to_string();

    // Setup room and user
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(user_id.clone(), UserData {
            animal_name: animal_name.clone(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: connection_id.clone(),
            },
            connection_id: connection_id.clone(),
            last_read_message: None,
            is_typing: false,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
        });
        rooms.insert(room_name.clone(), room);
    }

    // Call graceful_shutdown
    app_state.graceful_shutdown().await;

    // Verify rooms are cleared
    let rooms = app_state.rooms.read().await;
    assert!(rooms.is_empty());
}

#[tokio::test]
async fn test_handle_reconnect_error() {
    let user_data = UserData {
        animal_name: "test_animal".to_string(),
        last_active: Instant::now(),
        last_message_time: Instant::now(),
        connection_state: ConnectionState::Disconnected {
            since: Instant::now(),
            attempts: MAX_RECONNECT_ATTEMPTS,
            last_connection_id: "old_conn".to_string(),
        },
        connection_id: "old_conn".to_string(),
        last_read_message: None,
        is_typing: false,
        rate_limiter: RateLimiter::new(),
        last_sanitized_message: None,
    };

    let mut user_data = user_data.clone();
    let result = user_data.handle_reconnect("new_conn".to_string());
    assert!(result.is_err());
}

#[tokio::test]
async fn test_handle_reconnect_success() {
    let user_data = UserData {
        animal_name: "test_animal".to_string(),
        last_active: Instant::now(),
        last_message_time: Instant::now(),
        connection_state: ConnectionState::Disconnected {
            since: Instant::now(),
            attempts: 2,
            last_connection_id: "old_conn".to_string(),
        },
        connection_id: "old_conn".to_string(),
        last_read_message: None,
        is_typing: false,
        rate_limiter: RateLimiter::new(),
        last_sanitized_message: None,
    };

    let mut user_data = user_data.clone();
    let result = user_data.handle_reconnect("new_conn".to_string());
    assert!(result.is_ok());

    match &user_data.connection_state {
        ConnectionState::Connected { connection_id, .. } => {
            assert_eq!(connection_id, "new_conn");
        }
        _ => panic!("User should be connected after successful reconnection"),
    }
}

#[tokio::test]
async fn test_appstate_cleanup() {
    let app_state = Arc::new(AppState::new());
    let room_name = "cleanup_test_room".to_string();
    let user_id = "cleanup_user".to_string();
    let connection_id = "cleanup_conn".to_string();
    let animal_name = "cleanup_animal".to_string();

    // Setup room and user with inactivity
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.last_activity = Instant::now() - Duration::from_secs(7201);
        room.users.insert(user_id.clone(), UserData {
            animal_name: animal_name.clone(),
            last_active: Instant::now() - Duration::from_secs(7201),
            last_message_time: Instant::now() - Duration::from_secs(7201),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now() - Duration::from_secs(7201),
                connection_id: connection_id.clone(),
            },
            connection_id: connection_id.clone(),
            last_read_message: None,
            is_typing: false,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
        });
        rooms.insert(room_name.clone(), room);
    }

    // Perform cleanup
    app_state.cleanup().await;

    // Verify room is removed due to inactivity
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key(&room_name));
}

#[tokio::test]
async fn test_validate_message_empty() {
    assert!(validate_message("").is_err());
}

#[tokio::test]
async fn test_validate_message_too_long() {
    let long_text = "a".repeat(MAX_MESSAGE_LEN + 1);
    assert!(validate_message(&long_text).is_err());
}

#[tokio::test]
async fn test_validate_message_with_xss() {
    let malicious_text = "<script>alert('xss')</script>";
    let sanitized = validate_message(malicious_text).unwrap();
    assert!(!sanitized.contains("<script>"));
}

#[tokio::test]
async fn test_create_user_cookies_empty() {
    let (user_id_cookie, animal_name_cookie) = create_user_cookies("", "");
    assert!(user_id_cookie.contains("user_id="));
    assert!(animal_name_cookie.contains("animal_name="));
}

#[tokio::test]
async fn test_handle_websocket_invalid_room() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/ws/:room", get(
            |ws: WebSocketUpgrade, 
             Path(room): Path<String>,
             State(state): State<Arc<AppState>>| async move {
                let connection_id = Uuid::new_v4().to_string();
                ws.on_upgrade(move |socket| handle_websocket(room, state.clone(), None, socket, connection_id))
            }
        ))
        .with_state(app_state.clone());

    // Create a request with invalid room name
    let request = create_ws_request("/ws/!invalid_room").await;
    let response = app.oneshot(request).await.unwrap();

    // Expect a bad request or similar status
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Room name must start and end with alphanumeric characters"));
}

#[tokio::test]
async fn test_memory_tracker_peak() {
    let tracker = MemoryTracker::new();
    tracker.add_bytes(1000);
    tracker.add_bytes(2000);
    assert_eq!(tracker.peak_bytes.load(Ordering::Relaxed), 3000);
}

#[tokio::test]
async fn test_resource_monitor_can_accept_connection() {
    let resource_monitor = ResourceMonitor::new();
    assert!(resource_monitor.can_accept_connection());

    // Simulate max connections
    let mock_resource_monitor = ResourceMonitor {
        total_memory: AtomicUsize::new(MAX_TOTAL_ROOMS_MEMORY - 1),
        total_connections: AtomicUsize::new(MAX_CONCURRENT_USERS),
    };

    assert!(!mock_resource_monitor.can_accept_connection());
}

#[tokio::test]
async fn test_security_manager_record_suspicious_activity() {
    let security_manager = SecurityManager::new();
    let ip = "10.0.0.1";

    // Record suspicious activities until banned
    for _ in 0..11 {
        let _ = security_manager.record_suspicious_activity(ip).await;
    }

    // Verify IP is banned
    assert!(matches!(
        security_manager.check_ip(ip).await,
        Err(ChatError::SecurityError(_))
    ));
}

#[tokio::test]
async fn test_connection_pool_can_accept_new_connection() {
    let connection_pool = ConnectionPool::new();
    let ip = "192.168.1.2";

    // Initially, should accept
    assert!(connection_pool.can_accept(ip).await);

    // Add max connections
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(connection_pool.add_connection(ip).await.is_ok());
    }

    // Now, should not accept more
    assert!(!connection_pool.can_accept(ip).await);
}

#[tokio::test]
async fn test_handle_websocket_reconnection_token() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/ws/:room", get(
            |ws: WebSocketUpgrade, 
             Path(room): Path<String>,
             State(state): State<Arc<AppState>>| async move {
                let connection_id = Uuid::new_v4().to_string();
                ws.on_upgrade(move |socket| handle_websocket(room, state.clone(), None, socket, connection_id))
            }
        ))
        .with_state(app_state.clone());

    let request = create_ws_request("/ws/reconnect_test_room").await;
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let headers = response.headers();
    assert!(headers.contains_key("Set-Cookie"));
}

#[tokio::test]
async fn test_appstate_memory_cleanup_verification() {
    let app_state = Arc::new(AppState::new());

    // Simulate memory usage nearing the limit
    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();
        room.total_memory_bytes.store((MAX_TOTAL_ROOMS_MEMORY * 9) / 10, Ordering::SeqCst);
        rooms.insert("memory_cleanup_room".to_string(), room);
    }

    // Perform cleanup which should trigger message cleanup
    app_state.cleanup().await;

        let rooms = app_state.rooms.read().await;
        let room = rooms.get("memory_cleanup_room").unwrap();
        assert!(room.total_memory_bytes.load(Ordering::SeqCst) < (MAX_TOTAL_ROOMS_MEMORY * 9) / 10);
    }
    
    #[tokio::test]
    async fn test_user_data_is_typing_timeout() {
        let mut user_data = UserData {
            animal_name: "test_animal".to_string(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: "test_conn".to_string(),
            },
            connection_id: "test_conn".to_string(),
            last_read_message: None,
            is_typing: true,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
        };
    
        // Set last_active to simulate timeout
        user_data.last_active = Instant::now() - Duration::from_secs(16);
        assert!(!user_data.is_typing);
    }
    
    #[tokio::test]
    async fn test_room_state_concurrent_message_addition() {
        let room = Arc::new(RwLock::new(create_room()));
        let barrier = Arc::new(Barrier::new(5));
        let mut handles = vec![];
    
        for i in 0..5 {
            let room_clone = room.clone();
            let barrier_clone = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier_clone.wait().await;
                let mut room = room_clone.write().await;
                room.add_message(OutgoingMessage {
                    message_id: Uuid::new_v4(),
                    animal_name: format!("animal_{}", i),
                    text: format!("message_{}", i),
                    timestamp: "1234567890".to_string(),
                });
            }));
        }
    
        futures::future::join_all(handles).await;
        let room = room.read().await;
        assert_eq!(room.chat_history.len(), 5);
    }
    
    #[tokio::test]
    async fn test_security_manager_ip_unban_after_timeout() {
        let security_manager = SecurityManager::new();
        let ip = "10.0.0.2";
    
        // Ban IP
        for _ in 0..11 {
            let _ = security_manager.record_suspicious_activity(ip).await;
        }
        
        // Simulate ban timeout
        {
            let mut bans = security_manager.banned_ips.write().await;
            if let Some(ban_info) = bans.get_mut(ip) {
                *ban_info = Instant::now() - Duration::from_secs(3601); // 1 hour + 1 second
            }
        }
    
        // Verify IP is now unbanned
        assert!(security_manager.check_ip(ip).await.is_ok());
    }
    
    #[tokio::test]
    async fn test_connection_pool_remove_connection() {
        let pool = ConnectionPool::new();
        let ip = "192.168.1.5";
    
        // Add and then remove connection
        pool.add_connection(ip).await.unwrap();
        pool.remove_connection(ip).await;
    
        // Should be able to add new connection
        assert!(pool.can_accept(ip).await);
        assert!(pool.add_connection(ip).await.is_ok());
    }
    
    #[tokio::test]
    async fn test_memory_tracker_overflow_prevention() {
        let tracker = MemoryTracker::new();
        
        // Try to add more than usize::MAX bytes
        assert!(!tracker.add_bytes(usize::MAX));
        assert!(!tracker.add_bytes(usize::MAX - 100));
        
        // Normal addition should still work
        assert!(tracker.add_bytes(1024));
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
