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
            // Nothing to hand back to the name pool: the pool is a rotation
            // over the roster, and whether a name is free is derived from
            // `users` — which this removal has just updated. Putting names back
            // by hand is what used to make the pool grow and fill with
            // non-roster names (see `RoomState::assign_animal`).
            if room.users.remove(&uid).is_some() {
                debug!(user_id = %uid, room = %room_name, "reclaimed abandoned user");
            }
        }

        // History is bounded here for *every* room, not only on the join path.
        // Trimming used to happen when somebody joined, plus the `main` fade
        // below; a room that was busy but had no new joiners therefore grew
        // without limit, and since the byte ceiling is process-wide, one such
        // room could fill it and make every room on the server start dropping
        // messages (ENGINEERING-STANDARDS.md §3).
        //
        // `main` is permanent, so instead of being deleted it fades: once idle
        // it trims harder than everyone else.
        let is_main = room_name == MAIN_ROOM;
        let idle_for = now.duration_since(room.last_activity);
        let target = if is_main && idle_for >= MAIN_ROOM_FADE_IDLE {
            MAIN_ROOM_FADE_KEEP
        } else {
            MAX_MESSAGES_PER_ROOM
        };

        if room.chat_history.len() > target {
            info!(
                room = %room_name,
                from = room.chat_history.len(),
                to = target,
                idle_s = idle_for.as_secs(),
                "trimming room history"
            );
            room.retain_newest(target, &state.memory_tracker);
        }

        if is_main {
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
        // The names were just collected from this same map under this same
        // lock, so `remove` cannot miss; `if let` here would be an unreachable
        // branch that no test could ever cover.
        let room = rooms
            .remove(name)
            .expect("room was present when it was marked for removal");

        info!(room = %name, "deleted inactive room");

        let freed: usize = room.chat_history.iter().map(|m| m.estimate_size()).sum();
        if freed > 0 {
            state.memory_tracker.remove_bytes(freed);
        }
    }
}
