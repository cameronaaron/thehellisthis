//! Resource ceilings: the four guards that decide whether the server accepts
//! more work.
//!
//! All of them are admission control, not throttling — each one answers "may
//! this happen at all?" in O(1), on the connection's own path, before any
//! allocation the answer would have to be undone.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Instant, SystemTime};

use tokio::sync::RwLock;

use crate::config::{
    IP_BAN_DURATION, IP_COUNTER_RETENTION, MAX_CONCURRENT_CONNECTIONS_PER_IP, MAX_CONCURRENT_USERS,
    MAX_MESSAGES_PER_WINDOW, MAX_ROOM_JOIN_ATTEMPTS, MAX_SUSPICIOUS_EVENTS, MAX_TOTAL_ROOMS_MEMORY,
    MEMORY_GC_MIN_INTERVAL, RATE_LIMIT_WINDOW, SUSPICIOUS_ACTIVITY_WINDOW,
};
use crate::error::{ChatError, ChatResult};

// ---------------------------------------------------------------------------
// Per-user rate limiting
// ---------------------------------------------------------------------------

/// A fixed window per user. Fixed rather than sliding on purpose: a sliding
/// window needs a timestamp per event, which is per-message allocation for a
/// limit whose exact edge behaviour nobody can perceive.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    pub window_start: Instant,
    pub message_count: usize,
    pub join_attempts: usize,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            window_start: Instant::now(),
            message_count: 0,
            join_attempts: 0,
        }
    }

    pub fn can_send_message(&mut self) -> bool {
        self.roll_window();

        if self.message_count >= MAX_MESSAGES_PER_WINDOW {
            return false;
        }

        self.message_count += 1;
        true
    }

    pub fn can_join_room(&mut self) -> bool {
        self.roll_window();

        self.join_attempts += 1;
        self.join_attempts <= MAX_ROOM_JOIN_ATTEMPTS
    }

    /// Resets both counters when the window has elapsed.
    fn roll_window(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.window_start) > RATE_LIMIT_WINDOW {
            self.window_start = now;
            self.message_count = 0;
            self.join_attempts = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Global admission
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ResourceMonitor {
    pub total_memory: AtomicUsize,
    pub total_connections: AtomicUsize,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn can_accept_connection(&self) -> bool {
        self.total_connections.load(Ordering::Relaxed) < MAX_CONCURRENT_USERS
            && self.total_memory.load(Ordering::Relaxed) < MAX_TOTAL_ROOMS_MEMORY
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Process-wide byte ceiling for retained chat history.
///
/// The container has a hard memory limit; exceeding it is an OOM kill, which
/// disconnects every user in every room. Dropping one message is strictly
/// better than that, so [`MemoryTracker::add_bytes`] refuses rather than grows.
#[derive(Debug, Default)]
pub struct MemoryTracker {
    pub total_bytes: AtomicUsize,
    pub peak_bytes: AtomicUsize,
    pub last_gc: AtomicU64,
}

impl MemoryTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves `bytes`, or returns `false` having reserved nothing.
    pub fn add_bytes(&self, bytes: usize) -> bool {
        let total = self.total_bytes.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if total > MAX_TOTAL_ROOMS_MEMORY {
            self.total_bytes.fetch_sub(bytes, Ordering::SeqCst);
            return false;
        }

        let mut peak = self.peak_bytes.load(Ordering::Relaxed);
        while total > peak {
            match self.peak_bytes.compare_exchange_weak(
                peak,
                total,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => peak = observed,
            }
        }
        true
    }

    /// Releases `bytes`. Saturating: an accounting slip must never underflow
    /// the counter into `usize::MAX` and wedge the server at "full" forever.
    pub fn remove_bytes(&self, bytes: usize) {
        let current = self.total_bytes.load(Ordering::SeqCst);
        self.total_bytes
            .store(current.saturating_sub(bytes), Ordering::SeqCst);
    }

    /// True at most once per [`MEMORY_GC_MIN_INTERVAL`]; records the sweep.
    pub fn should_gc(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Saturating: a backwards clock step (NTP, container migration) would
        // otherwise panic here on a debug build and wedge GC on a release one.
        let elapsed = now.saturating_sub(self.last_gc.load(Ordering::Relaxed));
        if elapsed > MEMORY_GC_MIN_INTERVAL.as_secs() {
            self.last_gc.store(now, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// Per-IP and global connection accounting.
///
/// Every successful [`ConnectionPool::add_connection`] must be paired with a
/// [`ConnectionPool::remove_connection`] on *every* exit path, including error
/// returns taken before the socket is upgraded.
#[derive(Debug, Default)]
pub struct ConnectionPool {
    pub active: AtomicUsize,
    pub ip_counters: RwLock<HashMap<String, (AtomicUsize, Instant)>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops counters for IPs that have not connected recently, so the map is
    /// bounded by *recent* clients rather than by every client ever seen.
    pub async fn cleanup_stale(&self) {
        let mut counters = self.ip_counters.write().await;
        counters.retain(|_, (_, last_seen)| last_seen.elapsed() < IP_COUNTER_RETENTION);
    }

    pub async fn can_accept(&self, ip: &str) -> bool {
        let counters = self.ip_counters.read().await;
        counters.get(ip).is_none_or(|(counter, _)| {
            counter.load(Ordering::Relaxed) < MAX_CONCURRENT_CONNECTIONS_PER_IP
        })
    }

    pub async fn add_connection(&self, ip: &str) -> ChatResult<()> {
        let mut counters = self.ip_counters.write().await;
        let (counter, last_seen) = counters
            .entry(ip.to_string())
            .or_insert_with(|| (AtomicUsize::new(0), Instant::now()));

        *last_seen = Instant::now();

        if counter.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_CONNECTIONS_PER_IP {
            counter.fetch_sub(1, Ordering::SeqCst);
            return Err(ChatError::ResourceLimit(
                "Too many connections from IP".into(),
            ));
        }

        if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_USERS {
            counter.fetch_sub(1, Ordering::SeqCst);
            self.active.fetch_sub(1, Ordering::SeqCst);
            return Err(ChatError::ResourceLimit("Server at capacity".into()));
        }

        Ok(())
    }

    pub async fn remove_connection(&self, ip: &str) {
        let mut counters = self.ip_counters.write().await;
        if let Some((counter, _)) = counters.get_mut(ip) {
            counter.fetch_sub(1, Ordering::SeqCst);
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Security
// ---------------------------------------------------------------------------

/// IP reputation: repeated rejected connections earn a temporary ban.
#[derive(Debug, Default)]
pub struct SecurityManager {
    pub banned_ips: RwLock<HashMap<String, Instant>>,
    pub suspicious_activity: RwLock<HashMap<String, (usize, Instant)>>,
}

impl SecurityManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn check_ip(&self, ip: &str) -> ChatResult<()> {
        let banned = self.banned_ips.read().await;
        if let Some(banned_at) = banned.get(ip)
            && banned_at.elapsed() < IP_BAN_DURATION
        {
            return Err(ChatError::SecurityError("IP is banned".into()));
        }
        Ok(())
    }

    /// Records one rejected attempt, banning the IP once it exceeds
    /// [`MAX_SUSPICIOUS_EVENTS`] within [`SUSPICIOUS_ACTIVITY_WINDOW`].
    pub async fn record_suspicious_activity(&self, ip: &str) -> ChatResult<()> {
        let mut suspicious = self.suspicious_activity.write().await;
        let (count, first_seen) = suspicious
            .entry(ip.to_string())
            .or_insert_with(|| (0, Instant::now()));

        *count += 1;

        if *count > MAX_SUSPICIOUS_EVENTS && first_seen.elapsed() < SUSPICIOUS_ACTIVITY_WINDOW {
            let mut banned = self.banned_ips.write().await;
            banned.insert(ip.to_string(), Instant::now());
            return Err(ChatError::SecurityError(
                "Too many suspicious activities".into(),
            ));
        }
        Ok(())
    }
}
