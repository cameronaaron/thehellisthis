//! Resource ceilings: the guards that decide whether the server accepts more
//! work. All are admission control, answering "may this happen at all?" in
//! O(1) before any allocation would have to be undone.
//!
//! One file per concern: [`rate_limiting`] (per-user messages and joins),
//! [`memory`] (the process-wide byte ceiling), [`connections`] (per-address
//! and global connections), [`security`] (address reputation and bans).
//! [`ResourceMonitor`] is a few lines and stays here.

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
/// Every counter here is compared against a ceiling, so a decrement that
/// outruns its increment would wrap to `usize::MAX` and read as full for the
/// rest of the process's life (§5.10). A compare-exchange loop, not
/// `fetch_sub`, so the check and the subtraction are one atomic step.
fn saturating_dec(counter: &AtomicUsize) {
    let mut current = counter.load(Ordering::SeqCst);
    while let Err(actual) = counter.compare_exchange_weak(
        current,
        current.saturating_sub(1),
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        current = actual;
    }
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
