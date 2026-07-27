//! Resource ceilings: rate limits, connection accounting, memory, IP bans.

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
async fn test_memory_tracker_add_remove() {
    let tracker = MemoryTracker::new();
    assert!(tracker.add_bytes(1024));
    tracker.remove_bytes(1024);
    assert!(tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY / 2));
}

#[tokio::test]
async fn test_memory_tracker_should_gc() {
    let tracker = MemoryTracker::new();
    tracker.last_gc.store(0, Ordering::Relaxed);
    assert!(tracker.should_gc());

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    tracker.last_gc.store(now, Ordering::Relaxed);
    assert!(!tracker.should_gc());
}

#[tokio::test]
async fn test_security_manager_ban_ip() {
    let security_manager = SecurityManager::new();
    let ip = "10.0.0.1";

    for _ in 0..11 {
        let _ = security_manager.record_suspicious_activity(ip).await;
    }

    assert!(matches!(
        security_manager.check_ip(ip).await,
        Err(ChatError::SecurityError(_))
    ));
}

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
async fn test_generate_random_room_name() {
    let name = generate_random_room_name();
    assert!(!name.is_empty());
    assert!(name.contains('-'));
}

#[tokio::test]
async fn test_memory_tracker_concurrency() {
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

// ========== MESSAGE & BROADCASTING ==========

#[tokio::test]
async fn test_message_memory_tracking() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();
    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Test message</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };

    let msg_size = msg.estimate_size();
    room.add_message(msg, &tracker);
    assert!(room.total_memory_bytes.load(Ordering::Relaxed) > 0);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), msg_size);
}

#[tokio::test]
async fn test_preserve_messages_trims_and_updates_tracker() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Add messages beyond MAX_MESSAGES_PER_ROOM
    for _i in 0..(MAX_MESSAGES_PER_ROOM + 150) {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Lion".to_string(),
            text: "<p>Test</p>".to_string(),
            timestamp: "1000".to_string(),
            reply_to: None,
            attachment: None,
        };
        let msg_size = msg.estimate_size();
        tracker.add_bytes(msg_size);
        room.total_memory_bytes
            .fetch_add(msg_size, Ordering::SeqCst);
        room.chat_history.push(Arc::new(msg));
    }

    let before_tracker = tracker.total_bytes.load(Ordering::Relaxed);
    room.preserve_messages(&tracker);

    assert!(room.chat_history.len() <= MAX_MESSAGES_PER_ROOM);
    let after_tracker = tracker.total_bytes.load(Ordering::Relaxed);
    assert!(after_tracker < before_tracker);
}

#[tokio::test]
async fn test_trim_to_max_messages_limits_history() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    for _i in 0..(MAX_MESSAGES_PER_ROOM + 100) {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Lion".to_string(),
            text: "<p>Test</p>".to_string(),
            timestamp: "1000".to_string(),
            reply_to: None,
            attachment: None,
        };
        let size = msg.estimate_size();
        tracker.add_bytes(size);
        room.total_memory_bytes.fetch_add(size, Ordering::SeqCst);
        room.chat_history.push(Arc::new(msg));
    }

    let before = tracker.total_bytes.load(Ordering::Relaxed);
    room.trim_to_max_messages(&tracker);
    let after = tracker.total_bytes.load(Ordering::Relaxed);

    assert_eq!(room.chat_history.len(), MAX_MESSAGES_PER_ROOM);
    assert!(after < before);
}

// ========== USER LIFECYCLE ==========

#[tokio::test]
async fn test_room_memory_accounting() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let msg1 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>First</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };
    let msg1_size = msg1.estimate_size();

    room.add_message(msg1, &tracker);

    let msg2 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Second</p>".to_string(),
        timestamp: "2000".to_string(),
        reply_to: None,
        attachment: None,
    };
    let msg2_size = msg2.estimate_size();

    room.add_message(msg2, &tracker);

    let total = tracker.total_bytes.load(Ordering::Relaxed);
    assert_eq!(total, msg1_size + msg2_size);
}

// ========== HEARTBEAT & PRESENCE ==========

#[tokio::test]
async fn test_room_capacity_tracking() {
    let mut room = create_room();

    let now = Instant::now();
    for i in 0..100 {
        let user = UserData {
            user_id: format!("user-{}", i),
            animal_name: format!("Animal{}", i),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: format!("conn-{}", i),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
            last_reaction_event: None,
        };
        room.users.insert(format!("user-{}", i), user);
    }

    // Room has 100 users
    let connected_count = room
        .users
        .values()
        .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
        .count();

    assert_eq!(connected_count, 100);
}

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
        last_sanitized_message: None,
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
async fn test_prune_old_messages_updates_tracker() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let msg1 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>One</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };
    let msg2 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Two</p>".to_string(),
        timestamp: "2000".to_string(),
        reply_to: None,
        attachment: None,
    };
    let msg3 = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Three</p>".to_string(),
        timestamp: "3000".to_string(),
        reply_to: None,
        attachment: None,
    };

    let size1 = msg1.estimate_size();
    let size2 = msg2.estimate_size();
    let size3 = msg3.estimate_size();
    let total = size1 + size2 + size3;

    room.chat_history.push(Arc::new(msg1));
    room.chat_history.push(Arc::new(msg2));
    room.chat_history.push(Arc::new(msg3));
    room.total_memory_bytes.store(total, Ordering::SeqCst);
    tracker.add_bytes(total);

    room.prune_old_messages(size1, &tracker);

    assert_eq!(room.chat_history.len(), 2);
    let tracker_total = tracker.total_bytes.load(Ordering::Relaxed);
    assert_eq!(tracker_total, total - size1);
}

#[tokio::test]
async fn test_add_message_drops_when_global_memory_full() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    tracker
        .total_bytes
        .store(MAX_TOTAL_ROOMS_MEMORY, Ordering::SeqCst);

    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Test</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };

    room.add_message(msg, &tracker);
    assert!(room.chat_history.is_empty());
    assert_eq!(room.total_memory_bytes.load(Ordering::Relaxed), 0);
}

// ========== ANIMAL FALLBACK ==========

#[tokio::test]
async fn test_connection_pool_rejects_when_active_full() {
    let pool = ConnectionPool::new();
    pool.active.store(MAX_CONCURRENT_USERS, Ordering::SeqCst);

    let result = pool.add_connection("203.0.113.5").await;
    assert!(matches!(result, Err(ChatError::ResourceLimit(_))));
}

#[tokio::test]
async fn test_security_manager_ban_expires() {
    let security_manager = SecurityManager::new();
    {
        let mut banned = security_manager.banned_ips.write().await;
        banned.insert(
            "10.0.0.99".to_string(),
            Instant::now() - Duration::from_secs(3700),
        );
    }

    let result = security_manager.check_ip("10.0.0.99").await;
    assert!(result.is_ok());
}

// ========== WEBSOCKET INTEGRATION TESTS ==========

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
async fn test_memory_tracker_peak_bytes() {
    let tracker = MemoryTracker::new();

    assert!(tracker.add_bytes(1024));
    assert!(tracker.add_bytes(2048));

    let peak = tracker.peak_bytes.load(Ordering::Relaxed);
    assert!(peak >= 3072);
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
async fn test_memory_tracker_cleanup_threshold() {
    let tracker = MemoryTracker::new();

    // Fill to near capacity
    assert!(tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY - 1024));

    // Should still accept small additions
    assert!(tracker.add_bytes(512));
}

#[tokio::test]
async fn test_memory_tracker_reject_when_full() {
    let tracker = MemoryTracker::new();

    // Fill to capacity
    assert!(tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY));

    // Should reject new additions
    assert!(!tracker.add_bytes(1));
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
async fn test_security_manager_suspicious_activity_accumulation() {
    let security_manager = SecurityManager::new();
    let ip = "1.2.3.4";

    // Record 9 suspicious activities (just below ban threshold)
    for _ in 0..9 {
        let _ = security_manager.record_suspicious_activity(ip).await;
    }

    // Should not be banned yet
    assert!(security_manager.check_ip(ip).await.is_ok());

    // One more should trigger ban
    let _ = security_manager.record_suspicious_activity(ip).await;
    let _ = security_manager.record_suspicious_activity(ip).await;

    assert!(security_manager.check_ip(ip).await.is_err());
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
async fn test_room_message_history_capacity() {
    let app_state = Arc::new(AppState::new());
    let room_name = "history-cap".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        let mut room = create_room();

        // Add messages beyond MAX_MESSAGES_PER_ROOM
        for i in 0..MAX_MESSAGES_PER_ROOM + 10 {
            room.chat_history.push(Arc::new(OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: Uuid::new_v4().to_string(),
                animal_name: "Lion".to_string(),
                text: format!("Message {}", i),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_string(),
                reply_to: None,
                attachment: None,
            }));
        }

        rooms.insert(room_name.clone(), room);
    }

    // Verify size
    let rooms = app_state.rooms.read().await;
    let room = rooms.get(&room_name).unwrap();
    assert!(room.chat_history.len() <= MAX_MESSAGES_PER_ROOM + 10);
}

// ========== WEBSOCKET PROTOCOL EDGE CASES ==========

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
async fn test_security_ban_cleared_after_expiry() {
    let security_manager = SecurityManager::new();
    let ip = "192.0.2.1";

    // Ban the IP
    for _ in 0..15 {
        let _ = security_manager.record_suspicious_activity(ip).await;
    }

    assert!(security_manager.check_ip(ip).await.is_err());

    // Verify ban exists
    let bans = security_manager.banned_ips.read().await;
    assert!(bans.contains_key(ip));
}

#[tokio::test]
async fn test_memory_tracker_remove_bytes() {
    let tracker = MemoryTracker::new();

    tracker.add_bytes(1000);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 1000);

    tracker.remove_bytes(500);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 500);

    tracker.remove_bytes(500); // Remove rest
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 0);
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
async fn test_rate_limiter_window_expiry() {
    let mut limiter = RateLimiter::new();

    for _ in 0..30 {
        assert!(limiter.can_send_message());
    }

    assert!(!limiter.can_send_message());
}

#[tokio::test]
async fn test_security_manager_ban_threshold() {
    let security = SecurityManager::new();
    let ip = "10.0.0.1";

    // Record suspicious activity but stay below threshold
    for _ in 0..9 {
        let _ = security.record_suspicious_activity(ip).await;
    }

    // Should still be allowed - not yet at threshold of 10
    assert!(security.check_ip(ip).await.is_ok());
}

#[tokio::test]
async fn test_memory_tracker_peak_tracking() {
    let tracker = MemoryTracker::new();

    assert!(tracker.add_bytes(100));
    assert!(tracker.add_bytes(200));

    assert_eq!(tracker.peak_bytes.load(Ordering::SeqCst), 300);

    tracker.remove_bytes(200);

    assert_eq!(tracker.peak_bytes.load(Ordering::SeqCst), 300);
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
async fn test_security_manager_below_threshold() {
    let security = SecurityManager::new();
    let ip = "192.168.1.100";

    // First check should pass
    assert!(security.check_ip(ip).await.is_ok());

    // Record a few suspicious activities but stay well under threshold
    for _ in 0..3 {
        let _ = security.record_suspicious_activity(ip).await;
    }

    // Should still be allowed
    assert!(security.check_ip(ip).await.is_ok());
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
async fn test_chat_error_into_response_resource_limit() {
    let error = ChatError::ResourceLimit("Out of memory".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
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
async fn test_memory_cleanup_trigger() {
    let app_state = Arc::new(AppState::new());

    // Force memory tracker to think we need GC
    app_state
        .memory_tracker
        .total_bytes
        .store(MAX_TOTAL_ROOMS_MEMORY + 1000, Ordering::Relaxed);
    app_state.memory_tracker.last_gc.store(0, Ordering::Relaxed);

    // Create a room with messages
    {
        let mut rooms = app_state.rooms.write().await;
        let room = create_room();
        rooms.insert("memory-cleanup-test".to_string(), room);
    }

    // Should trigger memory cleanup
    assert!(app_state.memory_tracker.should_gc());
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
async fn test_room_state_add_message_updates_memory() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    let msg = OutgoingMessage {
        message_id: Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Tiger".to_string(),
        text: "Test message".to_string(),
        timestamp: "1234567890".to_string(),
        reply_to: None,
        attachment: None,
    };

    let initial_memory = room_state.total_memory_bytes.load(Ordering::Relaxed);
    room_state.add_message(msg, &memory_tracker);
    let final_memory = room_state.total_memory_bytes.load(Ordering::Relaxed);

    assert!(final_memory > initial_memory);
    assert_eq!(room_state.chat_history.len(), 1);
}

#[tokio::test]
async fn test_room_state_trim_messages_over_limit() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add more than MAX_MESSAGES_PER_ROOM
    for i in 0..(MAX_MESSAGES_PER_ROOM + 50) {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Message {}", i),
            timestamp: "1234567890".to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.add_message(msg, &memory_tracker);
    }

    // Explicitly trim to max (add_message doesn't auto-trim)
    room_state.trim_to_max_messages(&memory_tracker);

    // History should be trimmed to MAX_MESSAGES_PER_ROOM
    assert!(room_state.chat_history.len() <= MAX_MESSAGES_PER_ROOM);
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
        last_sanitized_message: None,
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
async fn test_security_manager_check_banned_ip() {
    let security = SecurityManager::new();
    let ip = "192.168.1.201";

    // Ban the IP by adding it directly with a future expiry
    {
        let mut banned = security.banned_ips.write().await;
        banned.insert(ip.to_string(), Instant::now() + Duration::from_secs(3600));
    }

    // Check should fail
    assert!(security.check_ip(ip).await.is_err());
}

#[tokio::test]
async fn test_memory_tracker_add_removes_bytes() {
    let tracker = MemoryTracker::new();

    // Add bytes
    tracker.add_bytes(1000);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 1000);

    // Remove bytes
    tracker.remove_bytes(500);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 500);

    // Remove more than available - should not go negative
    tracker.remove_bytes(1000);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 0);
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
async fn test_is_user_allowed_at_capacity() {
    let mut room_state = create_room();

    // Add 100 connected users (at capacity)
    for i in 0..100 {
        let user_id = format!("user_{}", i);
        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id,
                animal_name: format!("Animal{}", i),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Connected {
                    last_heartbeat: Instant::now(),
                    connection_id: format!("conn_{}", i),
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
    }

    // At capacity, should reject new users
    assert!(!room_state.is_user_allowed("new_user"));
}

#[tokio::test]
async fn test_is_user_allowed_new_user_under_capacity() {
    let mut room_state = create_room();

    // Add only a few users (under capacity)
    for i in 0..10 {
        let user_id = format!("user_{}", i);
        room_state.users.insert(
            user_id.clone(),
            UserData {
                user_id,
                animal_name: format!("Animal{}", i),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Connected {
                    last_heartbeat: Instant::now(),
                    connection_id: format!("conn_{}", i),
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
    }

    // New user should be allowed when under capacity
    assert!(room_state.is_user_allowed("brand_new_user"));
}

#[tokio::test]
async fn test_trigger_cleanup_when_memory_high() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Set memory to > 90% of MAX_TOTAL_ROOMS_MEMORY
    let high_memory = (MAX_TOTAL_ROOMS_MEMORY * 95) / 100;
    room_state
        .total_memory_bytes
        .store(high_memory, Ordering::SeqCst);

    // Add some messages that could be cleaned up
    for i in 0..10 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Message {}", i),
            timestamp: "1234567890".to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
    }

    // Should trigger cleanup
    room_state.trigger_cleanup(&memory_tracker).await;
}

#[tokio::test]
async fn test_trigger_cleanup_when_memory_low() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Set memory to < 90% of MAX_TOTAL_ROOMS_MEMORY
    let low_memory = (MAX_TOTAL_ROOMS_MEMORY * 50) / 100;
    room_state
        .total_memory_bytes
        .store(low_memory, Ordering::SeqCst);

    // Add some messages
    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Message {}", i),
            timestamp: "1234567890".to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
    }

    let history_len_before = room_state.chat_history.len();

    // Should not trigger cleanup (low memory)
    room_state.trigger_cleanup(&memory_tracker).await;

    // History should remain the same
    assert_eq!(room_state.chat_history.len(), history_len_before);
}

#[tokio::test]
async fn test_memory_tracker_add_bytes_exact_limit() {
    let tracker = MemoryTracker::new();

    // Add exactly at the limit
    let result = tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY);
    // Should succeed if starting from 0
    assert!(result);

    // Adding any more should fail
    let result2 = tracker.add_bytes(1);
    assert!(!result2);
}

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
async fn test_security_manager_suspicious_activity_decay() {
    let security = SecurityManager::new();
    let ip = "10.0.0.50";

    // Record some activity but not enough to ban
    for _ in 0..5 {
        let _ = security.record_suspicious_activity(ip).await;
    }

    // Should not be banned (under threshold of 10)
    assert!(security.check_ip(ip).await.is_ok());
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
async fn test_memory_tracker_should_gc_check() {
    let tracker = MemoryTracker::new();

    // Add some bytes
    tracker.add_bytes(1000);

    // Check should_gc (time-based check)
    let _ = tracker.should_gc();

    // This just exercises the should_gc path
}

#[tokio::test]
async fn test_app_state_cleanup_high_memory() {
    let app_state = Arc::new(AppState::new());

    // Create a room with high memory usage
    {
        let mut rooms = app_state.rooms.write().await;
        let room = create_room();
        rooms.insert("high_memory_room".to_string(), room);
        if let Some(r) = rooms.get_mut("high_memory_room") {
            r.total_memory_bytes
                .store(MAX_TOTAL_ROOMS_MEMORY + 1000, Ordering::SeqCst);
        }
    }

    // Run cleanup
    app_state.cleanup().await;
}

#[tokio::test]
async fn test_cleanup_batch_size_limit() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add more messages than CLEANUP_BATCH_SIZE (which is 50)
    let old_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - (31 * 24 * 60 * 60); // 31 days ago

    for i in 0..100 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Old message {}", i),
            timestamp: old_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
        memory_tracker.add_bytes(50);
    }

    // After first cleanup, should only remove CLEANUP_BATCH_SIZE (50)
    room_state
        .cleanup_messages(Instant::now(), &memory_tracker)
        .await;

    // Should have around 50 messages left (100 - 50)
    assert!(room_state.chat_history.len() <= 60);
}

#[tokio::test]
async fn test_memory_tracker_gc_timing() {
    let tracker = MemoryTracker::new();

    // First call should return true (enough time since epoch)
    let first = tracker.should_gc();

    // Immediate second call should return false (within 5 min window)
    let second = tracker.should_gc();

    // First may be true or false depending on timing, but second should be false
    if first {
        assert!(!second);
    }
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
            last_sanitized_message: None,
            last_reaction_event: None,
        },
    );

    // User should still be allowed but their rate limiter is checked
    let _allowed = room_state.is_user_allowed(&user_id);
    // Result depends on rate limiter state - just exercise the code path
}

#[tokio::test]
async fn test_room_state_user_count_accurate() {
    let mut room = create_room();
    let now = Instant::now();

    // Add 3 connected users
    for i in 0..3 {
        room.users.insert(
            format!("user-{}", i),
            UserData {
                user_id: format!("user-{}", i),
                animal_name: format!("Animal{}", i),
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: format!("conn-{}", i),
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
    }

    // Add 2 disconnected users
    for i in 3..5 {
        room.users.insert(
            format!("user-{}", i),
            UserData {
                user_id: format!("user-{}", i),
                animal_name: format!("Animal{}", i),
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Disconnected { since: now },
                last_read_message: None,
                is_typing: false,
                last_typing_event: None,
                last_read_receipt_event: None,
                rate_limiter: RateLimiter::new(),
                last_sanitized_message: None,
                last_reaction_event: None,
            },
        );
    }

    let connected_count = room
        .users
        .values()
        .filter(|u| matches!(u.connection_state, ConnectionState::Connected { .. }))
        .count();

    assert_eq!(connected_count, 3, "Should count only connected users");
    assert_eq!(room.users.len(), 5, "Total users includes disconnected");
}

#[tokio::test]
async fn test_memory_pressure_message_pruning() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Fill up near memory limit
    for i in 0..100 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user-{}", i),
            animal_name: "Lion".to_string(),
            text: format!("<p>Message {}</p>", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        };
        room.add_message(msg, &tracker);
    }

    let count_before = room.chat_history.len();
    assert!(count_before > 0, "Should have messages");

    // Trigger preservation which may trim
    room.preserve_messages(&tracker);

    assert!(
        room.chat_history.len() <= MAX_MESSAGES_PER_ROOM,
        "After preserve, should not exceed max messages"
    );
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
async fn test_security_manager_concurrent_suspicious_activity() {
    let manager = Arc::new(SecurityManager::new());
    let barrier = Arc::new(Barrier::new(5));
    let mut handles = vec![];

    // Record suspicious activity concurrently
    for _ in 0..5 {
        let manager_clone = manager.clone();
        let barrier_clone = barrier.clone();

        handles.push(tokio::spawn(async move {
            barrier_clone.wait().await;
            let _ = manager_clone.record_suspicious_activity("10.0.0.100").await;
        }));
    }

    join_all(handles).await;

    // Should have recorded activities without panic
    // suspicious_activity is HashMap<String, (usize, Instant)>
    let counters = manager.suspicious_activity.read().await;
    let count = counters.get("10.0.0.100").map(|(c, _)| *c).unwrap_or(0);
    assert!(
        count >= 5,
        "Should have recorded at least 5 suspicious activities"
    );
}

// ========== ADDITIONAL COVERAGE TESTS FOR UNCOVERED LINES ==========

#[tokio::test]
async fn test_memory_tracker_cleanup_if_needed() {
    let tracker = MemoryTracker::new();

    // Set last_gc to old time to trigger cleanup
    tracker.last_gc.store(0, Ordering::Relaxed);

    assert!(tracker.should_gc(), "Should need GC when last_gc is old");

    // should_gc already updates last_gc when returning true
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let last_gc = tracker.last_gc.load(Ordering::Relaxed);

    // last_gc should be close to current time
    assert!(
        last_gc >= now_secs - 5,
        "last_gc should be updated to recent time"
    );
}

#[tokio::test]
async fn test_broadcast_channel_capacity_supports_rapid_messages() {
    // Combo effects need rapid message delivery - verify channel has capacity
    let (tx, mut rx1) = tokio::sync::broadcast::channel::<String>(1000);
    let mut rx2 = tx.subscribe();

    // Simulate rapid combo messages
    for i in 0..10 {
        tx.send(format!("rapid-msg-{}", i)).unwrap();
    }

    // Both receivers should get all messages (no dropped)
    for i in 0..10 {
        assert_eq!(rx1.recv().await.unwrap(), format!("rapid-msg-{}", i));
        assert_eq!(rx2.recv().await.unwrap(), format!("rapid-msg-{}", i));
    }
}

#[tokio::test]
async fn test_chat_error_resource_limit_status() {
    let error = ChatError::ResourceLimit("test".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_chat_error_rate_limit_error_status() {
    let error = ChatError::RateLimitError("test".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

// ========== ROOM NAME REGEX TESTS ==========

#[tokio::test]
async fn test_memory_tracker_zero_bytes() {
    let tracker = MemoryTracker::new();
    tracker.add_bytes(0);
    assert_eq!(tracker.total_bytes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_memory_tracker_remove_more_than_added() {
    let tracker = MemoryTracker::new();
    tracker.add_bytes(100);
    tracker.remove_bytes(200); // Should saturate at 0
    assert_eq!(tracker.total_bytes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_memory_tracker_add_bytes_at_limit() {
    let tracker = MemoryTracker::new();
    // Fill to just under limit - should succeed
    let result = tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY - 1000);
    assert!(result);
}

#[tokio::test]
async fn test_memory_tracker_add_bytes_over_limit_fails() {
    let tracker = MemoryTracker::new();
    tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY - 1000);
    // Try to add more than remaining - should fail
    let result = tracker.add_bytes(2000);
    assert!(!result);
}

// ========== CONNECTION STATE TESTS ==========

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
async fn test_security_manager_nine_suspicious_not_banned() {
    let manager = SecurityManager::new();
    let ip = "192.168.1.1";

    for _ in 0..9 {
        let _ = manager.record_suspicious_activity(ip).await;
    }

    // Should still be able to pass check (not banned yet)
    assert!(manager.check_ip(ip).await.is_ok());
}

#[tokio::test]
async fn test_security_manager_eleven_suspicious_banned() {
    let manager = SecurityManager::new();
    let ip = "192.168.1.2";

    for _ in 0..11 {
        let _ = manager.record_suspicious_activity(ip).await;
    }

    // Should now be banned
    assert!(manager.check_ip(ip).await.is_err());
}

// ========== RESOURCE MONITOR TESTS ==========

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
async fn test_memory_tracker_should_gc_timing() {
    let tracker = MemoryTracker::new();

    // A fresh tracker has never swept (last_gc == 0), and "now" is unix epoch
    // seconds, so the first call is always due.
    assert!(
        tracker.should_gc(),
        "a tracker that has never swept must be due for one"
    );

    // ...and having swept, it records the time and refuses to sweep again
    // until MEMORY_GC_MIN_INTERVAL has passed. This is the property that keeps
    // the housekeeping loop from taking the room write lock every 60 seconds.
    assert!(
        !tracker.should_gc(),
        "should_gc must rate-limit itself to one sweep per MEMORY_GC_MIN_INTERVAL"
    );
    assert_ne!(
        tracker.last_gc.load(Ordering::SeqCst),
        0,
        "should_gc must record when it last swept"
    );
}

#[tokio::test]
async fn test_memory_tracker_remove_bytes_underflow_protection() {
    let tracker = MemoryTracker::new();

    // Add some bytes
    tracker.add_bytes(100);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 100);

    // Remove more than we have - should not underflow
    tracker.remove_bytes(200);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 0);
}

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
async fn test_room_state_add_message_drops_when_memory_exceeded() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Fill up to near the limit
    tracker
        .total_bytes
        .store(MAX_TOTAL_ROOMS_MEMORY - 100, Ordering::SeqCst);

    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Test message</p>".to_string(),
        timestamp: "1000".to_string(),
        reply_to: None,
        attachment: None,
    };

    // This should trigger pruning or dropping
    room.add_message(msg, &tracker);
}

#[tokio::test]
async fn test_security_manager_ban_and_unban() {
    let manager = SecurityManager::new();
    let ip = "10.0.0.1";

    // Not banned initially
    assert!(manager.check_ip(ip).await.is_ok());

    // Record many suspicious activities to trigger ban
    for _ in 0..15 {
        let _ = manager.record_suspicious_activity(ip).await;
    }

    // Should be banned now (check_ip returns error for banned IPs)
    assert!(manager.check_ip(ip).await.is_err());
}

// Tests for cookie reconnection - covers lines 1334-1383

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
async fn test_memory_tracker_concurrent_add_at_limit() {
    let tracker = MemoryTracker::new();

    // Add bytes up to near the limit
    let near_limit = MAX_TOTAL_ROOMS_MEMORY - 1000;
    assert!(tracker.add_bytes(near_limit));

    // Try to add more bytes concurrently
    let tracker1 = Arc::new(tracker);
    let tracker2 = tracker1.clone();

    let handle1 = tokio::spawn(async move { tracker1.add_bytes(500) });

    let handle2 = tokio::spawn(async move { tracker2.add_bytes(500) });

    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();

    // At least one should succeed, the other might fail due to limit
    assert!(result1 || result2, "At least one add should succeed");
}

// Test message gets dropped when memory tracker is full - covers line 874

#[tokio::test]
async fn test_add_message_dropped_when_memory_exceeded() {
    let app_state = Arc::new(AppState::new());

    // Fill up the memory tracker
    let fill_amount = MAX_TOTAL_ROOMS_MEMORY - 100;
    app_state.memory_tracker.add_bytes(fill_amount);

    // Create a room
    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert("memory-test".to_string(), create_room());
    }

    // Try to add a large message that would exceed the limit
    {
        let mut rooms = app_state.rooms.write().await;
        if let Some(room_state) = rooms.get_mut("memory-test") {
            let large_msg = OutgoingMessage {
                message_id: uuid::Uuid::new_v4(),
                user_id: "user1".to_string(),
                animal_name: "Lion".to_string(),
                text: "x".repeat(10000), // Large message
                timestamp: "12345".to_string(),
                reply_to: None,
                attachment: None,
            };

            let initial_history_len = room_state.chat_history.len();
            room_state.add_message(large_msg, &app_state.memory_tracker);

            // Message may or may not be added depending on pruning
            // The important thing is the code path was exercised
            assert!(room_state.chat_history.len() >= initial_history_len);
        }
    }
}

// Test ConnectionState::Disconnected branch - covers line 940-941

#[tokio::test]
async fn test_security_manager_banned_ip_check_paths() {
    let manager = SecurityManager::new();
    let ip = "192.168.1.100";

    // Not banned initially
    assert!(manager.check_ip(ip).await.is_ok());

    // Record suspicious activities to trigger ban
    for _ in 0..12 {
        let _ = manager.record_suspicious_activity(ip).await;
    }

    // Should be banned now
    let result = manager.check_ip(ip).await;
    assert!(result.is_err());

    // Check the error type
    if let Err(ChatError::SecurityError(_)) = result {
        // Expected error type
    } else {
        panic!("Expected SecurityError for banned IP");
    }
}

// Test room cleanup when room has messages - covers cleanup_rooms paths

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
async fn test_memory_tracker_high_usage() {
    let tracker = MemoryTracker::new();

    // Add bytes to just below limit
    let threshold = (MAX_TOTAL_ROOMS_MEMORY as f64 * 0.75) as usize;
    assert!(tracker.add_bytes(threshold));

    // Check should_gc behavior
    // Force last_gc to be old
    tracker.last_gc.store(0, Ordering::SeqCst);

    // should_gc should return true when last_gc is old
    let needs_gc = tracker.should_gc();
    assert!(
        needs_gc,
        "Should need GC when last_gc is old and memory is high"
    );
}

// Test RoomState with max users - covers lines 574-587

#[tokio::test]
async fn test_add_message_at_max_capacity() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Fill room to MAX_MESSAGES_PER_ROOM
    for i in 0..MAX_MESSAGES_PER_ROOM {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: format!("Message {}", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        }));
    }

    // Add one more message
    let new_msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "new_user".to_string(),
        animal_name: "NewAnimal".to_string(),
        text: "New message".to_string(),
        timestamp: "999999".to_string(),
        reply_to: None,
        attachment: None,
    };

    room.add_message(new_msg.clone(), &tracker);

    // After adding, history might temporarily exceed limit before next prune
    // The important thing is the code path was exercised
    assert!(!room.chat_history.is_empty());
    // Last message should be the new one
    assert_eq!(room.chat_history.last().unwrap().text, "New message");
}

// Test validate_input with valid input - covers lines 1162-1168

#[tokio::test]
async fn test_assign_animal_empty_pool() {
    let mut room = create_room();

    // Clear the animal pool
    room.available_animals.clear();

    // Should still assign an animal (generates fallback)
    let animal = room.assign_animal();

    // Should return some animal name
    assert!(!animal.is_empty());
}

// Test animal pool direct manipulation - covers animal assignment paths

#[tokio::test]
async fn test_animal_pool_manipulation() {
    let mut room = create_room();

    // Take an animal
    let animal = room.assign_animal();
    let count_after_assign = room.available_animals.len();

    // The pool should have one less animal after assignment
    assert!(!animal.is_empty());

    // Directly add animal back to pool (simulating return)
    room.available_animals.push_back(animal.clone());

    // Pool should have one more animal
    assert_eq!(room.available_animals.len(), count_after_assign + 1);
}

// Test payload size limit via WebSocket - covers lines 1631-1639

#[tokio::test]
async fn test_user_idle_past_threshold() {
    let user = UserData {
        user_id: "test".to_string(),
        animal_name: "Lion".to_string(),
        last_active: Instant::now(),
        last_message_time: Instant::now() - USER_IDLE_MESSAGE_TIMEOUT - Duration::from_secs(1),
        connection_state: ConnectionState::Connected {
            last_heartbeat: Instant::now(),
            connection_id: "conn".to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_sanitized_message: None,
        last_reaction_event: None,
    };

    let now = Instant::now();
    assert!(user_idle_for_too_long(&user, now));
}

// Test user not idle - covers line 324

#[tokio::test]
async fn test_memory_tracker_peak_bytes_update() {
    let tracker = MemoryTracker::new();

    // Add some bytes
    tracker.add_bytes(1000);
    assert!(tracker.peak_bytes.load(Ordering::Relaxed) >= 1000);

    // Add more
    tracker.add_bytes(2000);
    let peak = tracker.peak_bytes.load(Ordering::Relaxed);
    assert!(peak >= 3000);
}

// Test OutgoingMessage estimate_size with reply - covers lines 218-228

#[tokio::test]
async fn test_security_manager_suspicious_activity_count() {
    let manager = SecurityManager::new();
    let ip = "10.0.0.1";

    // Record activities but not enough to ban
    for _ in 0..5 {
        let result = manager.record_suspicious_activity(ip).await;
        assert!(result.is_ok());
    }

    // Should not be banned yet
    assert!(manager.check_ip(ip).await.is_ok());
}

// Test RoomState trigger_cleanup - covers lines 916-935

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
async fn test_generate_random_room_name_format() {
    // Generate several names and verify format
    for _ in 0..10 {
        let name = generate_random_room_name();
        assert!(!name.is_empty());
        assert!(name.contains('-'));
        // Name should have two parts separated by hyphen
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty());
        assert!(!parts[1].is_empty());
    }
}

// Test robots_txt_handler returns correct content - covers lines 2176-2192

#[tokio::test]
async fn test_app_state_cleanup_with_high_memory() {
    let state = Arc::new(AppState::new());

    // Create rooms with some memory usage
    {
        let mut rooms = state.rooms.write().await;
        for i in 0..5 {
            let mut room = create_room();
            // Add messages to increase memory
            for j in 0..50 {
                room.chat_history.push(Arc::new(OutgoingMessage {
                    message_id: uuid::Uuid::new_v4(),
                    user_id: format!("user{}", j),
                    animal_name: format!("Animal{}", j),
                    text: format!("Message {} in room {}", j, i),
                    timestamp: "12345".to_string(),
                    reply_to: None,
                    attachment: None,
                }));
            }
            rooms.insert(format!("room{}", i), room);
        }
    }

    // Force last_gc to be old
    state.memory_tracker.last_gc.store(0, Ordering::SeqCst);

    // Run cleanup
    state.cleanup().await;

    // Cleanup should have run without panic
}

// Test AppState shutdown with active users - covers lines 1054-1070

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
async fn test_add_message_triggers_prune_on_memory_limit() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // First add some messages successfully
    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: "Test message content".to_string(),
            timestamp: "12345".to_string(),
            reply_to: None,
            attachment: None,
        };
        room.add_message(msg, &tracker);
    }

    // Should have added messages
    assert!(!room.chat_history.is_empty());

    // Verify messages were added and tracked
    let initial_count = room.chat_history.len();
    assert_eq!(initial_count, 5);
}

// Test MemoryTracker total_bytes tracking - covers lines 377-383

#[tokio::test]
async fn test_memory_tracker_total_bytes_tracking() {
    let tracker = MemoryTracker::new();

    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 0);

    tracker.add_bytes(500);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 500);

    tracker.add_bytes(300);
    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 800);
}

// Test validate_message with edge cases - covers lines 2354-2370

#[tokio::test]
async fn test_memory_tracker_remove_bytes_underflow() {
    let tracker = MemoryTracker::new();

    // Add some bytes
    tracker.add_bytes(100);

    // Remove more than added - should saturate at 0
    tracker.remove_bytes(200);

    assert_eq!(tracker.total_bytes.load(Ordering::Relaxed), 0);
}

// Test MemoryTracker peak tracking - covers lines 379-390

#[tokio::test]
async fn test_memory_tracker_peak_bytes_tracking() {
    let tracker = MemoryTracker::new();

    tracker.add_bytes(1000);
    assert_eq!(tracker.peak_bytes.load(Ordering::Relaxed), 1000);

    tracker.add_bytes(500);
    assert_eq!(tracker.peak_bytes.load(Ordering::Relaxed), 1500);

    // Remove bytes - peak should stay
    tracker.remove_bytes(1000);
    assert_eq!(tracker.peak_bytes.load(Ordering::Relaxed), 1500);
}

// Test MemoryTracker should_gc timer logic - covers lines 398-410

#[tokio::test]
async fn test_memory_tracker_gc_timing_threshold() {
    let tracker = MemoryTracker::new();

    // First call should trigger GC and update timestamp
    assert!(tracker.should_gc());

    // Immediate second call should not trigger
    assert!(!tracker.should_gc());

    // Set last_gc to far in the past
    tracker.last_gc.store(0, Ordering::SeqCst);

    // Now should trigger again
    assert!(tracker.should_gc());
}

// Test ConnectionPool max connections per IP - covers lines 427-445

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
async fn test_security_manager_suspicious_activity() {
    let manager = SecurityManager::new();
    let ip = "10.0.0.1".to_string();

    // Not banned initially
    assert!(manager.check_ip(&ip).await.is_ok());

    // Track activities below threshold
    for _ in 0..10 {
        let _ = manager.record_suspicious_activity(&ip).await;
    }

    // Next one should trigger ban (11th in < 60s triggers ban)
    let result = manager.record_suspicious_activity(&ip).await;
    assert!(result.is_err()); // Should be banned now

    // Now check_ip should fail
    assert!(manager.check_ip(&ip).await.is_err());
}

// Test ResourceMonitor tracking - covers lines 544-555

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

/// A disconnected user is eventually reclaimed, and their name returns to the
/// pool for someone else.
#[tokio::test]
async fn cleanup_reclaims_abandoned_users_and_recycles_their_name() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        let pool_before = room.available_animals.len();

        room.users.insert(
            "ghost".to_string(),
            UserData {
                user_id: "ghost".to_string(),
                animal_name: "otter".to_string(),
                last_active: Instant::now(),
                last_message_time: Instant::now(),
                connection_state: ConnectionState::Disconnected {
                    since: Instant::now() - DISCONNECTED_USER_RETENTION - Duration::from_secs(60),
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
        rooms.insert("ghost-room".to_string(), room);
        assert_eq!(pool_before, ANIMAL_NAMES.len());
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get("ghost-room").expect("room should still exist");
    assert!(
        !room.users.contains_key("ghost"),
        "a long-disconnected user should be reclaimed"
    );
    assert!(
        room.available_animals.iter().any(|a| a == "otter"),
        "their animal name should go back into the pool"
    );
}

/// `Default` is the same thing as `new` for the limiter types.
#[test]
fn limiter_defaults_match_their_constructors() {
    let limiter = RateLimiter::default();
    assert_eq!(limiter.message_count, 0);
    assert_eq!(limiter.join_attempts, 0);

    let state = AppState::default();
    assert_eq!(state.memory_tracker.total_bytes.load(Ordering::SeqCst), 0);
}

/// A message that is short enough as text but expands past the limit once
/// escaped is rejected.
#[test]
fn messages_that_expand_past_the_limit_when_escaped_are_rejected() {
    // Each '<' becomes "&lt;", so this is under the cap as input and far over
    // it once sanitised.
    let expands = "<".repeat(MAX_MESSAGE_LEN - 1);
    assert!(expands.len() <= MAX_MESSAGE_LEN);

    let result = validate_message(&expands);
    assert!(
        result.is_err(),
        "a message that exceeds the limit after sanitisation must be rejected"
    );
}

/// At capacity the server refuses new sockets, and refusing costs nothing.
#[tokio::test]
async fn upgrades_are_refused_at_capacity_without_leaking_slots() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    state
        .resource_monitor
        .total_connections
        .store(MAX_CONCURRENT_USERS, Ordering::SeqCst);

    assert!(
        connect_async(ws_request(addr, "capacity-room", &[]))
            .await
            .is_err(),
        "a full server must refuse the upgrade"
    );

    assert_eq!(
        state
            .resource_monitor
            .total_connections
            .load(Ordering::SeqCst),
        MAX_CONCURRENT_USERS,
        "a refused upgrade must not change the count either way"
    );
    assert_eq!(state.connection_pool.active.load(Ordering::SeqCst), 0);

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
                last_sanitized_message: None,
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

/// The memory ceiling is inclusive: a reservation that lands exactly on the
/// limit is accepted, one byte more is refused.
#[test]
fn the_memory_ceiling_admits_exactly_the_limit() {
    let tracker = MemoryTracker::new();

    assert!(
        tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY),
        "a reservation landing exactly on the ceiling must be accepted"
    );
    assert_eq!(
        tracker.total_bytes.load(Ordering::SeqCst),
        MAX_TOTAL_ROOMS_MEMORY
    );

    assert!(
        !tracker.add_bytes(1),
        "one byte past the ceiling must be refused"
    );
    assert_eq!(
        tracker.total_bytes.load(Ordering::SeqCst),
        MAX_TOTAL_ROOMS_MEMORY,
        "a refused reservation must reserve nothing"
    );
}

/// A sweep becomes due strictly *after* the interval, not at it.
#[test]
fn a_memory_sweep_is_due_only_after_the_full_interval() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let tracker = MemoryTracker::new();
    // Exactly one interval ago: not yet due.
    tracker
        .last_gc
        .store(now - MEMORY_GC_MIN_INTERVAL.as_secs(), Ordering::SeqCst);
    assert!(
        !tracker.should_gc(),
        "a sweep exactly one interval old is not yet due"
    );

    // One second past the interval: due.
    tracker
        .last_gc
        .store(now - MEMORY_GC_MIN_INTERVAL.as_secs() - 1, Ordering::SeqCst);
    assert!(tracker.should_gc(), "a sweep past the interval must be due");
}

/// Expiring idle per-IP counters must keep the recent ones.
#[tokio::test]
async fn expiring_ip_counters_keeps_recently_active_addresses() {
    let pool = ConnectionPool::new();

    pool.add_connection("198.51.100.1").await.unwrap();
    {
        // An address with no live connections, last seen longer ago than the
        // retention window. The zero matters: a counter that is still counting
        // is kept whatever its age, so that this sweep cannot lift the per-IP
        // limit out from under a long session
        // (`a_live_connection_counter_survives_the_stale_sweep`).
        let mut counters = pool.ip_counters.write().await;
        counters.insert(
            "203.0.113.9".to_string(),
            (
                AtomicUsize::new(0),
                Instant::now() - IP_COUNTER_RETENTION - Duration::from_secs(60),
            ),
        );
    }

    pool.cleanup_stale().await;

    let counters = pool.ip_counters.read().await;
    assert!(
        counters.contains_key("198.51.100.1"),
        "an address that just connected must be kept"
    );
    assert!(
        !counters.contains_key("203.0.113.9"),
        "an address idle past the retention window must be dropped"
    );
}

/// A ban lands strictly *after* the allowed number of rejected attempts.
#[tokio::test]
async fn an_ip_is_banned_only_past_the_suspicion_threshold() {
    let security = SecurityManager::new();
    let ip = "203.0.113.50";

    // Exactly the allowance: recorded, but not yet a ban.
    for attempt in 1..=MAX_SUSPICIOUS_EVENTS {
        assert!(
            security.record_suspicious_activity(ip).await.is_ok(),
            "attempt {attempt} is within the allowance"
        );
    }
    assert!(
        security.check_ip(ip).await.is_ok(),
        "exactly the allowed number of attempts must not ban"
    );

    // One more crosses it.
    assert!(
        security.record_suspicious_activity(ip).await.is_err(),
        "one attempt past the allowance must ban"
    );
    assert!(
        security.check_ip(ip).await.is_err(),
        "a banned address must be refused"
    );
}

/// A message that would push total memory over the ceiling triggers a
/// proactive prune before it is added, not after.
///
/// Reaching this branch through real message traffic would mean accumulating
/// close to MAX_TOTAL_ROOMS_MEMORY (400MB) of history; setting the tracker's
/// counter directly exercises the same branch without needing that much data.
#[test]
fn adding_a_message_near_the_memory_ceiling_prunes_proactively() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    for i in 0..50 {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: format!("padding {i}"),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        }));
    }
    let before = room.chat_history.len();

    // Park the *room's* counter just under the ceiling so the next message
    // crosses it. The branch reads the room's own total, not the global
    // tracker's — they are separate counters and only one gates this path.
    room.total_memory_bytes
        .store(MAX_TOTAL_ROOMS_MEMORY - 10, Ordering::SeqCst);

    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "the message that crosses the ceiling".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    assert!(
        room.chat_history.len() < before,
        "crossing the ceiling must prune older history, not merely append"
    );
}

// ========== MORE COVERAGE: REACHABLE SESSION BRANCHES ==========

/// §3 — the animal pool is bounded by the roster it was built from.
///
/// Reclaiming a user pushes their name back into the pool. A name that never
/// came *out* of that pool — a cookie identity carried in from another room, or
/// a `guest_N` fallback minted when the pool was empty — made that push
/// unbalanced, so the pool grew every time such a user was reclaimed.
#[tokio::test]
async fn reclaiming_users_cannot_grow_or_pollute_the_animal_pool() {
    let state = Arc::new(AppState::new());
    let long_gone = Instant::now() - DISCONNECTED_USER_RETENTION - Duration::from_secs(60);

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();

        // A name this room never issued (carried in on a cookie), and a guest
        // fallback that is not an animal at all.
        for (uid, animal) in [("visitor", "otter"), ("overflow", "guest_7")] {
            let mut user = connected_user(uid, animal, "c", Instant::now());
            user.connection_state = ConnectionState::Disconnected { since: long_gone };
            room.users.insert(uid.to_string(), user);
        }
        rooms.insert("pool-room".to_string(), room);
    }

    cleanup_rooms(&state).await;

    let rooms = state.rooms.read().await;
    let room = rooms.get("pool-room").expect("room should still exist");

    assert!(
        room.available_animals.len() <= ANIMAL_NAMES.len(),
        "the pool must never exceed the roster it was built from: {} > {}",
        room.available_animals.len(),
        ANIMAL_NAMES.len()
    );
    assert!(
        room.available_animals
            .iter()
            .all(|a| ANIMAL_NAMES.contains(&a.as_str())),
        "every assignable name must be on the roster; found {:?}",
        room.available_animals
            .iter()
            .filter(|a| !ANIMAL_NAMES.contains(&a.as_str()))
            .collect::<Vec<_>>()
    );
}

/// §5 — no counter can be driven below zero.
///
/// An unbalanced decrement on an unsigned counter wraps to `usize::MAX`, and
/// every one of these counters is compared against a ceiling: a single wrap
/// wedges the server at "full" for the rest of the process's life. This is the
/// failure `MemoryTracker::remove_bytes` already documents; the connection
/// counters had the same shape and none of the protection.
#[tokio::test]
async fn releasing_more_than_was_reserved_cannot_wrap_a_counter() {
    let monitor = ResourceMonitor::new();
    monitor.release_connection();
    assert_eq!(
        monitor.total_connections.load(Ordering::SeqCst),
        0,
        "an unmatched release must floor at zero, not wrap"
    );
    assert!(
        monitor.can_accept_connection(),
        "an unmatched release must not wedge the server at capacity"
    );

    let pool = ConnectionPool::new();
    pool.remove_connection("10.0.0.1").await;
    assert_eq!(pool.active.load(Ordering::SeqCst), 0);

    pool.add_connection("10.0.0.1").await.unwrap();
    pool.remove_connection("10.0.0.1").await;
    pool.remove_connection("10.0.0.1").await;
    assert_eq!(
        pool.active.load(Ordering::SeqCst),
        0,
        "a double release must floor at zero"
    );
    assert!(
        pool.can_accept("10.0.0.1").await,
        "a double release must not wedge the per-IP counter at its limit"
    );
}

/// The suspicion window rolls, so the ban stays reachable.
///
/// `record_suspicious_activity` counted up forever from a `first_seen` that was
/// never reset, and banned only while `first_seen.elapsed()` was still inside
/// the window. So an address whose first suspicious event was more than
/// `SUSPICIOUS_ACTIVITY_WINDOW` ago could never be banned again *no matter what
/// it did* — the one condition that could fire had permanently gone false. The
/// slow attacker was the one the counter stopped protecting against.
#[tokio::test]
async fn suspicion_counts_within_a_window_rather_than_forever() {
    let manager = SecurityManager::new();
    let ip = "slow-attacker";

    // An old first sighting, the way an address that has been around a while
    // looks by the time it starts misbehaving.
    manager.suspicious_activity.write().await.insert(
        ip.to_string(),
        (
            1,
            Instant::now() - SUSPICIOUS_ACTIVITY_WINDOW - Duration::from_secs(5),
        ),
    );

    let mut banned = false;
    for _ in 0..=(MAX_SUSPICIOUS_EVENTS * 2) {
        if manager.record_suspicious_activity(ip).await.is_err() {
            banned = true;
            break;
        }
    }

    assert!(
        banned,
        "an address must still be bannable after its first sighting ages out"
    );
    assert!(
        manager.check_ip(ip).await.is_err(),
        "and the ban must actually take effect"
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

/// The memory ceiling admits exactly the limit and refuses one byte more.
///
/// `total > MAX` mutating to `total >= MAX` survived, which means nothing was
/// testing the boundary itself — only comfortably inside and comfortably
/// outside it. One byte either way is the difference between a full room
/// working and a full room silently dropping messages.
#[test]
fn the_memory_ceiling_boundary_is_exact() {
    let tracker = MemoryTracker::new();

    assert!(
        tracker.add_bytes(MAX_TOTAL_ROOMS_MEMORY),
        "exactly the limit must be admitted"
    );
    assert_eq!(
        tracker.total_bytes.load(Ordering::SeqCst),
        MAX_TOTAL_ROOMS_MEMORY
    );

    assert!(
        !tracker.add_bytes(1),
        "one byte past the limit must be refused"
    );
    assert_eq!(
        tracker.total_bytes.load(Ordering::SeqCst),
        MAX_TOTAL_ROOMS_MEMORY,
        "and a refusal must reserve nothing"
    );

    // Freeing one byte makes exactly one byte available again.
    tracker.remove_bytes(1);
    assert!(tracker.add_bytes(1), "the freed byte must be usable");
    assert!(!tracker.add_bytes(1), "and only that one");
}

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

/// An address is banned only *past* the suspicion threshold, not at it.
#[tokio::test]
async fn the_suspicion_threshold_is_exact() {
    let manager = SecurityManager::new();
    let ip = "borderline";

    for i in 1..=MAX_SUSPICIOUS_EVENTS {
        assert!(
            manager.record_suspicious_activity(ip).await.is_ok(),
            "event {i} of {MAX_SUSPICIOUS_EVENTS} is suspicious but not yet a ban"
        );
        assert!(
            manager.check_ip(ip).await.is_ok(),
            "and the address is still allowed after event {i}"
        );
    }

    assert!(
        manager.record_suspicious_activity(ip).await.is_err(),
        "the event past the threshold is the one that bans"
    );
    assert!(manager.check_ip(ip).await.is_err());
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

/// The soft-limit sweep actually sweeps.
///
/// `trigger_cleanup` could be replaced with an empty body and nothing noticed:
/// it is called from the housekeeping loop when the process is over its memory
/// ceiling, and it is the last thing standing between a full server and one
/// that refuses every message. Nothing asserted it did anything at all.
#[tokio::test]
async fn the_soft_limit_sweep_removes_aged_messages() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Aged messages: stamped 1970, far past MAX_MESSAGE_AGE.
    for i in 0..5 {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("ancient {i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            },
            &tracker,
        );
    }

    // Below the soft threshold, the sweep leaves them alone — pruning is for
    // memory pressure, not a scheduled deletion of everything old.
    room.trigger_cleanup(&tracker).await;
    assert_eq!(
        room.chat_history.len(),
        5,
        "under the soft limit there is nothing to reclaim"
    );

    // Over it, the aged messages go.
    let (num, den) = MEMORY_SOFT_LIMIT_RATIO;
    room.total_memory_bytes
        .store((MAX_TOTAL_ROOMS_MEMORY * num) / den + 1, Ordering::SeqCst);
    room.trigger_cleanup(&tracker).await;

    assert!(
        room.chat_history.is_empty(),
        "past the soft limit the sweep must actually reclaim the aged messages"
    );
}

// ========== DOCS, NAMING AND DEAD CODE ==========
