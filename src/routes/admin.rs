//! The operator dashboard: every open room, as a link, behind its own
//! password.

use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use http::{HeaderMap, StatusCode, header};

use crate::state::AppState;

/// Operator dashboard: every open room as a link, with its connected count.
///
/// Gated by `security::is_authorized_for_admin` on its own `ADMIN_TOKEN`,
/// since it discloses room names, which `/metrics` doesn't. A refusal is a real
/// 401 with a `WWW-Authenticate` challenge so the browser prompts (see that
/// function).
///
/// Room names pass through `ammonia::clean_text`; every stored name already
/// passed `matches_room_name_shape`, so this is defence in depth on the one
/// page that shows room names to a browser.
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

/// The dashboard markup for already-fetched rooms, a pure function so the
/// escaping and empty state are testable without state, a lock or a request.
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
