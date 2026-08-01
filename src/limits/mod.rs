//! Resource ceilings: the four guards that decide whether the server accepts
//! more work.
//!
//! All of them are admission control, not throttling — each one answers "may
//! this happen at all?" in O(1), on the connection's own path, before any
//! allocation the answer would have to be undone.
//!
//! One struct per concern, one file per struct: [`rate_limiting`] is per-user
//! messages and joins, [`memory`] is the process-wide byte ceiling,
//! [`connections`] is per-IP and global connection accounting, [`security`]
//! is IP reputation and bans. [`ResourceMonitor`] stays here — global
//! admission is a handful of lines, not a module of its own.

mod connections;
mod memory;
mod rate_limiting;
mod security;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::{MAX_CONCURRENT_USERS, MAX_TOTAL_ROOMS_MEMORY};

pub use connections::ConnectionPool;
pub use memory::MemoryTracker;
pub use rate_limiting::RateLimiter;
pub use security::SecurityManager;

/// Decrements an unsigned counter, flooring at zero.
///
/// Every counter in this module is compared against a ceiling, so a decrement
/// that outruns its increment does not merely misreport — it wraps to
/// `usize::MAX` and the comparison says "full" for the rest of the process's
/// life, with no traffic and no way back. `MemoryTracker::remove_bytes` already
/// guarded against exactly this; the connection counters had the same shape and
/// none of the protection.
///
/// A CAS loop rather than `fetch_sub`, because the check and the subtraction
/// have to be one atomic step: reading zero and then subtracting anyway is the
/// same bug with extra instructions.
fn saturating_dec(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        Some(current.saturating_sub(1))
    });
}

#[derive(Debug, Default)]
pub struct ResourceMonitor {
    pub total_memory: AtomicUsize,
    pub total_connections: AtomicUsize,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Releases one connection reservation, flooring at zero.
    pub fn release_connection(&self) {
        saturating_dec(&self.total_connections);
    }

    pub fn can_accept_connection(&self) -> bool {
        self.total_connections.load(Ordering::Relaxed) < MAX_CONCURRENT_USERS
            && self.total_memory.load(Ordering::Relaxed) < MAX_TOTAL_ROOMS_MEMORY
    }
}
