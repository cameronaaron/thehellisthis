//! IP reputation: repeated rejected connections earn a temporary ban.

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::{trace, warn};

use crate::config::{IP_BAN_DURATION, MAX_SUSPICIOUS_EVENTS, SUSPICIOUS_ACTIVITY_WINDOW};
use crate::error::{ChatError, ChatResult};

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
        self.check_ip_at(ip, Instant::now()).await
    }

    /// The check with its instant supplied, so a ban expiring is a moment a
    /// test can stand on rather than one it has to wait for.
    pub async fn check_ip_at(&self, ip: &str, now: Instant) -> ChatResult<()> {
        let banned = self.banned_ips.read().await;
        if let Some(banned_at) = banned.get(ip)
            && now.duration_since(*banned_at) < IP_BAN_DURATION
        {
            let remaining = IP_BAN_DURATION.saturating_sub(now.duration_since(*banned_at));
            warn!(address = %ip, remaining_s = remaining.as_secs(), "rejected: address is banned");
            return Err(ChatError::SecurityError("IP is banned".into()));
        }
        Ok(())
    }

    /// Records one rejected attempt, banning the IP once it exceeds
    /// [`MAX_SUSPICIOUS_EVENTS`] within [`SUSPICIOUS_ACTIVITY_WINDOW`].
    ///
    /// The window **rolls**, the same way [`crate::limits::RateLimiter::roll_window`]
    /// does. It used to count up forever from a `first_seen` that was never
    /// reset, while the ban fired only if that original sighting was still
    /// inside the window — so once an address had been known for longer than
    /// the window, the ban condition was permanently false and no amount of
    /// abuse could trip it. The attacker the counter stopped protecting
    /// against was the patient one, which is the wrong way round.
    pub async fn record_suspicious_activity(&self, ip: &str) -> ChatResult<()> {
        let mut suspicious = self.suspicious_activity.write().await;
        let (count, window_start) = suspicious
            .entry(ip.to_string())
            .or_insert_with(|| (0, Instant::now()));

        if window_start.elapsed() >= SUSPICIOUS_ACTIVITY_WINDOW {
            *count = 0;
            *window_start = Instant::now();
        }

        *count += 1;

        if *count > MAX_SUSPICIOUS_EVENTS {
            let mut banned = self.banned_ips.write().await;
            banned.insert(ip.to_string(), Instant::now());
            let ban_s = IP_BAN_DURATION.as_secs();
            warn!(address = %ip, count = *count, ban_s, "address banned");
            return Err(ChatError::SecurityError(
                "Too many suspicious activities".into(),
            ));
        }
        Ok(())
    }

    /// Drops expired bans and lapsed suspicion records.
    ///
    /// §3.5: both maps are keyed by client address — attacker-influenced, and
    /// otherwise unbounded. An expired ban used to be *tested* for expiry on
    /// every read but never removed, and a suspicion record was never removed
    /// at all, so both grew by one entry per address ever seen and never gave
    /// anything back. Checking expiry on read bounds what the entry *means*,
    /// not how much of it there is.
    pub async fn cleanup_stale(&self) {
        self.cleanup_stale_at(Instant::now()).await;
    }

    /// The sweep with its instant supplied, so a ban sitting exactly on its
    /// expiry is a case a test can construct rather than a race with the clock.
    pub async fn cleanup_stale_at(&self, now: Instant) {
        let mut banned = self.banned_ips.write().await;
        let banned_before = banned.len();
        banned.retain(|_, banned_at| now.duration_since(*banned_at) < IP_BAN_DURATION);
        let bans_expired = banned_before - banned.len();
        drop(banned);

        let mut suspicious = self.suspicious_activity.write().await;
        let suspicious_before = suspicious.len();
        suspicious.retain(|_, (_, window_start)| {
            now.duration_since(*window_start) < SUSPICIOUS_ACTIVITY_WINDOW
        });
        let suspicion_lapsed = suspicious_before - suspicious.len();

        if bans_expired > 0 || suspicion_lapsed > 0 {
            trace!(
                bans_expired,
                suspicion_lapsed, "security records cleaned up"
            );
        }
    }
}
