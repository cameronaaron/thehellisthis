//! Who is here: the roster, the user-count and system-event broadcasts, and
//! the admission and cleanup checks that depend on presence.

use std::sync::atomic::Ordering;
use std::time::Instant;

use tracing::{debug, trace};

use crate::config::{HEARTBEAT_TIMEOUT, MAX_TOTAL_ROOMS_MEMORY, MEMORY_SOFT_LIMIT_RATIO};
use crate::limits::MemoryTracker;
use crate::protocol::{OutgoingEvent, SystemEvent, encode_broadcast};

use super::{ConnectionState, RoomState};

impl RoomState {
    /// The names of everyone currently in the room, sorted.
    ///
    /// Sorted so the list does not reshuffle every time somebody asks — a
    /// `HashMap` iterates in whatever order it likes, and a panel that reorders
    /// itself on refresh reads as people coming and going.
    ///
    /// O(users), on a path a person triggers by opening a panel — not the
    /// message path (§1.1).
    pub fn roster(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .users
            .values()
            .filter(|u| u.is_connected())
            .map(|u| u.animal_name.clone())
            .collect();
        names.sort_unstable();
        names
    }

    /// Broadcasts the number of users whose heartbeat is still current.
    pub fn broadcast_user_count(&self) {
        let now = Instant::now();
        let count = self
            .users
            .values()
            .filter(|u| match &u.connection_state {
                ConnectionState::Connected { last_heartbeat, .. } => {
                    now.duration_since(*last_heartbeat) <= HEARTBEAT_TIMEOUT
                }
                ConnectionState::Disconnected { .. } => false,
            })
            .count();

        trace!(count, "broadcasting user count");
        // A send error means nobody is subscribed, which is not an error.
        let _ = self
            .sender
            .send(encode_broadcast(&OutgoingEvent::UserCount { count }));
    }

    pub fn broadcast_system_event(&self, event: SystemEvent) {
        trace!(?event, "broadcasting system event");
        let _ = self
            .sender
            .send(encode_broadcast(&OutgoingEvent::System { event }));
    }

    /// Admission check for a user about to join.
    ///
    /// Returns `false` when the room is at capacity, or when a known user is
    /// reconnecting faster than [`crate::config::MAX_ROOM_JOIN_ATTEMPTS`]
    /// allows. `limit` is the caller's job to pick — every room uses
    /// [`crate::config::MAX_USERS_PER_ROOM`] except `nova`, whose pairwise
    /// message-sealing cost caps it far lower
    /// ([`crate::config::room_user_limit`], [`crate::config::NOVA_MAX_USERS`]).
    pub fn is_user_allowed(&mut self, user_id: &str, limit: usize) -> bool {
        if self.connected_user_count() >= limit {
            debug!(limit, "room is at capacity");
            return false;
        }

        match self.users.get_mut(user_id) {
            Some(user) => {
                let allowed = user.rate_limiter.can_join_room();
                if !allowed {
                    debug!(%user_id, "rejoin attempts exhausted this window");
                }
                allowed
            }
            None => true,
        }
    }

    /// Prunes aged messages once the room passes the soft memory threshold.
    pub async fn trigger_cleanup(&mut self, memory_tracker: &MemoryTracker) {
        let (num, den) = MEMORY_SOFT_LIMIT_RATIO;
        let soft_limit = (MAX_TOTAL_ROOMS_MEMORY * num) / den;
        let current = self.total_memory_bytes.load(Ordering::Relaxed);
        if current > soft_limit {
            debug!(
                current,
                soft_limit, "over soft memory threshold; pruning aged messages"
            );
            self.cleanup_messages(Instant::now(), memory_tracker).await;
        }
    }
}
