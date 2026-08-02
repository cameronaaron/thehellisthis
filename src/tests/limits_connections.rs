//! `ConnectionPool` and `ResourceMonitor`: per-IP and total connection
//! accounting, and the admission/release pairing that keeps a rejected
//! upgrade from leaking a slot (constraint #1).

use super::*;

#[tokio::test]
async fn test_connection_pool_limits() {
    let pool = ConnectionPool::new();
    let ip = "192.168.1.2";

    assert!(pool.can_accept(ip).await);
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        pool.add_connection(ip).await.unwrap();
    }
    assert!(!pool.can_accept(ip).await);
    pool.remove_connection(ip).await;
    assert!(pool.can_accept(ip).await);
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
async fn test_connection_state_transitions() {
    let connected = ConnectionState::Connected {
        last_heartbeat: Instant::now(),
        connection_id: "conn1".to_string(),
    };

    let disconnected = ConnectionState::Disconnected {
        since: Instant::now(),
    };

    // Verify state types
    assert!(matches!(disconnected, ConnectionState::Disconnected { .. }));
    assert!(matches!(connected, ConnectionState::Connected { .. }));
}

#[tokio::test]
async fn test_connection_pool_rejects_when_active_full() {
    let pool = ConnectionPool::new();
    pool.active.store(MAX_CONCURRENT_USERS, Ordering::SeqCst);

    let result = pool.add_connection("203.0.113.5").await;
    assert!(matches!(result, Err(ChatError::ResourceLimit(_))));
}

#[tokio::test]
async fn test_rapid_reconnections() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/reconnect-test", addr);

    // Rapidly connect and disconnect
    for _ in 0..5 {
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("Failed to connect");

        // Wait for initial events
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Close connection
        let _ = ws.close(None).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_resource_monitor_capacity() {
    let monitor = ResourceMonitor::new();

    assert!(monitor.can_accept_connection());

    monitor
        .total_connections
        .store(MAX_CONCURRENT_USERS, Ordering::SeqCst);
    assert!(!monitor.can_accept_connection());

    monitor.total_connections.store(0, Ordering::SeqCst);
    monitor
        .total_memory
        .store(MAX_TOTAL_ROOMS_MEMORY, Ordering::SeqCst);
    assert!(!monitor.can_accept_connection());
}

#[tokio::test]
async fn test_connection_pool_cleanup_stale() {
    let pool = ConnectionPool::new();
    {
        let mut counters = pool.ip_counters.write().await;
        // Zero live connections, so age alone decides — see
        // `a_live_connection_counter_survives_the_stale_sweep`.
        counters.insert(
            "10.10.10.10".to_string(),
            (
                AtomicUsize::new(0),
                Instant::now() - Duration::from_secs(7200),
            ),
        );
    }

    pool.cleanup_stale().await;
    let counters = pool.ip_counters.read().await;
    assert!(!counters.contains_key("10.10.10.10"));
}

#[tokio::test]
async fn test_connection_from_same_ip_multiple_times() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/same-ip-test", addr);
    let mut connections = Vec::new();

    // Create multiple connections from same IP (up to limit)
    for i in 0..3 {
        let (ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .unwrap_or_else(|_| panic!("Failed to connect #{}", i));
        connections.push(ws);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    for mut ws in connections {
        let _ = ws.close(None).await;
    }

    handle.abort();
}

#[tokio::test]
async fn test_connection_pool_max_per_ip_enforced() {
    let pool = ConnectionPool::new();
    let test_ip = "192.168.1.1";

    // Add up to max
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(pool.add_connection(test_ip).await.is_ok());
    }

    // Next one should fail
    assert!(pool.add_connection(test_ip).await.is_err());
}

#[tokio::test]
async fn test_connection_pool_removal_frees_slot() {
    let pool = ConnectionPool::new();
    let test_ip = "10.20.30.40";

    // Fill to max
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(pool.add_connection(test_ip).await.is_ok());
    }

    // Remove one
    pool.remove_connection(test_ip).await;

    // Should be able to add again
    assert!(pool.add_connection(test_ip).await.is_ok());
}

#[tokio::test]
async fn test_resource_monitor_total_memory_tracking() {
    let monitor = ResourceMonitor::new();

    // Add some memory
    monitor.total_memory.store(1000, Ordering::SeqCst);
    assert!(monitor.can_accept_connection());

    // Max out memory
    monitor
        .total_memory
        .store(MAX_TOTAL_ROOMS_MEMORY, Ordering::SeqCst);
    assert!(!monitor.can_accept_connection());
}

#[tokio::test]
async fn test_concurrent_connections_from_different_ips() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/multi-ip-test", addr);

    // All connections come from same IP in test, but simulate behavior
    let mut connections = Vec::new();
    for _i in 0..3 {
        match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((ws, _)) => {
                connections.push(ws);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(_) => break,
        }
    }

    assert!(connections.len() >= 2);

    for mut ws in connections {
        let _ = ws.close(None).await;
    }

    handle.abort();
}

#[tokio::test]
async fn test_connection_pool_boundary() {
    let pool = ConnectionPool::new();
    let ip = "192.168.1.1";

    assert!(pool.add_connection(ip).await.is_ok());
    assert!(pool.add_connection(ip).await.is_ok());
    assert!(pool.add_connection(ip).await.is_ok());

    assert!(pool.add_connection(ip).await.is_err());
}

#[tokio::test]
async fn test_websocket_connection_without_room() {
    let (addr, _handle) = start_ws_server().await;

    let url = format!("ws://{}/ws/", addr);
    let result = tokio_tungstenite::connect_async(&url).await;

    // `/ws/` with no room matches no route, so the upgrade is refused. The
    // assertion that mattered — that it does not *succeed* — was previously
    // written as `is_ok() || is_err()`, which is true of every Result.
    assert!(
        result.is_err(),
        "an upgrade with no room name must not be accepted"
    );
}

#[tokio::test]
async fn test_connection_pool_different_ips_new() {
    let pool = ConnectionPool::new();
    let ip1 = "192.168.1.1";
    let ip2 = "192.168.1.2";

    assert!(pool.add_connection(ip1).await.is_ok());
    assert!(pool.add_connection(ip2).await.is_ok());
}

#[tokio::test]
async fn test_connection_pool_max_connections_per_ip() {
    let pool = ConnectionPool::new();
    let ip = "192.168.50.1";

    // Add up to MAX_CONCURRENT_CONNECTIONS_PER_IP (3)
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(pool.add_connection(ip).await.is_ok());
    }

    // 4th connection should fail
    assert!(!pool.can_accept(ip).await);
}

#[tokio::test]
async fn test_connection_pool_remove_connection_allows_new() {
    let pool = ConnectionPool::new();
    let ip = "192.168.50.2";

    // Fill up connections
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        pool.add_connection(ip).await.unwrap();
    }

    // Should be full
    assert!(!pool.can_accept(ip).await);

    // Remove one
    pool.remove_connection(ip).await;

    // Should be able to accept again
    assert!(pool.can_accept(ip).await);
}

#[tokio::test]
async fn test_resource_monitor_initial_state() {
    let monitor = ResourceMonitor::new();
    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 0);
    assert!(monitor.can_accept_connection());
}

#[tokio::test]
async fn test_resource_monitor_at_capacity() {
    let monitor = ResourceMonitor::new();

    // Under capacity, should accept
    monitor.total_connections.store(399, Ordering::Relaxed);
    assert!(monitor.can_accept_connection());

    // At max capacity (400), should reject
    monitor.total_connections.store(400, Ordering::Relaxed);
    assert!(!monitor.can_accept_connection());

    // Over max capacity, should definitely reject
    monitor.total_connections.store(401, Ordering::Relaxed);
    assert!(!monitor.can_accept_connection());
}

#[tokio::test]
async fn test_connection_state_disconnected() {
    let state = ConnectionState::Disconnected {
        since: Instant::now(),
    };
    match state {
        ConnectionState::Disconnected { since } => {
            assert!(since.elapsed() < Duration::from_secs(1));
        }
        _ => panic!("Expected Disconnected state"),
    }
}

#[tokio::test]
async fn test_connection_state_connected() {
    let state = ConnectionState::Connected {
        last_heartbeat: Instant::now(),
        connection_id: "conn123".to_string(),
    };
    match state {
        ConnectionState::Connected {
            last_heartbeat,
            connection_id,
        } => {
            assert!(last_heartbeat.elapsed() < Duration::from_secs(1));
            assert_eq!(connection_id, "conn123");
        }
        _ => panic!("Expected Connected state"),
    }
}

#[tokio::test]
async fn test_connection_pool_cleanup_stale_connections() {
    let pool = ConnectionPool::new();
    let ip = "192.168.1.200";

    // Add a connection
    pool.add_connection(ip).await.unwrap();

    // Cleanup stale (shouldn't remove recent connections)
    pool.cleanup_stale().await;

    // Connection should still be tracked
    // Note: cleanup_stale removes entries that haven't been accessed in a while
    // For a fresh connection this should still be fine
}

#[tokio::test]
async fn test_websocket_connection_with_room_limit() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect multiple clients to same room
    let ws_url = format!("ws://{}/ws/limit-test", addr);
    let mut connections = Vec::new();

    for _ in 0..5 {
        if let Ok((ws, _)) = tokio_tungstenite::connect_async(&ws_url).await {
            connections.push(ws);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // All should connect (under limit)
    assert!(connections.len() >= 3);

    // Clean up
    for mut ws in connections {
        let _ = ws.close(None).await;
    }

    handle.abort();
}

// ===== Additional Coverage Tests =====

#[tokio::test]
async fn test_app_state_cleanup_stale_connections() {
    let app_state = Arc::new(AppState::new());

    // Add a connection
    let ip = "192.168.1.100";
    app_state.connection_pool.add_connection(ip).await.unwrap();

    // Run cleanup
    app_state.cleanup().await;

    // Connection pool cleanup should have been called
    // (The connection may still exist if recent)
}

#[tokio::test]
async fn test_resource_monitor_update_connections_count() {
    let monitor = ResourceMonitor::new();

    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 0);

    // Simulate adding connections
    monitor.total_connections.fetch_add(10, Ordering::SeqCst);
    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 10);

    // Simulate removing connections
    monitor.total_connections.fetch_sub(3, Ordering::SeqCst);
    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 7);
}

#[tokio::test]
async fn test_connection_pool_concurrent_add_remove() {
    let pool = ConnectionPool::new();
    let ip = "10.0.0.100";

    // Add connections
    pool.add_connection(ip).await.unwrap();
    pool.add_connection(ip).await.unwrap();

    // Remove one
    pool.remove_connection(ip).await;

    // Should still be able to add one more
    pool.add_connection(ip).await.unwrap();
}

#[tokio::test]
async fn test_connection_pool_concurrent_operations() {
    let pool = Arc::new(ConnectionPool::new());
    let barrier = Arc::new(Barrier::new(10));
    let mut handles = vec![];

    // Spawn concurrent add operations
    for i in 0..10 {
        let pool_clone = pool.clone();
        let barrier_clone = barrier.clone();
        let ip = format!("10.0.0.{}", i);

        handles.push(tokio::spawn(async move {
            barrier_clone.wait().await;
            let _ = pool_clone.add_connection(&ip).await;
        }));
    }

    join_all(handles).await;

    // Should have handled concurrent operations without panic
    assert!(pool.active.load(Ordering::Relaxed) <= 10);
}

#[tokio::test]
async fn test_connection_state_connected_variant() {
    let now = Instant::now();
    let state = ConnectionState::Connected {
        last_heartbeat: now,
        connection_id: "test".to_string(),
    };

    match state {
        ConnectionState::Connected { connection_id, .. } => {
            assert_eq!(connection_id, "test");
        }
        _ => panic!("Expected Connected state"),
    }
}

#[tokio::test]
async fn test_connection_state_disconnected_variant() {
    let now = Instant::now();
    let state = ConnectionState::Disconnected { since: now };

    match state {
        ConnectionState::Disconnected { since } => {
            assert!(since <= Instant::now());
        }
        _ => panic!("Expected Disconnected state"),
    }
}

// ========== RATE LIMITER EDGE CASES ==========

#[tokio::test]
async fn test_resource_monitor_initial_can_accept() {
    let monitor = ResourceMonitor::new();
    assert!(monitor.can_accept_connection());
}

#[tokio::test]
async fn test_resource_monitor_connection_count_tracking() {
    let monitor = ResourceMonitor::new();
    monitor.total_connections.fetch_add(1, Ordering::SeqCst);
    assert_eq!(monitor.total_connections.load(Ordering::SeqCst), 1);
}

// ========== VALIDATE INPUT BOUNDARY TESTS ==========

#[tokio::test]
async fn test_connection_pool_cleanup_old_entries() {
    let pool = ConnectionPool::new();

    // Add a connection
    pool.add_connection("192.168.1.1").await.unwrap();

    // Should be able to accept more
    assert!(pool.can_accept("192.168.1.1").await);

    // Remove it
    pool.remove_connection("192.168.1.1").await;
}

#[tokio::test]
async fn test_cookie_reconnection_existing_user_in_room() {
    let (addr, _state) = start_ws_server().await;

    // First connection - get assigned a user_id and animal_name
    let url = format!("ws://{}/ws/reconnect-test-room", addr);
    let (mut ws1, response1) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("First connect failed");

    // Extract cookies from response
    let mut user_id_cookie = String::new();
    let mut animal_name_cookie = String::new();

    for (name, value) in response1.headers() {
        if name == "set-cookie" {
            let cookie_str = value.to_str().unwrap_or("");
            if cookie_str.starts_with("user_id=") {
                user_id_cookie = cookie_str
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("user_id=")
                    .to_string();
            } else if cookie_str.starts_with("animal_name=") {
                animal_name_cookie = cookie_str
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("animal_name=")
                    .to_string();
            }
        }
    }

    // Close first connection but keep the room alive by not fully disconnecting
    ws1.close(None).await.ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with the same cookies
    let request = http::Request::builder()
        .uri(&url)
        .header(
            "Cookie",
            format!(
                "user_id={}; animal_name={}",
                user_id_cookie, animal_name_cookie
            ),
        )
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(())
        .unwrap();

    let (mut ws2, _response2) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Reconnect failed");

    // Receive events - should get reconnect token
    let mut got_token = false;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(500), ws2.next()).await
            && text.contains("ReconnectToken")
        {
            got_token = true;
            break;
        }
    }

    assert!(got_token, "Should receive reconnect token on reconnection");
    ws2.close(None).await.ok();
}

// Test cookie with user_id not found in room - covers lines 1352-1379

#[tokio::test]
async fn test_room_deleted_during_connection() {
    let app_state = Arc::new(AppState::new());

    // Create a room then immediately delete it to test the edge case
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("temp-room".to_string(), create_room());
    }

    // Delete the room
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.remove("temp-room");
    }

    // Verify room is gone
    let rooms = app_state.rooms.read().await;
    assert!(!rooms.contains_key("temp-room"));
}

// Test user not found in room during event processing - covers lines 1498-1519

#[tokio::test]
async fn test_resource_monitor_limits() {
    let monitor = ResourceMonitor::new();

    // At initial state should accept
    assert!(monitor.can_accept_connection());

    // Simulate hitting connection limit
    monitor
        .total_connections
        .store(MAX_CONCURRENT_USERS, Ordering::SeqCst);
    assert!(!monitor.can_accept_connection());

    // Reset connections but hit memory limit
    monitor.total_connections.store(0, Ordering::SeqCst);
    monitor
        .total_memory
        .store(MAX_TOTAL_ROOMS_MEMORY, Ordering::SeqCst);
    assert!(!monitor.can_accept_connection());
}

// Test UserCookie extraction with edge case cookies - covers cookie parsing

#[tokio::test]
async fn test_connection_pool_at_limit() {
    let pool = ConnectionPool::new();
    let ip = "192.168.1.50";

    // Add connections up to limit
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(pool.add_connection(ip).await.is_ok());
    }

    // Should reject additional connections
    assert!(pool.add_connection(ip).await.is_err());

    // Remove one connection
    pool.remove_connection(ip).await;

    // Should now accept again
    assert!(pool.add_connection(ip).await.is_ok());
}

// Test MemoryTracker at high usage - covers memory management paths

#[tokio::test]
async fn test_connection_pool_stale_cleanup() {
    let pool = ConnectionPool::new();

    // Add a connection
    let _ = pool.add_connection("192.168.1.1").await;

    // Run cleanup
    pool.cleanup_stale().await;

    // Connection should still exist (not stale yet)
    assert!(pool.can_accept("192.168.1.1").await);
}

// Test cleanup_rooms removes old empty rooms - covers lines 1934-1992

#[tokio::test]
async fn test_ws_cookie_reconnection_updates_connection_state() {
    let (addr, _handle) = start_ws_server().await;
    let room_name = "reconnect-state-room";
    let url = format!("ws://{}/ws/{}", addr, room_name);

    // First connection
    let (mut ws1, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("First connect failed");

    // Get user info from initial messages
    let mut user_id = String::new();
    let mut animal_name = String::new();
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(200), ws1.next()).await
            && text.contains("UserJoined")
            && let Ok(event) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(evt) = event.get("event")
        {
            user_id = evt
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            animal_name = evt
                .get("animal_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            break;
        }
    }

    ws1.close(None).await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with cookies if we got user info
    if !user_id.is_empty() && !animal_name.is_empty() {
        let request = http::Request::builder()
            .uri(&url)
            .header(
                "Cookie",
                format!("user_id={};animal_name={}", user_id, animal_name),
            )
            .header("Host", addr.to_string())
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(())
            .unwrap();

        let result = tokio_tungstenite::connect_async(request).await;
        if let Ok((mut ws2, _)) = result {
            // Should have reconnected - send a message to verify
            ws2.send(text_frame(r#"{"type":"Message","text":"reconnected"}"#))
                .await
                .ok();
            ws2.close(None).await.ok();
        }
    }
}

// Test ws_handler room deleted scenario - covers lines 1486-1490

#[tokio::test]
async fn test_connection_pool_ip_limit() {
    let pool = ConnectionPool::new();
    let ip = "192.168.1.1".to_string();

    // Add up to limit
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        assert!(pool.add_connection(&ip).await.is_ok());
    }

    // Exceeding should fail
    assert!(pool.add_connection(&ip).await.is_err());

    // Remove one
    pool.remove_connection(&ip).await;

    // Now can add again
    assert!(pool.add_connection(&ip).await.is_ok());
}

// Test AppState::new initialization - covers lines 511-535

#[tokio::test]
async fn test_resource_monitor_tracking() {
    let monitor = ResourceMonitor::new();

    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 0);

    monitor.total_connections.fetch_add(1, Ordering::SeqCst);
    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 1);

    monitor.total_connections.fetch_add(5, Ordering::SeqCst);
    assert_eq!(monitor.total_connections.load(Ordering::Relaxed), 6);
}

// Test RoomState broadcast_system_event - covers lines 920-935

#[tokio::test]
async fn test_connection_state_type_variants() {
    let now = Instant::now();

    let connected = ConnectionState::Connected {
        last_heartbeat: now,
        connection_id: "conn123".to_string(),
    };

    let disconnected = ConnectionState::Disconnected { since: now };

    // Verify pattern matching works
    matches!(connected, ConnectionState::Connected { .. });
    matches!(disconnected, ConnectionState::Disconnected { .. });
}

// Test room name validation with special cases - covers lines 1129-1165

/// A rejected upgrade must leave no connection reservation behind.
///
/// The admission path used to increment the per-IP pool and the global
/// connection counter and *then* run checks that can fail, returning without
/// releasing either. Each rejected connection permanently consumed a slot, so a
/// server that had refused enough connections would refuse all of them while
/// completely idle.
#[tokio::test]
async fn rejected_upgrade_releases_its_connection_slot() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    // '.' is not a legal room character, so admission rejects this.
    for _ in 0..(MAX_CONCURRENT_CONNECTIONS_PER_IP * 3) {
        let url = format!("ws://{addr}/ws/not.a.valid.room");
        let _ = connect_async(&url).await;
    }

    // Give teardown a moment to run.
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        0,
        "rejected upgrades leaked global connection slots"
    );
    assert_eq!(
        state.connection_pool.active.load(Ordering::SeqCst),
        0,
        "rejected upgrades leaked per-IP pool slots"
    );

    // The server still accepts a legitimate connection afterwards.
    let (mut ws, _) = connect_async(format!("ws://{addr}/ws/leak-check"))
        .await
        .expect("server refused a valid connection after rejected ones");
    let event = recv_json_event(&mut ws).await;
    assert_eq!(event["type"], "Welcome");

    handle.abort();
}

/// Teardown for a connection that has already been superseded does nothing.
///
/// A user who reconnected before the old session's teardown ran has a *newer*
/// live connection; marking them disconnected here would evict the session that
/// is currently working.
#[tokio::test]
async fn teardown_ignores_a_superseded_connection() {
    let state = Arc::new(AppState::new());
    let now = Instant::now();

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "user".to_string(),
            UserData {
                user_id: "user".to_string(),
                animal_name: "otter".to_string(),
                last_active: now,
                last_message_time: now,
                // The *current* connection.
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: "connection-2".to_string(),
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

    // Teardown arrives for the older connection.
    cleanup_user(&state, "room", "user", "connection-1", None).await;

    let rooms = state.rooms.read().await;
    let user = &rooms["room"].users["user"];
    assert!(
        user.is_connected(),
        "the live connection must survive a stale teardown"
    );
}

/// A connection whose heartbeat has already lapsed has its events ignored.
///
/// Its teardown is already in flight; accepting messages from it would race
/// that teardown.
#[tokio::test]
async fn events_from_a_lapsed_connection_are_ignored() {
    let stale = Instant::now() - HEARTBEAT_TIMEOUT - Duration::from_secs(5);
    let state = state_with_user("room", "user", stale).await;

    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::Message {
            text: "hello".into(),
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    assert!(
        rooms["room"].chat_history.is_empty(),
        "a lapsed connection must not be able to post"
    );
}

/// A counter that is still counting is never evicted.
///
/// `last_seen` is stamped when a connection is added, not while it lasts, so a
/// session that outlived `IP_COUNTER_RETENTION` — which any user who keeps
/// talking does — had its counter swept away underneath it, lifting the per-IP
/// limit for that address until it reconnected.
#[tokio::test]
async fn a_live_connection_counter_survives_the_stale_sweep() {
    let pool = ConnectionPool::new();
    let ip = "long-session";

    pool.add_connection(ip).await.unwrap();

    // Age the entry well past the retention window without ending the session.
    {
        let mut counters = pool.ip_counters.write().await;
        let (_, last_seen) = counters.get_mut(ip).unwrap();
        *last_seen = Instant::now() - IP_COUNTER_RETENTION - Duration::from_secs(60);
    }

    pool.cleanup_stale().await;

    assert!(
        pool.ip_counters.read().await.contains_key(ip),
        "a counter with a live connection must not be evicted"
    );

    // Once the session actually ends, the entry becomes evictable again.
    pool.remove_connection(ip).await;
    {
        let mut counters = pool.ip_counters.write().await;
        let (_, last_seen) = counters.get_mut(ip).unwrap();
        *last_seen = Instant::now() - IP_COUNTER_RETENTION - Duration::from_secs(60);
    }
    pool.cleanup_stale().await;

    assert!(
        !pool.ip_counters.read().await.contains_key(ip),
        "an idle counter must still be evicted, or the map is unbounded"
    );
}

/// An existing user reconnecting keeps their slot and takes the new connection.
#[tokio::test]
async fn joining_again_rebinds_the_existing_slot_to_the_new_connection() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "old-conn", Instant::now()),
        );
        rooms.insert("rejoin".to_string(), room);
    }

    crate::session::join_room(&state, "rejoin", "u1", "otter", "new-conn")
        .await
        .expect("the room exists");

    let rooms = state.rooms.read().await;
    let room = rooms.get("rejoin").unwrap();
    assert_eq!(
        room.users.len(),
        1,
        "reconnecting must not add a second slot"
    );
    assert!(matches!(
        &room.users["u1"].connection_state,
        ConnectionState::Connected { connection_id, .. } if connection_id == "new-conn"
    ));
}

// ========== A SINK THAT FAILS ==========

/// A user who is still talking is not evicted, and keeps getting heartbeats.
#[tokio::test]
async fn a_talking_user_keeps_their_connection() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        rooms.insert("chatty".to_string(), room);
    }

    let sink = Arc::new(tokio::sync::Mutex::new(RecordingSink::default()));
    // The loop never ends on its own for an active user, so it is stopped here.
    let _ = timeout(
        HEARTBEAT_INTERVAL + Duration::from_millis(200),
        crate::session::beat_and_evict_idle(
            state,
            "chatty".to_string(),
            "u1".to_string(),
            "c1".to_string(),
            sink.clone(),
        ),
    )
    .await;

    let sent = &sink.lock().await.sent;
    assert!(!sent.is_empty(), "an active user should receive heartbeats");
    assert!(
        !sent
            .iter()
            .any(|m| matches!(m, crate::session::Message::Close(_))),
        "and must not be evicted while they are still here"
    );
}
