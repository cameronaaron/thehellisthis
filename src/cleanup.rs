//! Periodic housekeeping: reclaiming abandoned users, fading `main`, and
//! deleting rooms nobody is in.
//!
//! Two of the three behaviours here are the product, not maintenance. Rooms
//! that die and history that fades are what make the place feel live
//! (ENGINEERING-STANDARDS.md §7); the memory they free is a side effect.

use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, info};

use crate::config::{
    DISCONNECTED_USER_RETENTION, EMPTY_ROOM_CLEANUP_DELAY, MAIN_ROOM, MAIN_ROOM_FADE_IDLE,
    MAIN_ROOM_FADE_KEEP, MAX_MESSAGES_PER_ROOM,
};
use crate::protocol::OutgoingMessage;
use crate::state::AppState;

/// One housekeeping pass over every room.
///
/// Takes the room write lock exactly once and does all of its work inside it.
/// Splitting into a read pass and a write pass would mean re-deriving the
/// decisions after the map may have changed underneath, for a lock held tens of
/// microseconds either way.
pub async fn cleanup_rooms(state: &Arc<AppState>) {
    let now = Instant::now();
    let mut rooms_to_remove: Vec<String> = Vec::new();
    let mut rooms = state.rooms.write().await;

    for (room_name, room) in rooms.iter_mut() {
        // Reclaim users who disconnected long enough ago that they are not
        // coming back, returning their animal name to the pool.
        let stale: Vec<String> = room
            .users
            .iter()
            .filter(|(_, user)| match &user.connection_state {
                crate::room::ConnectionState::Disconnected { since } => {
                    now.duration_since(*since) > DISCONNECTED_USER_RETENTION
                }
                crate::room::ConnectionState::Connected { .. } => false,
            })
            .map(|(uid, _)| uid.clone())
            .collect();

        for uid in stale {
            if let Some(user) = room.users.remove(&uid) {
                room.available_animals.push_back(user.animal_name);
                debug!(user_id = %uid, room = %room_name, "reclaimed abandoned user");
            }
        }

        // `main` is permanent, so it fades instead of being deleted.
        if room_name == MAIN_ROOM {
            let idle_for = now.duration_since(room.last_activity);
            let target = if idle_for >= MAIN_ROOM_FADE_IDLE {
                MAIN_ROOM_FADE_KEEP
            } else {
                MAX_MESSAGES_PER_ROOM
            };

            if room.chat_history.len() > target {
                info!(
                    from = room.chat_history.len(),
                    to = target,
                    idle_s = idle_for.as_secs(),
                    "fading main room history"
                );
                room.retain_newest(target, &state.memory_tracker);
            }
            continue;
        }

        let has_connected_users = room.users.values().any(crate::room::UserData::is_connected);
        if !has_connected_users
            && now.duration_since(room.last_activity) >= EMPTY_ROOM_CLEANUP_DELAY
        {
            rooms_to_remove.push(room_name.clone());
        }
    }

    for name in &rooms_to_remove {
        let Some(room) = rooms.remove(name) else {
            continue;
        };

        info!(room = %name, "deleted inactive room");

        let freed: usize = room
            .chat_history
            .iter()
            .map(OutgoingMessage::estimate_size)
            .sum();
        if freed > 0 {
            state.memory_tracker.remove_bytes(freed);
        }
    }
}
