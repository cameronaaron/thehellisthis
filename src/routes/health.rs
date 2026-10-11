//! The liveness probe the container runtime polls, and the robots file that
//! keeps crawlers out of the WebSocket path.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{Json, extract::State, response::IntoResponse};
use serde::Serialize;

use crate::config::VERSION;
use crate::state::AppState;

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
