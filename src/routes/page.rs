//! The client page and script, and the room-name checks that decide whether a
//! visitor gets the page or a plain-text refusal.

use std::sync::Arc;
use std::sync::LazyLock;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use http::{HeaderMap, StatusCode, header};
use tracing::{debug, warn};

use crate::config::{MAX_ROOM_NAME_LEN, MAX_ROOMS, MIN_ROOM_NAME_LEN, RESERVED_ROOM_NAMES};
use crate::state::AppState;
use crate::validation::matches_room_name_shape;

/// The client, compiled into the binary: one artifact that can't be out of
/// date with itself, served without a syscall.
pub(crate) const CLIENT_HTML: &str = include_str!("../../index.html");
const CLIENT_JS: &str = include_str!("../../client.js");

/// `nova` only: `novachannel`'s browser bindings (`nova-wasm/`, built with
/// `wasm-pack build --target web` and committed), embedded for the same
/// reasons as [`CLIENT_HTML`]/[`CLIENT_JS`]. `client.js` loads them only in
/// the `roomName === NOVA_ROOM_NAME` branch, so other rooms' pages don't grow.
const NOVA_JS: &str = include_str!("../../nova-wasm/pkg/nova_wasm.js");
const NOVA_WASM: &[u8] = include_bytes!("../../nova-wasm/pkg/nova_wasm_bg.wasm");

/// FNV-1a (64-bit): a cache key, not a security control. Nothing trusts it;
/// it only has to change when the script does.
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

/// The script's content-derived version stamp, so `/app.js` can be `immutable`
/// for a year: the URL changes exactly when the file does.
static CLIENT_JS_VERSION: LazyLock<String> =
    LazyLock::new(|| format!("{:016x}", fnv1a(CLIENT_JS.as_bytes())));

static CLIENT_JS_ETAG: LazyLock<String> = LazyLock::new(|| format!("\"{}\"", *CLIENT_JS_VERSION));

/// `nova.js` and its wasm can't share `/app.js`'s `immutable` stamp:
/// `init()` loads `new URL('nova_wasm_bg.wasm', import.meta.url)`, and relative
/// resolution drops the query string. Both get an ETag and `no-cache`
/// (revalidated, a cheap 304 when unchanged). With `max-age=3600` the browser
/// and edge cached them separately, so a deploy within the hour could pair new
/// JS with old wasm: `wasm.novaclient_myKeyPackage is not a function` (§9.1b).
static NOVA_JS_ETAG: LazyLock<String> =
    LazyLock::new(|| format!("\"{:016x}\"", fnv1a(NOVA_JS.as_bytes())));
static NOVA_WASM_ETAG: LazyLock<String> =
    LazyLock::new(|| format!("\"{:016x}\"", fnv1a(NOVA_WASM)));

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

/// Serves the client script. Separate from the page so the CSP can refuse
/// inline script (`script-src 'self'`), which an inline block made impossible
/// without `'unsafe-inline'`. It's also the cacheable half: the page is per
/// room, the script the same for everyone.
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

/// Serves `nova`'s wasm glue module at a fixed path (`client.js` imports it
/// directly). `no-cache` makes it revalidate every request, so it and
/// [`nova_wasm_handler`] can't come from two deploys at once (see
/// [`NOVA_JS_ETAG`]).
pub async fn nova_js_handler(headers: HeaderMap) -> Response {
    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && if_none_match
            .split(',')
            .any(|tag| tag.trim() == *NOVA_JS_ETAG)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, no-cache"),
            (header::ETAG, NOVA_JS_ETAG.as_str()),
        ],
        NOVA_JS,
    )
        .into_response()
}

/// Serves `nova`'s wasm binary. The glue module fetches it relative to its own
/// URL, so this route must stay a sibling of `/nova.js`; `no-cache` for the
/// reason in [`nova_js_handler`].
pub async fn nova_wasm_handler(headers: HeaderMap) -> Response {
    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && if_none_match
            .split(',')
            .any(|tag| tag.trim() == *NOVA_WASM_ETAG)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    (
        [
            (header::CONTENT_TYPE, "application/wasm"),
            (header::CACHE_CONTROL, "public, no-cache"),
            (header::ETAG, NOVA_WASM_ETAG.as_str()),
        ],
        NOVA_WASM,
    )
        .into_response()
}

/// The one thing wrong with a room name, in the order a visitor wants it
/// reported; `None` means the name is fine (capacity needs the room map and is
/// checked by the caller). A pure function so constraint #4's order (reserved,
/// length, shape) is testable without a full response.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RoomNameRejection {
    Reserved,
    BadLength,
    BadShape,
}

impl RoomNameRejection {
    fn message(&self) -> &'static str {
        match self {
            Self::Reserved => "Invalid room name",
            Self::BadLength => "Room name must be between 3 and 50 characters",
            Self::BadShape => "Room name must start/end with alphanumeric chars",
        }
    }
}

pub(crate) fn room_name_rejection(room: &str) -> Option<RoomNameRejection> {
    if RESERVED_ROOM_NAMES.contains(&room) {
        return Some(RoomNameRejection::Reserved);
    }
    if room.len() < MIN_ROOM_NAME_LEN || room.len() > MAX_ROOM_NAME_LEN {
        return Some(RoomNameRejection::BadLength);
    }
    if !matches_room_name_shape(room) {
        return Some(RoomNameRejection::BadShape);
    }
    None
}

/// Serves the client for any valid room name.
///
/// Returns `Response`, not `impl IntoResponse`, so the happy path serves
/// `CLIENT_PAGE` by reference: one concrete return type forced it to match the
/// rejection branches' owned strings and clone the ~70 KiB page on every
/// request (§1.6).
pub async fn room_handler(
    Path(room): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    if let Some(reason) = room_name_rejection(&room) {
        match reason {
            RoomNameRejection::Reserved => {
                warn!(room = %room, "reserved path requested as a room");
            }
            RoomNameRejection::BadLength => {
                debug!(room = %room, "room name length out of range");
            }
            RoomNameRejection::BadShape => debug!(room = %room, "room name shape rejected"),
        }
        return Html(reason.message()).into_response();
    }

    let rooms = state.rooms.read().await;
    if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
        warn!(room = %room, limit = MAX_ROOMS, "room cap reached");
        return Html("Maximum number of rooms reached").into_response();
    }

    Html(CLIENT_PAGE.as_str()).into_response()
}
