// src/tests.rs

// Explicit per-module imports rather than a single `use crate::*`: when a test
// fails, the import list is what tells you which part of the server it belongs
// to, and third-party types are named here rather than inherited from whatever
// `main.rs` happened to import.
use crate::animals::ANIMAL_NAMES;
use crate::cleanup::{cleanup_rooms, cleanup_rooms_at, history_trim_target, is_abandoned};
use crate::config::*;
use crate::emoji::{REACTION_EMOJI, is_reaction_emoji};
use crate::error::ChatError;
use crate::identity::{OptionalUserCookie, create_user_cookies, parse_identity_cookies};
use crate::limits::{ConnectionPool, MemoryTracker, RateLimiter, ResourceMonitor, SecurityManager};
use crate::protocol::{
    Attachment, ClientEvent, OutgoingEvent, OutgoingMessage, ReplyInfo, SystemEvent,
    encode_broadcast,
};
use crate::room::{ConnectionState, RoomState, UserData, create_room, user_idle_for_too_long};
use crate::routes::{
    RoomNameRejection, admin_dashboard_handler, fnv1a, health_handler, main_room_handler,
    metrics_handler, render_admin_dashboard, robots_txt_handler, room_handler, room_name_rejection,
    root_redirect,
};
use crate::security::is_allowed_origin;
use crate::session::{
    admit_user, apply_client_event, apply_client_event_at, cleanup_user, render_off_thread,
    resolve_render, within_throttle, ws_handler,
};
use crate::startup::{
    DEFAULT_PORT, build_router, generate_random_room_name, init_tracing, resolve_port, serve,
    spawn_housekeeping,
};
use crate::state::{AppState, announce_departures};
use crate::validation::{
    extract_client_ip, hash_client_address, sanitize_attachment, sanitize_reply, validate_input,
    validate_message,
};

use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::response::IntoResponse;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use futures::future::join_all;
use futures::{SinkExt, StreamExt};
use http::HeaderMap;
use serde_json::Value as JsonValue;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::Barrier;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tower::util::ServiceExt;
use uuid::Uuid;

/// Builds a text frame.
///
/// `tungstenite` 0.27 changed `Message::Text` to take `Utf8Bytes` rather than
/// `String`. One helper rather than a conversion at each of the ~30 call sites,
/// so the next signature change is one edit.
fn text_frame(body: impl Into<tokio_tungstenite::tungstenite::Utf8Bytes>) -> WsMessage {
    WsMessage::Text(body.into())
}

/// Puts a real message in a room and returns its id.
///
/// Reactions are only accepted for messages the room actually holds, so a test
/// that reacts needs something to react *to*. Before that check existed these
/// tests used a bare `Uuid::new_v4()`, which is precisely the state a client
/// could put the server into: a reaction bucket for a message that never
/// existed, which nothing could ever evict.
fn message_in(room: &mut RoomState, tracker: &MemoryTracker, text: &str) -> Uuid {
    let id = Uuid::new_v4();
    room.add_message(
        OutgoingMessage {
            message_id: id,
            user_id: "author".to_string(),
            animal_name: "otter".to_string(),
            text: text.to_string(),
            timestamp: "1700000000000".to_string(),
            reply_to: None,
            attachment: None,
        },
        tracker,
    );
    id
}

/// The page's CSS and markup with comments removed.
///
/// §6.7: a test that greps the shipped bytes must not be able to match the
/// prose explaining the thing it forbids. Both sweeps below were written with
/// a comment describing the exact declaration they reject, and both failed on
/// their own documentation until they scanned this instead.
fn embedded_html_without_comments() -> String {
    let mut out = String::with_capacity(EMBEDDED_HTML.len());
    let mut rest = EMBEDDED_HTML;

    loop {
        // CSS block comments.
        let css = rest.find("/*");
        // HTML comments.
        let html = rest.find("<!--");

        let (start, close, skip) = match (css, html) {
            (Some(c), Some(h)) if c < h => (c, "*/", 2),
            (Some(_), Some(h)) => (h, "-->", 3),
            (Some(c), None) => (c, "*/", 2),
            (None, Some(h)) => (h, "-->", 3),
            (None, None) => break,
        };

        out.push_str(&rest[..start]);
        let after = &rest[start + skip..];
        match after.find(close) {
            Some(end) => rest = &after[end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }

    out.push_str(rest);
    out
}

/// Every individual selector opening a rule in the embedded stylesheet, one
/// entry per comma-separated alternative.
///
/// This codebase writes a multi-selector rule as one selector per line, each
/// ending in `,` except the last, which ends in ` {` — e.g.
/// `.app-bar-actions .nav-link,\n.app-bar-actions .mute-btn {`. A selector
/// scan that only looks at lines ending in ` {` sees `.mute-btn` and never
/// `.nav-link`: the first alternative in every multi-selector rule was
/// unchecked by both dead-CSS contracts below. Accumulating `,`-terminated
/// lines until the line that opens the block is what makes every alternative
/// visible, not just the last one.
fn css_rule_selectors(css: &str) -> Vec<String> {
    let mut selectors = Vec::new();
    let mut pending: Vec<String> = Vec::new();

    for line in css.lines() {
        let trimmed = line.trim();

        if let Some(head) = trimmed.strip_suffix(',') {
            pending.push(head.trim().to_string());
            continue;
        }

        if let Some(head) = trimmed.strip_suffix(" {") {
            pending.push(head.trim().to_string());
            selectors.extend(pending.drain(..).filter(|s| !s.is_empty()));
            continue;
        }

        // Any other line — a declaration, a blank line, a closing brace — ends
        // whatever selector group was accumulating. A `,` inside a property
        // value (there are none in this stylesheet, but nothing enforces that)
        // would otherwise leak into the next rule's selector list.
        pending.clear();
    }

    selectors
}

/// Every class token named in a single selector, e.g. `.a.b:hover .c` → `[a,
/// b, c]`.
///
/// A rule like `.system-message.error { … }` mentions `system-message` — but
/// only ever matches an element that *also* has `error`. It does not style a
/// bare `.system-message`. Checking "does the selector text contain this
/// substring" instead of extracting real tokens is how `.system-message` read
/// as styled when only its `.error`/`.warning`/`.success` modifiers existed —
/// no base rule, so `addSystemMessage`'s default `type = 'info'` case, and
/// every unmodified message, rendered with no padding, no radius, and no
/// centring at all.
fn classes_in_selector(selector: &str) -> Vec<String> {
    selector
        .match_indices('.')
        .map(|(i, _)| {
            selector[i + 1..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        })
        .filter(|c| !c.is_empty())
        .collect()
}

/// The client script with its `//` comments removed.
///
/// §6.7 again, for JavaScript: a scan for string-literal content must read
/// code, not prose. `classes_the_page_can_render`'s broad single-quote walk
/// treats every `'` in the *entire file* as opening or closing a literal, with
/// no idea which ones sit inside a `///` doc comment — so an English
/// possessive like "the room's real remaining life" shifted every quote
/// pairing after it, and classes as central as `.message.sent` briefly read
/// as dead because the walker was no longer looking at the bytes it thought it
/// was. No `//` appears inside a real string literal in this file today
/// (checked before relying on it); if that ever changes this needs the
/// state-machine treatment `strip_hash_comments` never needed either.
fn strip_js_comments(script: &str) -> String {
    script
        .lines()
        .map(|line| match line.find("//") {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every class the page renders, from markup, assignments and template strings.
fn classes_the_page_can_render() -> std::collections::HashSet<String> {
    let mut alive: std::collections::HashSet<String> = std::collections::HashSet::new();

    for source in [EMBEDDED_HTML, EMBEDDED_JS] {
        for (index, _) in source.match_indices("class=\"") {
            let rest = &source[index + "class=\"".len()..];
            if let Some(end) = rest.find('"') {
                alive.extend(rest[..end].split_whitespace().map(str::to_string));
            }
        }
    }

    for marker in [
        "className = '",
        "className = `",
        "classList.add('",
        "classList.toggle('",
        "className: '",
    ] {
        for (index, _) in EMBEDDED_JS.match_indices(marker) {
            let rest = &EMBEDDED_JS[index + marker.len()..];
            let Some(end) = rest.find(['\'', '`']) else {
                continue;
            };

            // `reaction-pill${mine ? ' mine' : ''}` is one class name and one
            // expression. Cutting each `${…}` out leaves the literal parts;
            // splitting on whitespace first would throw `reaction-pill` away
            // along with the expression glued to it.
            let mut literal = String::new();
            let mut depth = 0usize;
            let mut chars = rest[..end].chars().peekable();
            while let Some(c) = chars.next() {
                if depth == 0 && c == '$' && chars.peek() == Some(&'{') {
                    chars.next();
                    depth = 1;
                    literal.push(' ');
                } else if depth > 0 {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                    }
                } else {
                    literal.push(c);
                }
            }
            alive.extend(literal.split_whitespace().map(str::to_string));
        }
    }

    // `row.className = \`message ${isSent ? 'sent' : 'received'} run-end\`;`
    // and `updateConnectionStatus('connected')` both put a class-shaped word
    // where the parsing above cannot reach it: one is inside a `${…}`
    // ternary, the other is a value passed to a function that assembles the
    // class somewhere else entirely. Neither is a pattern worth chasing
    // individually — the general shape is "a bare, lowercase, hyphenated word
    // in quotes", which in this file is overwhelmingly a class or state name.
    // Any single-quoted JS string literal of that shape counts as alive,
    // rather than trying to trace which ones a stylesheet selector consumes.
    let is_class_shaped = |s: &str| {
        !s.is_empty()
            && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    // Comments stripped first: an English possessive ("the room's real
    // remaining life") is an unpaired `'` that this walk cannot tell from a
    // string delimiter, and one apostrophe in a doc comment shifts every
    // pairing after it for the rest of the file.
    let js_without_comments = strip_js_comments(EMBEDDED_JS);
    let mut rest: &str = &js_without_comments;
    while let Some(start) = rest.find('\'') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('\'') else { break };
        let literal = &rest[..end];
        if is_class_shaped(literal) {
            alive.insert(literal.to_string());
        }
        rest = &rest[end + 1..];
    }

    alive
}

/// A workflow or shell script with its `#` comments removed.
///
/// §6.7: a sweep over a script must read the commands, not the prose about
/// them. The CSS sweeps needed the same thing and got
/// `embedded_html_without_comments`; this is that idea for `#`-commented files.
fn strip_hash_comments(script: &str) -> String {
    script
        .lines()
        .map(|line| match line.find('#') {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn start_ws_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/ws/{room}", get(ws_handler))
        .with_state(app_state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    );
    let handle = tokio::spawn(async move {
        let _ = server.await;
    });

    (addr, handle)
}

async fn recv_json_event(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> JsonValue {
    loop {
        let msg = timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("timed out waiting for ws event")
            .expect("ws stream ended")
            .expect("ws recv failed");

        match msg {
            WsMessage::Text(text) => {
                if let Ok(val) = serde_json::from_str::<JsonValue>(&text) {
                    return val;
                }
            }
            WsMessage::Binary(_)
            | WsMessage::Ping(_)
            | WsMessage::Pong(_)
            | WsMessage::Frame(_) => continue,
            WsMessage::Close(_) => panic!("ws closed unexpectedly"),
        }
    }
}

fn extract_system_event<'a>(event: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    if let Some(t) = event.get("type").and_then(|v| v.as_str())
        && t == key
    {
        return Some(event);
    }
    event.get(key)
}

/// The peak-memory compare-and-swap must survive concurrent writers.
///
/// `add_bytes` updates the high-water mark with a CAS loop; the retry arm only
/// runs when two threads race. Hammering it from many threads is what actually
/// exercises that arm, and asserts the invariant that matters: the peak is
/// never below the total.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn peak_memory_tracking_is_correct_under_concurrent_writers() {
    let tracker = Arc::new(MemoryTracker::new());
    let mut handles = Vec::new();

    for _ in 0..16 {
        let tracker = tracker.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..200 {
                tracker.add_bytes(64);
            }
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    let total = tracker.total_bytes.load(Ordering::SeqCst);
    let peak = tracker.peak_bytes.load(Ordering::SeqCst);

    assert_eq!(total, 16 * 200 * 64, "every reservation should be counted");
    assert!(peak >= total, "peak must never be below the running total");
}

/// A frame sink that records what it was given and can be told to fail.
///
/// The four connection tasks all end the same way — a send returns an error
/// because the peer is gone — and reaching that arm with a real socket means
/// making the peer vanish between two exact frames, which is a race no test
/// wins reliably. Making the tasks generic over their sink turns that race into
/// a parameter (§6.1c).
#[derive(Default)]
struct RecordingSink {
    sent: Vec<crate::session::Message>,
    fail_after: Option<usize>,
    /// Fail on a particular kind of frame rather than a count. The four tasks
    /// race, so "the third frame" is whichever task happened to win — refusing
    /// a *pong* is the only way to test the pong path deterministically.
    fail_on_pong: bool,
}

impl RecordingSink {
    fn failing_immediately() -> Self {
        Self {
            sent: Vec::new(),
            fail_after: Some(0),
            fail_on_pong: false,
        }
    }

    fn failing_after(n: usize) -> Self {
        Self {
            sent: Vec::new(),
            fail_after: Some(n),
            fail_on_pong: false,
        }
    }

    fn refusing_pongs() -> Self {
        Self {
            sent: Vec::new(),
            fail_after: None,
            fail_on_pong: true,
        }
    }
}

/// Lets one `RecordingSink` be both handed to a session and inspected by the
/// test, since `run_session` takes ownership of its sink.
struct SharedSink(Arc<tokio::sync::Mutex<RecordingSink>>);

impl crate::session::FrameSink for SharedSink {
    async fn send_frame(&mut self, message: crate::session::Message) -> Result<(), ()> {
        self.0.lock().await.send_frame(message).await
    }
}

impl crate::session::FrameSink for RecordingSink {
    async fn send_frame(&mut self, message: crate::session::Message) -> Result<(), ()> {
        if self.fail_on_pong && matches!(message, crate::session::Message::Pong(_)) {
            return Err(());
        }
        if self.fail_after.is_some_and(|n| self.sent.len() >= n) {
            return Err(());
        }
        self.sent.push(message);
        Ok(())
    }
}

/// The embedded HTML - same source used by the server
/// Serialises the tests that set `PORT`.
///
/// Environment variables are process-global and the suite runs in parallel, so
/// two tests setting and removing the same one race: either can read the
/// other's value, or find it already cleared. It did not fail locally and did
/// fail when cargo-mutants ran the baseline in a copied tree, which is the
/// worst shape a test failure comes in.
static PORT_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Serialises the tests that set `METRICS_TOKEN`, for the same reason
/// `PORT_ENV` exists: it is process-global, and the suite runs in parallel.
static METRICS_TOKEN_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Serialises the tests that set `ADMIN_TOKEN`, for the same reason
/// `METRICS_TOKEN_ENV` exists.
static ADMIN_TOKEN_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const EMBEDDED_HTML: &str = include_str!("../../index.html");

const EMBEDDED_JS: &str = include_str!("../../client.js");

/// The client as shipped: the page plus the script the page loads.
///
/// The two were one file until the client's JavaScript was moved out of an
/// inline `<script>` block so the Content-Security-Policy could refuse inline
/// script. Tests assert over both, because a behaviour can now live in either
/// and neither half alone is "the client".
static SHIPPED_CLIENT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!("{EMBEDDED_HTML}\n{EMBEDDED_JS}"));

/// Builds a WebSocket handshake request with arbitrary extra headers.
fn ws_request(addr: SocketAddr, room: &str, extra: &[(&str, String)]) -> http::Request<()> {
    let mut builder = http::Request::builder()
        .uri(format!("ws://{addr}/ws/{room}"))
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");

    for (name, value) in extra {
        builder = builder.header(*name, value);
    }
    builder.body(()).unwrap()
}

/// A 1x1 PNG, as the client would send it: base64, no `data:` prefix.
const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A minimal GIF87a header, enough to sniff.
const TINY_GIF: &str = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

fn png_attachment() -> Attachment {
    Attachment {
        mime: "image/png".to_string(),
        data: TINY_PNG.to_string(),
        width: 1,
        height: 1,
        faded: false,
    }
}

/// Captures `tracing` output so a test can assert what an operator would see.
///
/// Scoped, not global: `with_default` installs it for one closure, so this does
/// not fight the `init_tracing` other tests call. Without it, everything a
/// human reads to understand a running server — which room faded, who was
/// evicted — is behaviour nothing checks. Mutation testing made that concrete:
/// the guard around the trim log could be flipped three ways and no test
/// noticed, because no test read the log.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).to_string()
    }
}

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs `body` with logging captured, and returns what was logged.
async fn capturing_logs<F, Fut>(body: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    capturing_logs_at(tracing::Level::INFO, body).await
}

/// Same as [`capturing_logs`], but at a caller-chosen level.
///
/// `trace!`/`debug!` field expressions are only evaluated when a subscriber
/// has that level enabled (§6.1d) — a test that wants to assert on a `trace!`
/// or `debug!` line needs this, not the `INFO`-capped default, or the event
/// never fires at all and the line it comes from reads as dead code to
/// coverage tooling even though the branch around it genuinely ran.
async fn capturing_logs_at<F, Fut>(level: tracing::Level, body: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(level)
        .finish();

    // `with_default` is scoped to this task rather than global, so it does not
    // conflict with the subscriber `init_tracing` may already have installed.
    let guard = tracing::subscriber::set_default(subscriber);
    body().await;
    drop(guard);

    logs.text()
}

async fn start_ws_server_with_state() -> (SocketAddr, Arc<AppState>, tokio::task::JoinHandle<()>) {
    let app_state = Arc::new(AppState::new());
    let app = build_router(app_state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    );
    let handle = tokio::spawn(async move {
        let _ = server.await;
    });

    (addr, app_state, handle)
}

/// The housekeeping loops actually run on their intervals.
///
/// Asserted with simulated time rather than by waiting a real minute: without
/// this, a broken interval loop would be invisible to the suite — the tasks are
/// detached and nothing ever awaits them.
#[tokio::test(start_paused = true)]
async fn housekeeping_loops_run_on_their_interval() {
    let state = Arc::new(AppState::new());

    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        // Empty and long idle: the next sweep should delete it.
        room.last_activity = Instant::now() - EMPTY_ROOM_CLEANUP_DELAY - Duration::from_secs(60);
        rooms.insert("doomed-room".to_string(), room);
    }

    spawn_housekeeping(&state);

    // Advance past one room-cleanup interval and let the task run.
    tokio::time::advance(ROOM_CLEANUP_INTERVAL + Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    tokio::task::yield_now().await;

    let rooms = state.rooms.read().await;
    assert!(
        !rooms.contains_key("doomed-room"),
        "the housekeeping loop should have swept the idle empty room"
    );
}

/// Minimal HTTP/1.1 GET of /health, so the test does not add an HTTP client
/// dependency just to read one response body.
async fn reqwest_health(addr: SocketAddr) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).to_string()
}

/// Decodes a broadcast frame back into the event it encodes.
///
/// `RoomState::sender` carries the already-encoded frame (`Arc<str>`, from
/// `encode_broadcast`), not the event itself — every connection's
/// `forward_broadcasts` task shares that one encoding rather than redoing it,
/// which is the whole point (§1). Tests that used to receive an `OutgoingEvent`
/// straight off the channel and match on it now receive the frame and decode
/// it back, the same round trip a real client's `JSON.parse` makes.
fn decode_broadcast(frame: &str) -> OutgoingEvent {
    serde_json::from_str(frame).expect("a broadcast frame must decode as an OutgoingEvent")
}

fn connected_user(
    user_id: &str,
    animal: &str,
    connection_id: &str,
    heartbeat: Instant,
) -> UserData {
    let now = Instant::now();
    UserData {
        user_id: user_id.to_string(),
        animal_name: animal.to_string(),
        last_active: now,
        last_message_time: now,
        connection_state: ConnectionState::Connected {
            last_heartbeat: heartbeat,
            connection_id: connection_id.to_string(),
        },
        last_read_message: None,
        is_typing: false,
        last_typing_event: None,
        last_read_receipt_event: None,
        rate_limiter: RateLimiter::new(),
        last_message_text: None,
        last_reaction_event: None,
    }
}

async fn state_with_user(room: &str, user_id: &str, heartbeat: Instant) -> Arc<AppState> {
    let state = Arc::new(AppState::new());
    let mut rooms = state.rooms.write().await;
    let mut room_state = create_room();
    room_state.users.insert(
        user_id.to_string(),
        connected_user(user_id, "otter", "c1", heartbeat),
    );
    rooms.insert(room.to_string(), room_state);
    drop(rooms);
    state
}

/// The whole suite's source, for the sweeps that read the tests themselves.
///
/// Read from disk rather than `include_str!`d: the suite is a directory of
/// modules now, and a hand-written list of them would go stale the first time
/// somebody adds one — the failure `every_declared_dependency_is_used` already
/// had once.
fn suite_source() -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tests");
    let mut all = String::new();
    for entry in std::fs::read_dir(dir).expect("src/tests should be readable") {
        let path = entry.expect("a readable entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            all.push_str(&std::fs::read_to_string(&path).expect("a readable module"));
            all.push('\n');
        }
    }
    all
}

mod attachments;
mod ci;
mod client_ui_client_ip;
mod client_ui_protocol_events;
mod client_ui_rendering;
mod client_ui_security_policy;
mod constants;
mod contracts;
mod frontend_parity;
mod limits_connections;
mod limits_memory;
mod limits_misc;
mod limits_rate_limiting;
mod limits_security;
mod memory_budget;
mod observability;
mod protocol;
mod reactions;
mod rooms_animals;
mod rooms_app_state;
mod rooms_deletion;
mod rooms_history;
mod rooms_pruning;
mod rooms_routing;
mod routes;
mod session_broadcast;
mod session_frames;
mod session_identity;
mod session_join_and_room;
mod session_presence;
mod session_throttles;
mod startup;
mod validation;
