//! Process startup and the housekeeping loops.

use super::*;

#[tokio::test]
async fn test_reserved_path_main() {
    let app = Router::new().route("/main", get(main_room_handler));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_reserved_path_admin_rejected() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Invalid room name"));
}

#[tokio::test]
async fn test_reserved_path_api_rejected() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Invalid room name"));
}

/// The port falls back rather than crashing on a bad value.
///
/// A typo in a deploy variable should not become a crash loop that takes the
/// site down; the previous form `.expect("PORT must be a number")` would have.
#[test]
fn port_resolution_falls_back_instead_of_crashing() {
    assert_eq!(resolve_port(Some("8080")), 8080);
    assert_eq!(resolve_port(Some("  8080  ")), 8080);
    assert_eq!(resolve_port(None), DEFAULT_PORT);

    // Anything unusable falls back, including a port that would mean "any".
    for bad in ["", "not-a-port", "-1", "99999", "0", "80.5"] {
        assert_eq!(
            resolve_port(Some(bad)),
            DEFAULT_PORT,
            "{bad:?} should fall back to the default"
        );
    }
}

/// Logging setup is safe to call more than once.
#[test]
fn tracing_initialisation_is_idempotent() {
    init_tracing();
    init_tracing();
}

/// `serve` runs the real router and stops when its shutdown future resolves.
///
/// Binding port 0 and driving the actual entry-point function is what makes
/// this a test of the server rather than of a reassembled approximation of it.
#[tokio::test]
async fn serve_answers_requests_and_stops_on_shutdown() {
    let state = Arc::new(AppState::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(state, listener, async move {
        let _ = shutdown_rx.await;
    }));

    // The real route table is being served.
    let mut attempt = 0;
    let body = loop {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => break reqwest_health(addr).await,
            Err(_) if attempt < 20 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(e) => panic!("server never accepted a connection: {e}"),
        }
    };
    assert!(body.contains("healthy"), "unexpected /health body: {body}");

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("serve should stop once its shutdown future resolves")
        .unwrap();
}

// ========== EVENT APPLICATION GUARDS ==========

/// `run` serves on the port it was given, and stops when told to.
#[tokio::test]
async fn run_serves_until_it_is_shut_down() {
    let state = Arc::new(AppState::new());

    // Port 0 lets the OS choose, but then `run` owns the listener and the test
    // cannot learn the port — so bind first to find a free one, release it, and
    // hand the number over.
    let scout = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = scout.local_addr().unwrap().port();
    drop(scout);

    let serving = tokio::spawn({
        let state = state.clone();
        async move { crate::startup::run(state, port, std::future::pending()).await }
    });

    // The server is up once it answers.
    let mut healthy = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        healthy,
        "`run` should be listening on the port it was given"
    );

    serving.abort();
}

/// The shutdown sequence runs when the signal resolves, and survives a broken
/// signal handler.
///
/// The signal is a parameter precisely so this is reachable: a test cannot send
/// itself a SIGINT without killing the test runner. Both arms matter — the
/// happy one announces departures, and the error arm is what stops a server
/// with no working signal handler from shutting itself down immediately.
#[tokio::test]
async fn the_shutdown_sequence_waits_for_its_signal() {
    let state = Arc::new(AppState::new());
    let mut receiver;
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        receiver = room.sender.subscribe();
        rooms.insert("closing".to_string(), room);
    }

    // A signal that has already arrived.
    crate::startup::wait_then_announce(state.clone(), std::future::ready(Ok(()))).await;

    assert!(
        receiver.try_recv().is_ok(),
        "a delivered signal must run the shutdown announcement"
    );
    assert!(state.rooms.read().await.is_empty());

    // A signal handler that failed to install must *not* resolve: returning
    // here would shut the server down the instant it started.
    let broken = crate::startup::wait_then_announce(
        Arc::new(AppState::new()),
        std::future::ready(Err(std::io::Error::other("no signal handler"))),
    );
    assert!(
        timeout(Duration::from_millis(150), broken).await.is_err(),
        "with no working signal handler there is no shutdown to wait for, so \
         this must never resolve"
    );
}

/// The process reports failure through its exit code rather than by exiting.
///
/// `std::process::exit` does not return, so a test that reached it would take
/// the test runner with it — which is why `main_inner` returns an `ExitCode`
/// and `main` is one line.
#[tokio::test]
async fn the_process_exit_code_reports_a_failed_bind() {
    // Occupy the port the server would use, so the bind fails.
    let occupied = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();

    // SAFETY: single-threaded within this test, and the value is removed
    // immediately after. `PORT` is what the container sets.
    // Held for the whole window: another test reading or clearing `PORT`
    // mid-flight is a race.
    let _env = PORT_ENV.lock().await;

    // SAFETY: the lock makes this the only test touching `PORT`, and the value
    // is removed before the lock is released.
    unsafe { std::env::set_var("PORT", port.to_string()) };
    let code = crate::startup::main_inner(std::future::pending()).await;
    unsafe { std::env::remove_var("PORT") };

    assert_eq!(
        format!("{code:?}"),
        format!("{:?}", std::process::ExitCode::FAILURE),
        "a bind failure must leave the process with a failing exit code"
    );
}

/// `run` completes cleanly when its shutdown fires.
///
/// The success path — bind, serve, shut down, return `Ok` — was unreachable
/// while `run` reached for `ctrl_c()` itself. Taking the signal as a parameter
/// is what makes the whole lifecycle testable in a few milliseconds.
#[tokio::test]
async fn run_returns_cleanly_when_its_shutdown_fires() {
    let scout = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = scout.local_addr().unwrap().port();
    drop(scout);

    let result = timeout(
        Duration::from_secs(5),
        crate::startup::run(
            Arc::new(AppState::new()),
            port,
            std::future::ready(Ok::<(), std::io::Error>(())),
        ),
    )
    .await
    .expect("an already-fired shutdown should stop the server promptly");

    assert!(
        result.is_ok(),
        "a clean shutdown is not an error: {result:?}"
    );
}

/// The process reports success when it stops cleanly.
#[tokio::test]
async fn the_process_exit_code_reports_a_clean_stop() {
    let scout = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = scout.local_addr().unwrap().port();
    drop(scout);

    // SAFETY: the value is set and removed within this test; `PORT` is what the
    // container sets.
    // Held for the whole window: another test reading or clearing `PORT`
    // mid-flight is a race.
    let _env = PORT_ENV.lock().await;

    // SAFETY: the lock makes this the only test touching `PORT`, and the value
    // is removed before the lock is released.
    unsafe { std::env::set_var("PORT", port.to_string()) };
    let code = timeout(
        Duration::from_secs(5),
        crate::startup::main_inner(std::future::ready(Ok::<(), std::io::Error>(()))),
    )
    .await
    .expect("an already-fired shutdown should stop the process promptly");
    unsafe { std::env::remove_var("PORT") };

    assert_eq!(
        format!("{code:?}"),
        format!("{:?}", std::process::ExitCode::SUCCESS),
        "stopping cleanly must leave a successful exit code"
    );
}

/// The server's public surface has not quietly shrunk either.
///
/// The client manifest exists because a bulk edit removed things nobody knew
/// were load-bearing. The server has the same exposure and a worse blast
/// radius: a config constant deleted is a limit that stops being enforced, and
/// a route unmounted is a page that stops existing — neither of which fails to
/// compile, because the tests that used them go away in the same edit.
///
/// Deliberately coarse. It is not asserting behaviour — every one of these has
/// its own contract for that — it is asserting *existence*, which is the thing
/// no other test checks.
#[test]
fn the_servers_public_surface_still_exists() {
    const CONFIG: &str = include_str!("../../config.rs.inventory");

    let config_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/config.rs"))
            .expect("config.rs should be readable");

    let mut gone: Vec<&str> = Vec::new();
    for line in CONFIG.lines() {
        let name = line.trim();
        if name.is_empty() || name.starts_with('#') {
            continue;
        }
        if !config_source.contains(&format!("const {name}")) {
            gone.push(name);
        }
    }

    assert!(
        gone.is_empty(),
        "these tunables are listed in config.rs.inventory and no longer exist \
         in config.rs. A deleted limit is a limit that stops being enforced, \
         and nothing fails to compile because the tests using it went in the \
         same edit. Restore it, or delete the line here in the same commit: \
         {gone:?}"
    );

    // The route table, which `router_mounts_every_public_route` exercises but
    // does not enumerate.
    let startup = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/startup.rs"))
        .expect("startup.rs should be readable");
    for route in [
        "/",
        "/main",
        "/app.js",
        "/health",
        "/metrics",
        "/admin",
        "/robots.txt",
        "/ws/{room}",
        "/{room}",
    ] {
        assert!(
            startup.contains(&format!("\"{route}\"")),
            "the route {route} is no longer mounted"
        );
    }
}
