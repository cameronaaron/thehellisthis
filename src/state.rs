//! Process-wide state shared by every connection.
//!
//! One `RwLock<HashMap<..>>` over all rooms, held for as short a span as the
//! operation allows. Every method here documents what it does under that lock,
//! because lock duration — not CPU — is this server's scaling limit.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::config::{MAX_ROOMS, MAX_TOTAL_ROOMS_MEMORY};
use crate::limits::{ConnectionPool, MemoryTracker, ResourceMonitor, SecurityManager};
use crate::protocol::{OutgoingEvent, SystemEvent};
use crate::room::RoomState;

pub struct AppState {
    pub rooms: RwLock<HashMap<String, RoomState>>,
    pub resource_monitor: ResourceMonitor,
    pub memory_tracker: MemoryTracker,
    pub connection_pool: ConnectionPool,
    pub security_manager: SecurityManager,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            // Pre-sized to the room cap: the map never has to rehash, and the
            // allocation is bounded by config rather than by traffic.
            rooms: RwLock::new(HashMap::with_capacity(MAX_ROOMS)),
            resource_monitor: ResourceMonitor::new(),
            memory_tracker: MemoryTracker::new(),
            connection_pool: ConnectionPool::new(),
            security_manager: SecurityManager::new(),
        }
    }

    /// Tells every connected user they are being disconnected, then drops all
    /// rooms.
    ///
    /// Best-effort by design: the container gets a short grace period on
    /// SIGINT, and a clean "user left" is worth more to the people watching
    /// than a guaranteed one is to the process that is exiting anyway.
    pub async fn shutdown(&self) {
        info!("beginning graceful shutdown");
        let mut rooms = self.rooms.write().await;

        for room_state in rooms.values() {
            announce_departures(room_state);
        }

        rooms.clear();
        info!("shutdown complete");
    }

    /// Periodic housekeeping: expire stale IP counters, and sweep room history
    /// if the process is over its memory ceiling.
    ///
    /// The write lock is taken only once the cheap checks say there is work.
    pub async fn cleanup(&self) {
        // Every map keyed by a client address gets swept here (§3.5). Adding
        // one without adding it to this line is how the last two grew unbounded.
        self.connection_pool.cleanup_stale().await;
        self.security_manager.cleanup_stale().await;

        if !self.memory_tracker.should_gc() {
            return;
        }

        let mut rooms = self.rooms.write().await;
        let total_memory: usize = rooms
            .values()
            .map(|r| r.total_memory_bytes.load(Ordering::Relaxed))
            .sum();

        debug!(total_memory, rooms = rooms.len(), "periodic memory sweep");

        if total_memory > MAX_TOTAL_ROOMS_MEMORY {
            for room in rooms.values_mut() {
                room.trigger_cleanup(&self.memory_tracker).await;
            }
        }
    }
}

/// Announces every *connected* user in `room_state` as having left.
///
/// A function rather than the inner half of `shutdown`'s nested loop, so "who
/// gets told" — connected users only, not everyone the room has ever known —
/// is one thing to read and one thing a test can call directly against a
/// room built by hand, instead of a loop body reachable only by shutting an
/// `AppState` down.
pub(crate) fn announce_departures(room_state: &RoomState) {
    let mut announced = 0usize;
    for user in room_state.users.values() {
        if user.is_connected() {
            let _ = room_state.sender.send(OutgoingEvent::System {
                event: SystemEvent::UserLeft {
                    user_id: user.user_id.clone(),
                    animal_name: user.animal_name.clone(),
                },
            });
            announced += 1;
        }
    }
    if announced > 0 {
        debug!(announced, "announced departures for shutdown");
    }
}
