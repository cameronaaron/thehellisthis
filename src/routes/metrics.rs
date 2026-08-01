//! Prometheus text exposition — seven gauges, gated on `METRICS_TOKEN`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use http::{HeaderMap, StatusCode};

use crate::state::AppState;

/// Hand-rolled rather than pulling in a metrics runtime: seven gauges read
/// straight from the atomics that already exist do not justify a registry, a
/// background exporter task, and two dependencies.
pub async fn metrics_handler(headers: HeaderMap, State(state): State<Arc<AppState>>) -> Response {
    // No distinction between "no token configured" and "wrong token": both
    // read as though this route does not exist. See
    // `security::is_authorized_for_metrics`.
    if !crate::security::is_authorized_for_metrics(&headers) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (room_count, total_users, total_messages) = {
        let rooms = state.rooms.read().await;
        (
            rooms.len(),
            rooms.values().map(|r| r.users.len()).sum::<usize>(),
            rooms.values().map(|r| r.chat_history.len()).sum::<usize>(),
        )
    };

    let connections = state
        .resource_monitor
        .total_connections
        .load(Ordering::Relaxed);
    let memory = state.memory_tracker.total_bytes.load(Ordering::Relaxed);
    let peak_memory = state.memory_tracker.peak_bytes.load(Ordering::Relaxed);
    let active_pool = state.connection_pool.active.load(Ordering::Relaxed);

    let metrics = format!(
        "# HELP chat_rooms_total Total number of active chat rooms\n\
         # TYPE chat_rooms_total gauge\n\
         chat_rooms_total {room_count}\n\
         # HELP chat_connections_total Total active WebSocket connections\n\
         # TYPE chat_connections_total gauge\n\
         chat_connections_total {connections}\n\
         # HELP chat_users_total Total users across all rooms\n\
         # TYPE chat_users_total gauge\n\
         chat_users_total {total_users}\n\
         # HELP chat_messages_total Total messages in memory\n\
         # TYPE chat_messages_total gauge\n\
         chat_messages_total {total_messages}\n\
         # HELP chat_memory_bytes Current memory usage in bytes\n\
         # TYPE chat_memory_bytes gauge\n\
         chat_memory_bytes {memory}\n\
         # HELP chat_memory_peak_bytes Peak memory usage in bytes\n\
         # TYPE chat_memory_peak_bytes gauge\n\
         chat_memory_peak_bytes {peak_memory}\n\
         # HELP chat_connection_pool_active Active connections in pool\n\
         # TYPE chat_connection_pool_active gauge\n\
         chat_connection_pool_active {active_pool}\n"
    );

    ([("Content-Type", "text/plain; version=0.0.4")], metrics).into_response()
}
