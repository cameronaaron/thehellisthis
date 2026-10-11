//! Process startup: the router, the housekeeping loops and the shutdown
//! sequence.
//!
//! Separate from `main.rs` so a test can call all of it. `main` can't be
//! called, so anything left in that file is unreachable, and its coverage
//! exemption is honest only while it holds nothing but the entry point
//! (scripts/coverage-exemptions.toml).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, routing::get};
use axum_server::Server;
use http::{HeaderValue, Method, header};
use tower_http::cors::{Any, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{error, info};

use crate::cleanup::cleanup_rooms;
use crate::config::{RESOURCE_CLEANUP_INTERVAL, ROOM_CLEANUP_INTERVAL, VERSION};
use crate::routes::{
    admin_dashboard_handler, app_js_handler, health_handler, main_room_handler, metrics_handler,
    nova_js_handler, nova_wasm_handler, robots_txt_handler, room_handler, root_redirect,
};
use crate::security::security_header_layers;
use crate::session::{nova_operator_ws_handler, ws_handler};
use crate::state::AppState;

pub(crate) const DEFAULT_PORT: u16 = 3000;

/// Builds the router. Separate from `main` so tests can exercise the real
/// route table rather than a hand-assembled approximation of it.
pub(crate) fn build_router(state: Arc<AppState>) -> Router {
    // The client is same-origin and needs no CORS. `/health` is the one
    // route an external monitor might poll cross-origin, so the layer goes on
    // that route alone; as a router-wide `.layer()` it once put
    // `access-control-allow-origin: *` on every page. `/metrics` and `/admin`
    // need an `Authorization` header this policy wouldn't pass a preflight
    // anyway, real scrapers aren't browsers, and `/admin` names every open
    // room, which no other origin should get.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET])
        .allow_headers([header::CONTENT_TYPE]);

    // Cloudflare's edge cache (`cache.enabled` in wrangler.jsonc) is
    // checked before this binary runs, so a hit skips `room_handler`'s
    // checks, `admin_dashboard_handler`'s auth and `metrics_handler`'s
    // gauges. Only `/app.js` is meant to be cached (content-addressed and
    // `immutable`, constraint #13); every other dynamic route gets
    // `no-store`, so the cache can't serve a stale room-full page, a stale
    // gauge, or one visitor's authenticated `/admin` page to the next
    // unauthenticated request.
    let no_store = || {
        SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        )
    };

    let mut router = Router::new()
        .route("/", get(root_redirect))
        .route("/main", get(main_room_handler).layer(no_store()))
        .route("/app.js", get(app_js_handler))
        .route("/nova.js", get(nova_js_handler))
        .route("/nova_wasm_bg.wasm", get(nova_wasm_handler))
        .route(
            "/health",
            get(health_handler)
                .layer::<_, std::convert::Infallible>(cors)
                .layer(no_store()),
        )
        .route("/metrics", get(metrics_handler).layer(no_store()))
        .route("/admin", get(admin_dashboard_handler).layer(no_store()))
        .route("/robots.txt", get(robots_txt_handler))
        // A real route, not a room name. axum resolves a literal segment
        // before a `{room}` capture whatever the order, but registering it
        // first keeps that from being load-bearing.
        .route("/ws/nova-operator", get(nova_operator_ws_handler))
        .route("/ws/{room}", get(ws_handler))
        // Last: every other single segment is a room name.
        .route("/{room}", get(room_handler).layer(no_store()));

    for layer in security_header_layers() {
        router = router.layer(layer);
    }

    router.with_state(state)
}

/// Spawns the two housekeeping loops: detached for the process's life,
/// dropped with the runtime at shutdown, since a half-run pass on a dying
/// process has nothing to preserve.
pub(crate) fn spawn_housekeeping(state: &Arc<AppState>) {
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

/// The port from `PORT`, else [`DEFAULT_PORT`]. A malformed value falls back
/// rather than panicking, so a deploy-config typo can't become a crash loop.
pub(crate) fn resolve_port(raw: Option<&str>) -> u16 {
    raw.and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_PORT)
}

pub(crate) fn init_tracing() {
    // Honours RUST_LOG (the container sets `info`). A bare `cargo run`
    // gets this crate at `debug` and dependencies at `info`, mainly for
    // nova's seal/open trail (session/nova.rs), which helps a developer
    // at a terminal but not production's log volume.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,infinite_chat=debug"));

    // Colour only for a person at a terminal: in a file or a log
    // collector ANSI escapes break `grep`, and this project's smoke
    // script once matched nothing and reported success because of them.
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());

    // `try_init` rather than `init`: a second call must not panic, which is
    // what lets tests call this.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .try_init();
}

/// Serves until `shutdown` resolves. Takes a bound listener so a test can
/// bind port 0 and drive the server exactly as production runs it.
pub(crate) async fn serve(
    state: Arc<AppState>,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let app = build_router(state);
    let server = Server::<SocketAddr>::from_listener(listener)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>());

    tokio::select! {
        result = server => log_server_result(result),
        () = shutdown => info!("shutdown complete"),
    }
}

/// Reports how the server stopped. A function so the error path is testable
/// without a server failing at an exact instant (§6.4).
pub(crate) fn log_server_result(result: std::io::Result<()>) {
    if let Err(e) = result {
        error!(error = %e, "server error");
    }
}

/// Resolves when the process is asked to stop, after announcing departures.
/// Cloudflare stops containers with SIGINT, so this turns a scale-down into
/// "user left" instead of a silent drop.
pub(crate) async fn wait_then_announce(
    state: Arc<AppState>,
    signal: impl std::future::Future<Output = std::io::Result<()>> + Send,
) {
    if signal.await.is_err() {
        error!("failed to listen for shutdown signal");
        // Never returning is correct: without a working signal handler there is
        // no shutdown to wait for, and resolving here would stop the server.
        std::future::pending::<()>().await;
    }

    announce_shutdown(&state).await;
}

/// What happens once the stop signal arrives. Split from the wait because a
/// test can't deliver a real SIGINT to itself; announcing departures, the part
/// that matters, needs no signal.
pub(crate) async fn announce_shutdown(state: &Arc<AppState>) {
    info!("shutdown signal received");
    state.shutdown().await;
}

/// Binds the listening socket. Separate from [`run`] so a test reaches the one
/// failure (port in use), the difference between starting and crash-looping.
pub(crate) async fn bind_listener(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, version = VERSION, "infinite-chat listening");
    Ok(listener)
}

/// Everything the process does, minus the runtime and the signal source. It
/// takes the shutdown future, not `ctrl_c()`, so a test can run bind, serve and
/// shutdown end to end.
pub(crate) async fn run(
    state: Arc<AppState>,
    port: u16,
    shutdown: impl std::future::Future<Output = std::io::Result<()>> + Send + 'static,
) -> std::io::Result<()> {
    spawn_housekeeping(&state);
    let listener = bind_listener(port).await?;
    serve(state.clone(), listener, wait_then_announce(state, shutdown)).await;
    Ok(())
}

/// Everything the process does, as a value: returning `ExitCode` instead of
/// calling `std::process::exit`, which never returns and would end the test
/// runner.
pub(crate) async fn main_inner(
    shutdown: impl std::future::Future<Output = std::io::Result<()>> + Send + 'static,
) -> std::process::ExitCode {
    init_tracing();

    let port = resolve_port(std::env::var("PORT").ok().as_deref());

    match run(Arc::new(AppState::new()), port, shutdown).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, port, "failed to bind");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Test-only helper for generating room names that look like the real ones.
#[cfg(test)]
pub(crate) fn generate_random_room_name() -> String {
    use rand::seq::IndexedRandom;

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

    let mut rng = rand::rng();
    let adjective = ADJECTIVES.choose(&mut rng).unwrap_or(&"hidden");
    let noun = NOUNS.choose(&mut rng).unwrap_or(&"room");
    format!("{adjective}-{noun}")
}
