//! infinite-chat — an ephemeral, room-based WebSocket chat server.
//!
//! Rooms are created by visiting them and deleted when nobody is talking. There
//! is no database and no account system: all state is in memory and all of it
//! is designed to disappear. See `CLAUDE.md` for the map and
//! `ENGINEERING-STANDARDS.md` for why each rule here exists.

mod animals;
mod cleanup;
mod config;
mod emoji;
mod error;
mod identity;
mod limits;
mod protocol;
mod room;
mod routes;
mod security;
mod session;
mod state;
mod validation;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, routing::get};
use axum_server::Server;
use http::{Method, header};
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

use crate::cleanup::cleanup_rooms;
use crate::config::{RESOURCE_CLEANUP_INTERVAL, ROOM_CLEANUP_INTERVAL, VERSION};
use crate::routes::{
    app_js_handler, health_handler, main_room_handler, metrics_handler, robots_txt_handler,
    room_handler, root_redirect,
};
use crate::security::security_header_layers;
use crate::session::ws_handler;
use crate::state::AppState;

const DEFAULT_PORT: u16 = 3000;

/// Builds the router. Separate from [`main`] so tests can exercise the real
/// route table rather than a hand-assembled approximation of it.
fn build_router(state: Arc<AppState>) -> Router {
    // The client is same-origin, so CORS is needed only so the operational
    // endpoints can be scraped. It is restricted to GET and grants no
    // credentials — it used to allow every method from every origin, which is
    // reach nothing here ever needed.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET])
        .allow_headers([header::CONTENT_TYPE]);

    let mut router = Router::new()
        .route("/", get(root_redirect))
        .route("/main", get(main_room_handler))
        .route("/app.js", get(app_js_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/robots.txt", get(robots_txt_handler))
        .route("/ws/{room}", get(ws_handler))
        // Last: every other single segment is a room name.
        .route("/{room}", get(room_handler))
        .layer(cors);

    for layer in security_header_layers() {
        router = router.layer(layer);
    }

    router.with_state(state)
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

/// The port to listen on, from `PORT`, falling back to [`DEFAULT_PORT`].
///
/// A malformed value falls back rather than panicking: the container sets this
/// variable, and a typo in a deploy config should not turn into a crash loop
/// that takes the site down.
fn resolve_port(raw: Option<&str>) -> u16 {
    raw.and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_PORT)
}

fn init_tracing() {
    // Honours RUST_LOG; the container sets it to `info`.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // `try_init` rather than `init`: a second call must not panic, which is
    // what lets tests call this.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Serves until `shutdown` resolves.
///
/// Takes an already-bound listener so a test can bind port 0, learn the real
/// port, and drive the whole server the way production runs it — rather than
/// asserting against a reassembled approximation of it.
async fn serve(
    state: Arc<AppState>,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let app = build_router(state);
    let server = Server::<SocketAddr>::from_listener(listener)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>());

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                error!(error = %e, "server error");
            }
        }
        () = shutdown => info!("shutdown complete"),
    }
}

/// Resolves when the process is asked to stop, having announced departures.
///
/// Cloudflare Containers stop instances with SIGINT, so a clean exit here is
/// what turns a routine scale-down into "user left" rather than a silent socket
/// drop for everyone in the room.
async fn shutdown_signal(state: Arc<AppState>) {
    if signal::ctrl_c().await.is_err() {
        error!("failed to listen for shutdown signal");
        // Never returning is correct: without a working signal handler there is
        // no shutdown to wait for, and resolving here would stop the server.
        std::future::pending::<()>().await;
    }

    info!("shutdown signal received");
    state.shutdown().await;
}

#[tokio::main]
async fn main() {
    init_tracing();

    let state = Arc::new(AppState::new());
    spawn_housekeeping(&state);

    let port = resolve_port(std::env::var("PORT").ok().as_deref());
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!(error = %e, %addr, "failed to bind");
            std::process::exit(1);
        }
    };

    info!(%addr, version = VERSION, "infinite-chat listening");
    serve(state.clone(), listener, shutdown_signal(state)).await;
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
