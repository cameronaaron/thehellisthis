//! `SecurityManager`: IP bans, suspicious-activity accumulation, and the
//! stale-entry sweeps that keep both bounded (§3.5).

use super::*;
use crate::session::check_admission;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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

/// Running out of connection slots is not evidence of anything, and must not
/// ban the address it happens to.
///
/// The shape of the lockout this pins: the per-address limit is three, a
/// returning tab is refused for as long as the connection it replaces is
/// still held, and `client.js` retries at 1s/2s/4s/8s. Three tabs through one
/// network blip is over eleven refusals inside the sixty-second suspicion
/// window — and eleven was the ban threshold, for an hour. This was the only
/// caller of `record_suspicious_activity`, so the whole ban list was reachable
/// by exactly one population: real users with tabs open.
///
/// Asserted against `check_admission` rather than the pool, because the
/// refusal and the (absent) escalation are the same call and the bug was in
/// how they were wired together.
#[tokio::test]
async fn exhausting_the_per_address_connection_limit_never_bans_the_address() {
    let state = Arc::new(AppState::new());
    let ip = hash_client_address("198.51.100.7");

    // Hold the address at its ceiling, the way three live tabs do.
    for _ in 0..MAX_CONCURRENT_CONNECTIONS_PER_IP {
        state
            .connection_pool
            .add_connection(&ip)
            .await
            .expect("the first three connections are within the limit");
    }

    // Now retry far past the suspicion threshold, as a blip through three
    // tabs does inside one window.
    for attempt in 0..(MAX_SUSPICIOUS_EVENTS * 2) {
        let refused = check_admission(&state, Some(&ip), "main").await;
        assert!(
            matches!(refused, Err(ChatError::RateLimitError(_))),
            "attempt {attempt} should be refused for capacity, not anything else: \
             {refused:?}"
        );
    }

    assert!(
        state.security_manager.banned_ips.read().await.is_empty(),
        "being refused for having too many connections open must never earn a \
         ban: the limit has already done its job by refusing, and escalating \
         it locks a real user out of the whole site for {}s",
        IP_BAN_DURATION.as_secs()
    );
    assert!(
        state
            .security_manager
            .suspicious_activity
            .read()
            .await
            .is_empty(),
        "nor may it accumulate towards one"
    );

    // And once a slot frees, the same address is admitted again immediately —
    // no residue from all those refusals.
    state.connection_pool.remove_connection(&ip).await;
    assert!(
        check_admission(&state, Some(&ip), "main").await.is_ok(),
        "a freed slot must be usable by the address that was just refused"
    );
}

/// A foreign `Origin` is what the ban list is for.
///
/// A browser sends an `Origin` that does not match the host it is talking to
/// only when a third-party page is driving it — the shipped client cannot
/// produce one — so unlike running out of slots, repeating it is evidence.
/// This is the other half of the change above: the ban machinery is still
/// reachable, just by something a real visitor never does.
#[tokio::test]
async fn repeated_foreign_origin_upgrades_do_earn_a_ban() {
    let (addr, state, handle) = start_ws_server_with_state().await;

    for _ in 0..=MAX_SUSPICIOUS_EVENTS {
        let mut request = format!("ws://{addr}/ws/main")
            .into_client_request()
            .expect("request builds");
        request
            .headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        // Every one of these is refused; what matters is what accumulates.
        let _ = connect_async(request).await;
    }

    let banned = state.security_manager.banned_ips.read().await;
    assert!(
        !banned.is_empty(),
        "a host that keeps opening sockets from a foreign origin is the case \
         the ban list exists for, and must still reach it"
    );

    handle.abort();
}

// ========== WEBSOCKET INTEGRATION TESTS ==========

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

/// A retention boundary is a moment, and the moment is now testable.
///
/// Every `elapsed() < DURATION` in the two housekeeping sweeps survived
/// mutation as both `<=` and `==`: the clock is read *inside* the comparison,
/// so no test can put an entry exactly on the boundary — the reading has always
/// moved past it before the comparison runs. That is not evidence the boundary
/// does not matter, only that nothing could reach it. Passing the instant in is
/// the same fix `cleanup_rooms_at` got, and it needs no injectable clock (§9.4).
///
/// An entry exactly at its retention age is expired, not kept: the retention is
/// how long something is remembered *for*, so the moment it is reached, it is
/// over.
#[tokio::test]
async fn an_entry_exactly_at_its_retention_age_is_swept() {
    let now = Instant::now();

    // A zero counter is kept only while it is inside the retention window.
    for (age, survives) in [
        (IP_COUNTER_RETENTION - Duration::from_nanos(1), true),
        (IP_COUNTER_RETENTION, false),
    ] {
        let pool = ConnectionPool::new();
        pool.add_connection("addr").await.expect("first connection");
        pool.remove_connection("addr").await;
        {
            let mut counters = pool.ip_counters.write().await;
            let (_, last_seen) = counters.get_mut("addr").expect("the entry");
            *last_seen = now - age;
        }

        pool.cleanup_stale_at(now).await;
        assert_eq!(
            pool.ip_counters.read().await.contains_key("addr"),
            survives,
            "an idle counter {age:?} old against a retention of \
             {IP_COUNTER_RETENTION:?} should survive: {survives}"
        );
    }

    // A ban lapses exactly when its duration is up, and the sweep drops it.
    for (age, still_banned) in [
        (IP_BAN_DURATION - Duration::from_nanos(1), true),
        (IP_BAN_DURATION, false),
    ] {
        let security = SecurityManager::new();
        security
            .banned_ips
            .write()
            .await
            .insert("addr".to_string(), now - age);

        assert_eq!(
            security.check_ip_at("addr", now).await.is_err(),
            still_banned,
            "a ban {age:?} old against a duration of {IP_BAN_DURATION:?} \
             should still refuse: {still_banned}"
        );

        security.cleanup_stale_at(now).await;
        assert_eq!(
            security.banned_ips.read().await.contains_key("addr"),
            still_banned,
            "the sweep must drop a ban the check no longer honours, or the map \
             keeps entries nothing will ever read (§3.5)"
        );
    }

    // The suspicious-activity window rolls on the same rule.
    for (age, survives) in [
        (SUSPICIOUS_ACTIVITY_WINDOW - Duration::from_nanos(1), true),
        (SUSPICIOUS_ACTIVITY_WINDOW, false),
    ] {
        let security = SecurityManager::new();
        security
            .suspicious_activity
            .write()
            .await
            .insert("addr".to_string(), (1, now - age));

        security.cleanup_stale_at(now).await;
        assert_eq!(
            security
                .suspicious_activity
                .read()
                .await
                .contains_key("addr"),
            survives,
            "a window {age:?} old against {SUSPICIOUS_ACTIVITY_WINDOW:?} \
             should survive: {survives}"
        );
    }
}
