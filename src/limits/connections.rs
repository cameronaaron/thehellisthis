//! Per-IP and global connection accounting — the reservation every upgrade
//! takes and must release on every exit path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::{trace, warn};

use crate::config::{
    IP_COUNTER_RETENTION, MAX_CONCURRENT_CONNECTIONS_PER_IP, MAX_CONCURRENT_USERS,
};
use crate::error::{ChatError, ChatResult};

use super::saturating_dec;

/// Every successful [`ConnectionPool::add_connection`] must be paired with a
/// [`ConnectionPool::remove_connection`] on every exit path, including errors
/// before the upgrade.
#[derive(Debug, Default)]
pub struct ConnectionPool {
    pub active: AtomicUsize,
    pub ip_counters: RwLock<HashMap<String, (AtomicUsize, Instant)>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops counters for addresses not seen recently, so the map is bounded by
    /// recent clients. A non-zero counter is never dropped, whatever its age:
    /// `last_seen` is stamped when a connection is added, so a session outliving
    /// [`IP_COUNTER_RETENTION`] used to lose its counter mid-session and silently
    /// lift that address's limit.
    pub async fn cleanup_stale(&self) {
        self.cleanup_stale_at(Instant::now()).await;
    }

    /// The sweep with its instant supplied: with `elapsed()` inside the
    /// comparison no test could land exactly on the retention boundary, so `<=` and
    /// `==` survived mutation. The `cleanup_rooms_at` fix, with no injectable
    /// clock (§9.4).
    pub async fn cleanup_stale_at(&self, now: Instant) {
        let mut counters = self.ip_counters.write().await;
        let before = counters.len();
        counters.retain(|_, (counter, last_seen)| {
            counter.load(Ordering::Relaxed) > 0
                || now.duration_since(*last_seen) < IP_COUNTER_RETENTION
        });
        let dropped = before - counters.len();
        if dropped > 0 {
            // Bound before the macro (§6.1d): `trace!` evaluates arguments only
            // when enabled, so inline this read as uncovered in a branch that ran.
            let remaining = counters.len();
            trace!(dropped, remaining, "dropped stale IP counters");
        }
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
            // A race, not a normal rejection: `can_accept` said yes moments ago
            // (§5.2's ordering), so another connection from this address was
            // reserved in between.
            warn!("per-address limit hit at reservation time, after the pre-check passed");
            return Err(ChatError::ResourceLimit(
                "Too many connections from IP".into(),
            ));
        }

        if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_USERS {
            counter.fetch_sub(1, Ordering::SeqCst);
            self.active.fetch_sub(1, Ordering::SeqCst);
            warn!("global connection limit hit at reservation time, after the pre-check passed");
            return Err(ChatError::ResourceLimit("Server at capacity".into()));
        }

        let active = self.active.load(Ordering::Relaxed);
        trace!(active, "connection reserved");
        Ok(())
    }

    pub async fn remove_connection(&self, ip: &str) {
        let mut counters = self.ip_counters.write().await;
        if let Some((counter, _)) = counters.get_mut(ip) {
            saturating_dec(counter);
        }
        saturating_dec(&self.active);
        let active = self.active.load(Ordering::Relaxed);
        trace!(active, "connection released");
    }
}
