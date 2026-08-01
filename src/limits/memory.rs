//! The process-wide byte ceiling for retained chat history, tracked so
//! admission can refuse before a room grows past it.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::SystemTime;

use tracing::{trace, warn};

use crate::config::{MAX_TOTAL_ROOMS_MEMORY, MEMORY_GC_MIN_INTERVAL};

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
            warn!(
                requested = bytes,
                limit = MAX_TOTAL_ROOMS_MEMORY,
                "refused: process-wide memory ceiling reached"
            );
            return false;
        }

        // `>` rather than `>=` is a skipped no-op, not a correctness choice:
        // at equality the compare-exchange would store the value already there
        // and exit on the next read. Mutation testing reports the two spellings
        // as indistinguishable because they are (§6.6d) — recorded here so the
        // next sweep does not spend the analysis again.
        let mut peak = self.peak_bytes.load(Ordering::Relaxed);
        while total > peak {
            match self.peak_bytes.compare_exchange_weak(
                peak,
                total,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    trace!(
                        peak = total,
                        limit = MAX_TOTAL_ROOMS_MEMORY,
                        "new memory peak"
                    );
                    break;
                }
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
