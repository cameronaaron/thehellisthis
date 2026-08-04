//! Process startup: the router, the housekeeping loops, and the shutdown
//! sequence.
//!
//! Separated from `main.rs` so every one of these can be called by a test.
//! `main` cannot be — the runtime attribute makes it the program's entry
//! point — so anything left in that file is a line no test can ever reach, and
//! the coverage exemption for it is only honest while there is nothing there
//! but the entry point (scripts/coverage-exemptions.toml).

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
use crate::session::ws_handler;
use crate::state::AppState;

pub(crate) const DEFAULT_PORT: u16 = 3000;

/// Builds the router. Separate from [`main`] so tests can exercise the real
/// route table rather than a hand-assembled approximation of it.
pub(crate) fn build_router(state: Arc<AppState>) -> Router {
    // The client is same-origin and needs no CORS grant at all. `/health` is
    // the one route an external monitor might poll cross-origin, so it is the
    // only route this applies to — attached directly to that route's
    // `MethodRouter` rather than the whole router with `.layer()`, which used
    // to put `access-control-allow-origin: *` on every route including `/main`
    // and every room page, far past what the comment here claimed it granted.
    // `/metrics` does not get it either: it requires an `Authorization` header
    // this policy does not allow through a CORS preflight, so granting it here
    // would not have made cross-origin scraping of `/metrics` work anyway —
    // real scrapers (Prometheus, curl) are not browsers and CORS is a
    // browser-enforced restriction that does not apply to them regardless.
    // `/admin` is the same story: an `Authorization: Basic` header a preflight
    // would never let through, and it also names every open room, which is
    // exactly the kind of response no other origin should be handed anyway.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET])
        .allow_headers([header::CONTENT_TYPE]);

    // Cloudflare's edge cache (`cache.enabled` in wrangler.jsonc) checks for a
    // cached response *before this binary ever runs* — a cache hit means
    // `room_handler`'s capacity/reserved-name checks, `admin_dashboard_handler`'s
    // auth check, and every gauge `metrics_handler` reads never execute at all.
    // `/app.js` is the one response actually meant to be cached (constraint
    // #13's content-addressed, `immutable` URL); every other route that isn't
    // static markup gets an explicit `no-store` so turning the edge cache on
    // cannot silently serve a stale room-full page, a stale gauge, or — the
    // sharp edge — one visitor's authenticated `/admin` response to the next
    // request that happens to land on the same cached URL with no credentials
    // at all.
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
        .route("/ws/{room}", get(ws_handler))
        // Last: every other single segment is a room name.
        .route("/{room}", get(room_handler).layer(no_store()));

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

/// The port to listen on, from `PORT`, falling back to [`DEFAULT_PORT`].
///
/// A malformed value falls back rather than panicking: the container sets this
/// variable, and a typo in a deploy config should not turn into a crash loop
/// that takes the site down.
pub(crate) fn resolve_port(raw: Option<&str>) -> u16 {
    raw.and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_PORT)
}

pub(crate) fn init_tracing() {
    // Honours RUST_LOG; the container sets it to `info`.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Colour only when a person is watching. Redirected to a file or a
    // container's log collector, ANSI escapes are noise that breaks every
    // `grep` an operator writes — including this project's own smoke script,
    // which silently matched nothing and reported success because of them.
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());

    // `try_init` rather than `init`: a second call must not panic, which is
    // what lets tests call this.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .try_init();
}

/// Serves until `shutdown` resolves.
///
/// Takes an already-bound listener so a test can bind port 0, learn the real
/// port, and drive the whole server the way production runs it — rather than
/// asserting against a reassembled approximation of it.
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

/// Reports how the server stopped.
///
/// A function rather than an inline arm so the error path can be exercised
/// without arranging for a live server to fail mid-flight — which needs the
/// socket to break at an exact instant, the kind of test §6.4 rules out.
pub(crate) fn log_server_result(result: std::io::Result<()>) {
    if let Err(e) = result {
        error!(error = %e, "server error");
    }
}

/// Resolves when the process is asked to stop, having announced departures.
///
/// Cloudflare Containers stop instances with SIGINT, so a clean exit here is
/// what turns a routine scale-down into "user left" rather than a silent socket
/// drop for everyone in the room.
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

/// What happens once the stop signal has arrived.
///
/// Split from the waiting so it can be tested: `shutdown_signal` blocks on a
/// real SIGINT, which a test cannot deliver without killing the test runner,
/// but everything that matters — announcing the departures — is here and takes
/// no signal at all. The two remaining lines up there are the wait itself.
pub(crate) async fn announce_shutdown(state: &Arc<AppState>) {
    info!("shutdown signal received");
    state.shutdown().await;
}

/// Binds the listening socket for `port`.
///
/// Separate from [`run`] so the failure path is reachable from a test: binding
/// a port already in use is the one thing that goes wrong here, and it is the
/// difference between a container that starts and one that crash-loops.
pub(crate) async fn bind_listener(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, version = VERSION, "infinite-chat listening");
    Ok(listener)
}

/// Everything the process does, minus the runtime and the signal source.
///
/// Takes the shutdown future rather than reaching for `ctrl_c()`, so a test can
/// drive the whole lifecycle — bind, serve, shut down, return — without needing
/// to deliver a real signal to itself.
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

/// Everything the process does, as a value rather than an exit.
///
/// Returning `ExitCode` instead of calling `std::process::exit` is what lets a
/// test run this: `exit` does not return, so a test that reached it would take
/// the test runner with it.
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
