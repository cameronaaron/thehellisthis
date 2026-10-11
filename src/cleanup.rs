//! Periodic housekeeping: reclaiming abandoned users, fading `main` and
//! deleting rooms nobody is in. Rooms that die and history that fades are the
//! product (ENGINEERING-STANDARDS.md §7); the freed memory is a side effect.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, info};

use crate::config::{
    DISCONNECTED_USER_RETENTION, EMPTY_ROOM_CLEANUP_DELAY, MAIN_ROOM, MAIN_ROOM_FADE_IDLE,
    MAIN_ROOM_FADE_KEEP, MAX_MESSAGES_PER_ROOM, NOVA_ROOM,
};
use crate::room::{ConnectionState, RoomState, UserData};
use crate::state::AppState;

/// One housekeeping pass over every room, under one write lock: splitting it
/// into read and write passes would re-derive decisions after the map may
/// have changed, for a lock held tens of microseconds anyway.
pub async fn cleanup_rooms(state: &Arc<AppState>) {
    cleanup_rooms_at(state, Instant::now()).await;
}

/// One housekeeping pass as of `now`.
///
/// The instant is a parameter so tests can land on the boundaries: with
/// `Instant::now()` they could only approach them, and mutation testing found
/// all three ("retained exactly the retention period", "idle exactly the fade
/// time") unpinned. One parameter at the edge, not the injectable clock §9.4
/// rejects.
pub async fn cleanup_rooms_at(state: &Arc<AppState>, now: Instant) {
    let mut rooms_to_remove: Vec<String> = Vec::new();
    let mut rooms = state.rooms.write().await;

    for (room_name, room) in rooms.iter_mut() {
        reclaim_stale_users(room_name, room, now);

        // Every room's history is bounded here, not only on join: a busy
        // room without new joiners once grew without limit and, since the
        // ceiling is process-wide, could make every room drop messages
        // (ENGINEERING-STANDARDS.md §3).
        let is_main = room_name == MAIN_ROOM;
        // `nova` is permanent like `main` but doesn't fade on its schedule,
        // so only `is_main` feeds the fade target.
        let is_permanent = is_main || room_name == NOVA_ROOM;
        let idle_for = now.duration_since(room.last_activity);
        let target = history_trim_target(is_main, idle_for);

        // No `len > target` guard: `retain_newest` makes that comparison and
        // returns 0, so a guard here survived mutation either way.
        //
        // Values bound outside the macro because `tracing` evaluates fields
        // only when the level is enabled, which reads as uncovered lines in a
        // branch the test takes.
        let from = room.chat_history.len();
        let dropped = room.retain_newest(target, &state.memory_tracker);
        if dropped > 0 {
            let idle_s = idle_for.as_secs();
            info!(room = %room_name, from, to = target, idle_s, "trimming room history");
        }

        let has_connected_users = room.users.values().any(UserData::is_connected);
        if is_abandoned(is_permanent, has_connected_users, idle_for) {
            rooms_to_remove.push(room_name.clone());
        }
    }

    for name in &rooms_to_remove {
        // The names were just collected from this map under this lock, so
        // `remove` can't miss; an `if let` would be an untestable branch.
        let room = rooms
            .remove(name)
            .expect("room was present when it was marked for removal");

        info!(room = %name, "deleted inactive room");

        // No `freed > 0` guard: `remove_bytes(0)` is already a no-op, so the
        // guard was a comparison with no behaviour behind it.
        let freed: usize = room.chat_history.iter().map(|m| m.estimate_size()).sum();
        state.memory_tracker.remove_bytes(freed);
    }
}

/// Removes users disconnected long enough that they aren't coming back. No
/// names are returned to the pool by hand: it is a rotation over the roster,
/// and whether a name is free is derived from `users`
/// (`RoomState::assign_animal`).
fn reclaim_stale_users(room_name: &str, room: &mut RoomState, now: Instant) {
    let stale: Vec<String> = room
        .users
        .iter()
        .filter(|(_, user)| match &user.connection_state {
            ConnectionState::Disconnected { since } => {
                now.duration_since(*since) > DISCONNECTED_USER_RETENTION
            }
            ConnectionState::Connected { .. } => false,
        })
        .map(|(uid, _)| uid.clone())
        .collect();

    for uid in stale {
        if room.users.remove(&uid).is_some() {
            debug!(user_id = %uid, room = %room_name, "reclaimed abandoned user");
        }
    }
}

/// How many messages a room should keep right now. `main` fades rather than
/// being deleted: after `MAIN_ROOM_FADE_IDLE` idle it trims harder. A pure
/// function of two values, so a test can check the threshold exactly without
/// building an old room.
pub(crate) fn history_trim_target(is_main: bool, idle_for: Duration) -> usize {
    if is_main && idle_for >= MAIN_ROOM_FADE_IDLE {
        MAIN_ROOM_FADE_KEEP
    } else {
        MAX_MESSAGES_PER_ROOM
    }
}

/// Whether a room should be deleted: not permanent (`main` fades instead;
/// `nova` is the demo), nobody connected, and idle at least
/// `EMPTY_ROOM_CLEANUP_DELAY`.
pub(crate) fn is_abandoned(
    is_permanent: bool,
    has_connected_users: bool,
    idle_for: Duration,
) -> bool {
    !is_permanent && !has_connected_users && idle_for >= EMPTY_ROOM_CLEANUP_DELAY
}
