//! infinite-chat — an ephemeral, room-based WebSocket chat server.
//!
//! Rooms are created by visiting them and deleted when nobody is talking. There
//! is no database and no account system: all state is in memory and all of it
//! is designed to disappear. See `CLAUDE.md` for the map and
//! `ENGINEERING-STANDARDS.md` for why each rule here exists.

mod animals;
mod cleanup;
mod config;
mod error;
mod identity;
mod limits;
mod protocol;
mod room;
mod routes;
mod session;
mod state;
mod validation;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, routing::get};
use axum_server::Server;
use http::header;
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

use crate::cleanup::cleanup_rooms;
use crate::config::{RESOURCE_CLEANUP_INTERVAL, ROOM_CLEANUP_INTERVAL, VERSION};
use crate::routes::{
    health_handler, main_room_handler, metrics_handler, robots_txt_handler, room_handler,
    root_redirect,
};
use crate::session::ws_handler;
use crate::state::AppState;

const DEFAULT_PORT: u16 = 3000;

/// Builds the router. Separate from [`main`] so tests can exercise the real
/// route table rather than a hand-assembled approximation of it.
fn build_router(state: Arc<AppState>) -> Router {
    // The client is served from the same origin it connects back to, so CORS
    // exists only for the operational endpoints. It grants no credentials.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    Router::new()
        .route("/", get(root_redirect))
        .route("/main", get(main_room_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/robots.txt", get(robots_txt_handler))
        .route("/ws/{room}", get(ws_handler))
        // Last: every other single segment is a room name.
        .route("/{room}", get(room_handler))
        .layer(cors)
        .with_state(state)
}

/// Spawns the two housekeeping loops.
///
/// Detached tasks with no join handle: they run for the process lifetime, and
/// shutdown drops them with the runtime rather than draining them — a half-run
/// cleanup pass on a dying process has nothing to preserve.
fn spawn_housekeeping(state: &Arc<AppState>) {
    let rooms_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(ROOM_CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            cleanup_rooms(&rooms_state).await;
        }
    });

    let resource_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(RESOURCE_CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            resource_state.cleanup().await;
        }
    });
}

#[tokio::main]
async fn main() {
    // Honours RUST_LOG; the container sets it to `info`.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let state = Arc::new(AppState::new());
    spawn_housekeeping(&state);

    // Cloudflare Containers stop instances with SIGINT, so a clean exit here is
    // what turns a routine scale-down into a "user left" rather than a silent
    // socket drop for everyone in the room.
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let shutdown_state = state.clone();
    tokio::spawn(async move {
        if signal::ctrl_c().await.is_err() {
            error!("failed to listen for shutdown signal");
            return;
        }
        info!("shutdown signal received");
        shutdown_state.shutdown().await;
        let _ = shutdown_tx.send(());
    });

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!(%addr, version = VERSION, "infinite-chat listening");

    let app = build_router(state);
    let server = Server::bind(addr).serve(app.into_make_service_with_connect_info::<SocketAddr>());

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                error!(error = %e, "server error");
            }
        }
        _ = shutdown_rx => info!("shutdown complete"),
    }
}

/// Test-only helper for generating room names that look like the real ones.
#[cfg(test)]
fn generate_random_room_name() -> String {
    use rand::prelude::SliceRandom;

    const ADJECTIVES: &[&str] = &[
        "latent",
        "mellow",
        "shiny",
        "mystic",
        "curious",
        "whimsical",
        "cosmic",
        "hidden",
        "vivid",
        "serendipitous",
        "obscure",
        "nebular",
        "celestial",
        "fae",
        "ethereal",
    ];
    const NOUNS: &[&str] = &[
        "toy",
        "garden",
        "forest",
        "ocean",
        "cavern",
        "nebula",
        "playground",
        "bazaar",
        "temple",
        "dojo",
        "lair",
        "grove",
        "spire",
        "oasis",
        "realm",
    ];

    let mut rng = rand::thread_rng();
    let adjective = ADJECTIVES.choose(&mut rng).unwrap_or(&"hidden");
    let noun = NOUNS.choose(&mut rng).unwrap_or(&"room");
    format!("{adjective}-{noun}")
}
