//! The operator dashboard: every open room, as a link, behind its own
//! password.

use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use http::{HeaderMap, StatusCode, header};

use crate::state::AppState;

/// Operator dashboard: every room currently open, as a clickable link, with
/// its connected-user count.
///
/// Gated by `security::is_authorized_for_admin` on its own token
/// (`ADMIN_TOKEN`) rather than `METRICS_TOKEN` — this discloses room names,
/// which `/metrics` deliberately does not. An unauthorized request gets a real
/// `401` with a `WWW-Authenticate` challenge rather than the `404` `/metrics`
/// uses, so a browser visiting the bookmarked link raises its native password
/// prompt instead of failing silently; see that function's doc comment for
/// why the two endpoints answer differently on purpose.
///
/// Room names are rendered through `ammonia::clean_text` before going into the
/// page. Every name in `state.rooms` already passed `matches_room_name_shape`
/// to get there, so this is defense in depth rather than a gap it closes —
/// cheap enough that the belt is worth wearing alongside the suspenders on a
/// page whose entire job is exposing data to an operator's browser.
pub async fn admin_dashboard_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Response {
    if !crate::security::is_authorized_for_admin(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"admin\"")],
        )
            .into_response();
    }

    let mut rooms: Vec<(String, usize)> = {
        let guard = state.rooms.read().await;
        guard
            .iter()
            .map(|(name, room)| (name.clone(), room.connected_user_count()))
            .collect()
    };
    rooms.sort_by(|a, b| a.0.cmp(&b.0));

    Html(render_admin_dashboard(&rooms)).into_response()
}

/// The dashboard page's markup, given the rooms to list.
///
/// A pure function of already-fetched data rather than inline in the async
/// handler above, so the escaping and the empty-state text are one thing a
/// test can check directly against a `Vec`, without a state, a lock, or an
/// HTTP request to get there.
pub(crate) fn render_admin_dashboard(rooms: &[(String, usize)]) -> String {
    let rows = if rooms.is_empty() {
        "<li>No open rooms.</li>".to_string()
    } else {
        rooms
            .iter()
            .map(|(name, count)| {
                let safe = ammonia::clean_text(name);
                format!("<li><a href=\"/{safe}\">{safe}</a> — {count} connected</li>")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <title>Open rooms</title></head><body><h1>Open rooms ({room_count})</h1>\
         <ul>{rows}</ul></body></html>",
        room_count = rooms.len(),
    )
}
