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
    /// Everyone currently in the room, by name, sorted so the panel doesn't
    /// reshuffle (and look like people coming and going) each time. O(users), on a
    /// path someone triggers by opening a panel, not the message path (§1.1).
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

    /// Admission check for a joining user: `false` at capacity, or when a known
    /// user reconnects faster than [`crate::config::MAX_ROOM_JOIN_ATTEMPTS`]
    /// allows. The caller picks `limit` ([`crate::config::room_user_limit`]): the
    /// `nova` room is capped by its group's fixed capacity
    /// ([`crate::config::NOVA_MAX_USERS`]).
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
