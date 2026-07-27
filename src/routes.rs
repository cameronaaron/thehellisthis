//! Plain HTTP handlers. The chat itself is entirely over WebSocket; these
//! serve the page, the robots file, and the two operational endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tracing::{debug, warn};

use crate::config::{
    MAX_ROOM_NAME_LEN, MAX_ROOMS, MIN_ROOM_NAME_LEN, RESERVED_ROOM_NAMES, VERSION,
};
use crate::state::AppState;
use crate::validation::matches_room_name_shape;

/// The single-page client, compiled into the binary.
///
/// `include_str!` rather than a file read: the container image then has exactly
/// one artifact that can be out of date with itself, and serving the page costs
/// no syscall.
const CLIENT_HTML: &str = include_str!("../index.html");

pub async fn root_redirect() -> Redirect {
    Redirect::permanent("/main")
}

pub async fn main_room_handler() -> impl IntoResponse {
    Html(CLIENT_HTML)
}

/// Serves the client for any valid room name.
///
/// Checks run in the order a visitor would want them reported — what is wrong
/// with the name they typed first, capacity second. That order is asserted in
/// `tests.rs`; reordering it changes what users are told.
pub async fn room_handler(
    Path(room): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if RESERVED_ROOM_NAMES.contains(&room.as_str()) {
        warn!(room = %room, "reserved path requested as a room");
        return Html("Invalid room name".to_string());
    }

    if room.len() < MIN_ROOM_NAME_LEN || room.len() > MAX_ROOM_NAME_LEN {
        debug!(room = %room, "room name length out of range");
        return Html("Room name must be between 3 and 50 characters".to_string());
    }

    if !matches_room_name_shape(&room) {
        debug!(room = %room, "room name shape rejected");
        return Html("Room name must start/end with alphanumeric chars".to_string());
    }

    let rooms = state.rooms.read().await;
    if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
        warn!(room = %room, limit = MAX_ROOMS, "room cap reached");
        return Html("Maximum number of rooms reached".to_string());
    }

    Html(CLIENT_HTML.to_string())
}

pub async fn robots_txt_handler() -> impl IntoResponse {
    (
        [("Content-Type", "text/plain")],
        "User-agent: *\n\
         Allow: /\n\
         Disallow: /ws/\n\
         Disallow: /_*\n\
         Crawl-delay: 10\n\n\
         # Prevent access to WebSocket endpoints\n\
         Disallow: /ws/*\n",
    )
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub connections: usize,
    pub rooms: usize,
    pub memory_bytes: usize,
}

/// Liveness probe. The Cloudflare container runtime polls this to decide
/// whether the instance is up, so it must never take the room write lock.
pub async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let room_count = state.rooms.read().await.len();

    Json(HealthResponse {
        status: "healthy",
        version: VERSION,
        connections: state
            .resource_monitor
            .total_connections
            .load(Ordering::Relaxed),
        rooms: room_count,
        memory_bytes: state.memory_tracker.total_bytes.load(Ordering::Relaxed),
    })
}

/// Prometheus text exposition.
///
/// Hand-rolled rather than pulling in a metrics runtime: seven gauges read
/// straight from the atomics that already exist do not justify a registry, a
/// background exporter task, and two dependencies.
pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> Response {
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
