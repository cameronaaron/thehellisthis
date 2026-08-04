//! Periodic housekeeping: reclaiming abandoned users, fading `main`, and
//! deleting rooms nobody is in.
//!
//! Two of the three behaviours here are the product, not maintenance. Rooms
//! that die and history that fades are what make the place feel live
//! (ENGINEERING-STANDARDS.md §7); the memory they free is a side effect.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, info};

use crate::config::{
    DISCONNECTED_USER_RETENTION, EMPTY_ROOM_CLEANUP_DELAY, MAIN_ROOM, MAIN_ROOM_FADE_IDLE,
    MAIN_ROOM_FADE_KEEP, MAX_MESSAGES_PER_ROOM, NOVA_ROOM,
};
use crate::room::{ConnectionState, RoomState, UserData};
use crate::state::AppState;

/// One housekeeping pass over every room.
///
/// Takes the room write lock exactly once and does all of its work inside it.
/// Splitting into a read pass and a write pass would mean re-deriving the
/// decisions after the map may have changed underneath, for a lock held tens of
/// microseconds either way.
pub async fn cleanup_rooms(state: &Arc<AppState>) {
    cleanup_rooms_at(state, Instant::now()).await;
}

/// One housekeeping pass, as of `now`.
///
/// The instant is a parameter so the boundaries can be tested *on* the
/// boundary. Every comparison here is against a duration, and a test using its
/// own `Instant::now()` can get close to a threshold but never land on it —
/// which left "retained for exactly the retention period" and "idle by exactly
/// the fade time" as behaviour nothing pinned, and mutation testing found all
/// three (`>` reading as `>=` changes who gets reclaimed).
///
/// This is one parameter on one function, at the edge where the untestable
/// thing is — not the injectable clock §9.4 rejects, which would thread a time
/// source through every timing decision in the codebase.
pub async fn cleanup_rooms_at(state: &Arc<AppState>, now: Instant) {
    let mut rooms_to_remove: Vec<String> = Vec::new();
    let mut rooms = state.rooms.write().await;

    for (room_name, room) in rooms.iter_mut() {
        reclaim_stale_users(room_name, room, now);

        // History is bounded here for *every* room, not only on the join path.
        // Trimming used to happen when somebody joined, plus the `main` fade
        // below; a room that was busy but had no new joiners therefore grew
        // without limit, and since the byte ceiling is process-wide, one such
        // room could fill it and make every room on the server start dropping
        // messages (ENGINEERING-STANDARDS.md §3).
        let is_main = room_name == MAIN_ROOM;
        // `nova` is permanent like `main` (a demo link should stay alive) but
        // does not fade like `main` — there is no reason a research demo's
        // history should shrink on the flagship room's schedule, so only
        // `is_main` feeds the fade target below.
        let is_permanent = is_main || room_name == NOVA_ROOM;
        let idle_for = now.duration_since(room.last_activity);
        let target = history_trim_target(is_main, idle_for);

        // No `len > target` guard: `retain_newest` already makes that comparison
        // and returns 0 when there is nothing to do, so a second one here was
        // code with no behaviour — a mutant could flip it to `>=` and nothing
        // could tell, because both spellings called a function that no-ops.
        //
        // The values are bound outside the macro deliberately: `tracing`
        // evaluates a log's fields only when the level is enabled, so as
        // arguments they do not run under a test with no subscriber, and read
        // as uncovered lines in a branch the test definitely takes.
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
        // The names were just collected from this same map under this same
        // lock, so `remove` cannot miss; `if let` here would be an unreachable
        // branch that no test could ever cover.
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

/// Removes users who disconnected long enough ago that they are not coming
/// back — nothing is handed back to the name pool by hand: the pool is a
/// rotation over the roster, and whether a name is free is derived from
/// `users`, which this removal has just updated (see `RoomState::assign_animal`).
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

/// How many messages a room should be trimmed to keep, right now.
///
/// `main` is permanent, so instead of being deleted it fades: once idle for
/// `MAIN_ROOM_FADE_IDLE` it trims harder than every other room. A pure
/// function of two primitives rather than a room and an instant, so a test
/// can ask it directly — "is main's fade threshold really `MAIN_ROOM_FADE_IDLE`,
/// not one interval off" — without constructing a room old enough to answer.
pub(crate) fn history_trim_target(is_main: bool, idle_for: Duration) -> usize {
    if is_main && idle_for >= MAIN_ROOM_FADE_IDLE {
        MAIN_ROOM_FADE_KEEP
    } else {
        MAX_MESSAGES_PER_ROOM
    }
}

/// Whether a room has earned deletion: not permanent (`main`, which fades
/// instead of dying, or `nova`, the novachannel demo); nobody connected; idle
/// for at least `EMPTY_ROOM_CLEANUP_DELAY`.
pub(crate) fn is_abandoned(
    is_permanent: bool,
    has_connected_users: bool,
    idle_for: Duration,
) -> bool {
    !is_permanent && !has_connected_users && idle_for >= EMPTY_ROOM_CLEANUP_DELAY
}
