//! Plain HTTP handlers. The chat itself is entirely over WebSocket; these
//! serve the page, the robots file, and the two operational endpoints.

use std::sync::Arc;
use std::sync::LazyLock;

use axum::{
    Json,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use http::{HeaderMap, StatusCode, header};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tracing::{debug, warn};

use crate::config::{
    MAX_ROOM_NAME_LEN, MAX_ROOMS, MIN_ROOM_NAME_LEN, RESERVED_ROOM_NAMES, VERSION,
};
use crate::state::AppState;
use crate::validation::matches_room_name_shape;

/// The client, compiled into the binary.
///
/// `include_str!` rather than a file read: the container image then has exactly
/// one artifact that can be out of date with itself, and serving either costs
/// no syscall.
pub(crate) const CLIENT_HTML: &str = include_str!("../index.html");
const CLIENT_JS: &str = include_str!("../client.js");

/// FNV-1a (64-bit) — a cache key, not a security control.
///
/// A cryptographic hash would carry a dependency to solve a problem that does
/// not exist here: nothing trusts this value, it only has to change whenever
/// the script changes.
pub(crate) const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    hash
}

/// Version stamp for the script, derived from its content.
///
/// Lets `/app.js` be served `immutable` with a year-long lifetime: the URL
/// changes exactly when the file does, so a deploy can never serve a stale
/// script and a repeat visit never revalidates one.
static CLIENT_JS_VERSION: LazyLock<String> =
    LazyLock::new(|| format!("{:016x}", fnv1a(CLIENT_JS.as_bytes())));

static CLIENT_JS_ETAG: LazyLock<String> = LazyLock::new(|| format!("\"{}\"", *CLIENT_JS_VERSION));

/// The page with the script URL stamped, built once.
static CLIENT_PAGE: LazyLock<String> = LazyLock::new(|| {
    CLIENT_HTML.replace(
        "src=\"/app.js\"",
        &format!("src=\"/app.js?v={}\"", *CLIENT_JS_VERSION),
    )
});

pub async fn root_redirect() -> Redirect {
    Redirect::permanent("/main")
}

pub async fn main_room_handler() -> impl IntoResponse {
    Html(CLIENT_PAGE.as_str())
}

/// Serves the client script.
///
/// Split out of the page so the Content-Security-Policy can refuse inline
/// script outright (`script-src 'self'`). While the script was an inline block,
/// any policy permitting it had to allow `'unsafe-inline'`, which is precisely
/// the capability an injected `<script>` needs — on a server whose whole job is
/// turning user Markdown into HTML.
///
/// The separate file is also the cacheable half of the response: the page is
/// per-room, the script is identical for everyone.
pub async fn app_js_handler(headers: HeaderMap) -> Response {
    // The URL is content-addressed, so a matching ETag can always 304.
    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && if_none_match
            .split(',')
            .any(|tag| tag.trim() == *CLIENT_JS_ETAG)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::ETAG, CLIENT_JS_ETAG.as_str()),
        ],
        CLIENT_JS,
    )
        .into_response()
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

    Html(CLIENT_PAGE.clone())
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
