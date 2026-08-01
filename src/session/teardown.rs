//! Releasing a session's resources and announcing the departure.

use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, info};

use crate::protocol::{OutgoingEvent, SystemEvent};
use crate::room::ConnectionState;
use crate::state::AppState;

/// Releases a session's resources and announces the departure.
///
/// Guarded on `connection_id`: a user who reconnected before this teardown ran
/// already has a *newer* live connection, and marking them disconnected here
/// would evict the session that is currently working.
pub async fn cleanup_user(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    connection_id: &str,
    ip: Option<&str>,
) {
    if let Some(ip) = ip {
        state.connection_pool.remove_connection(ip).await;
    }

    state.resource_monitor.release_connection();

    let mut rooms = state.rooms.write().await;
    let Some(room_state) = rooms.get_mut(room) else {
        return;
    };
    let Some(user) = room_state.users.get_mut(user_id) else {
        return;
    };

    let ConnectionState::Connected {
        connection_id: current_id,
        ..
    } = &user.connection_state
    else {
        return;
    };

    if current_id != connection_id {
        debug!(
            stale = %connection_id,
            current = %current_id,
            "ignoring teardown for a superseded connection"
        );
        return;
    }

    let (uid, animal) = (user.user_id.clone(), user.animal_name.clone());
    let _ = room_state.sender.send(OutgoingEvent::System {
        event: SystemEvent::UserLeft {
            user_id: uid.clone(),
            animal_name: animal.clone(),
        },
    });

    user.connection_state = ConnectionState::Disconnected {
        since: Instant::now(),
    };
    room_state.broadcast_user_count();

    let remaining = room_state.connected_user_count();
    info!(
        room = %room,
        user_id = %uid,
        animal_name = %animal,
        remaining,
        "departed"
    );
}
