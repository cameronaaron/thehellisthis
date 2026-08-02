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

/// The client, compiled into the binary.
///
/// `include_str!` rather than a file read: the container image then has exactly
/// one artifact that can be out of date with itself, and serving either costs
/// no syscall.
pub(crate) const CLIENT_HTML: &str = include_str!("../../index.html");
const CLIENT_JS: &str = include_str!("../../client.js");

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

/// The one thing wrong with a room name, in the order a visitor would want it
/// reported. `None` means the name itself is fine — it says nothing about
/// capacity, which needs the room map and so is a separate check in the
/// caller.
///
/// A pure function of the string alone, rather than inline in the handler, so
/// the ordering constraint #4 (CLAUDE.md) depends on — reserved, then length,
/// then shape — is one a test can assert directly against a room name, not
/// only by reading a full HTTP response.
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
/// Returns `Response` rather than `impl IntoResponse` so the happy path can
/// serve `CLIENT_PAGE` by reference. `impl Trait` in return position must
/// resolve to one concrete type for the whole function, and the rejection
/// branches return owned strings built per request — unifying against them
/// forced the common case, a page load for a room in good standing, to
/// `.clone()` the ~70 KiB client page on every request rather than share the
/// one already sitting in `CLIENT_PAGE`. `main_room_handler` never had this
/// problem: it has only one branch, so it was never forced to match a type
/// that required an allocation.
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
