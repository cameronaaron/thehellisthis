//! The WebSocket upgrade: every check that can refuse a connection, the
//! reservations that follow once nothing has, and placing the visitor in
//! their room.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::extract::{ConnectInfo, Path, State, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, header};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::{
    MAX_CONCURRENT_CONNECTIONS_PER_IP, MAX_CONCURRENT_USERS, MAX_ROOM_NAME_LEN, NOVA_ROOM,
};
use crate::error::ChatError;
use crate::identity::{OptionalUserCookie, UserCookie, create_user_cookies};
use crate::limits::RateLimiter;
use crate::room::{ConnectionState, RoomState, UserData, create_room};
use crate::security::is_allowed_origin;
use crate::state::AppState;
use crate::validation::{extract_client_ip, hash_client_address, validate_input};

use super::lifecycle::handle_websocket;

/// Upgrades the connection to a WebSocket for `room`.
#[axum::debug_handler]
pub async fn ws_handler(
    State(state): State<Arc<AppState>>,
    conn_info: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    OptionalUserCookie(cookie): OptionalUserCookie,
    Path(room): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // WebSocket upgrades aren't covered by the same-origin policy: any page can
    // open a socket here, and the browser only reports the origin. Rejecting
    // foreign origins stops a third-party page driving visitors' browsers into
    // rooms and spending the connection budget.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());

    if !is_allowed_origin(origin, host) {
        warn!(?origin, "rejected websocket upgrade from foreign origin");
        return ChatError::SecurityError("Origin not allowed".to_string()).into_response();
    }

    // Hashed at the boundary: everything downstream only compares addresses,
    // so the real one is never stored, logged or left in a memory dump (§5.6).
    let ip = extract_client_ip(&headers, Some(&conn_info))
        .as_deref()
        .map(hash_client_address);

    let host = host.unwrap_or_default().to_string();

    match ws_handler_inner(state, ip, cookie, room, ws, &host).await {
        Ok(response) => response.into_response(),
        Err(e) => e.into_response(),
    }
}

/// Every check that can refuse a connection, before anything is reserved.
///
/// A function so the ordering §5.2 depends on is explicit (nothing here
/// mutates, so a refusal can't leak a reservation) and so a test can reach it
/// without an upgrade and assert the logging. Refusals were once silent: the
/// visitor saw an error and the log had nothing.
pub(crate) async fn check_admission(
    state: &Arc<AppState>,
    ip: Option<&str>,
    room: &str,
) -> Result<(), ChatError> {
    if let Some(ip) = ip {
        state.security_manager.check_ip(ip).await?;

        if !state.connection_pool.can_accept(ip).await {
            warn!(
                room = %room,
                address = %ip,
                limit = MAX_CONCURRENT_CONNECTIONS_PER_IP,
                "refused: per-address connection limit"
            );
            let _ = state.security_manager.record_suspicious_activity(ip).await;
            return Err(ChatError::RateLimitError(
                "Too many connections from your IP".to_string(),
            ));
        }
    }

    if !state.resource_monitor.can_accept_connection() {
        // Bound before the macro: `tracing` evaluates fields only when the
        // level is enabled, so inline they read as uncovered in a branch that
        // ran (§6.1d).
        let connections = state
            .resource_monitor
            .total_connections
            .load(Ordering::Relaxed);
        warn!(
            room = %room,
            connections,
            limit = MAX_CONCURRENT_USERS,
            "refused: server at capacity"
        );
        return Err(ChatError::ResourceLimit(
            "Server is at capacity".to_string(),
        ));
    }

    validate_input(room, MAX_ROOM_NAME_LEN)
        .map_err(|e| ChatError::InvalidMessage(e.to_string()))?;

    Ok(())
}

/// Releases the two connection reservations taken during admission, on
/// every failure after the counters went up and before the upgrade; an
/// upgraded socket releases via [`super::teardown::cleanup_user`]. Missing it
/// is a slow leak that ends in refusing every connection while idle.
async fn release_connection_slot(state: &Arc<AppState>, ip: Option<&str>) {
    if let Some(ip) = ip {
        state.connection_pool.remove_connection(ip).await;
    }
    state.resource_monitor.release_connection();
}

pub async fn ws_handler_inner(
    state: Arc<AppState>,
    ip: Option<String>,
    cookie: Option<UserCookie>,
    room: String,
    ws: WebSocketUpgrade,
    host: &str,
) -> Result<Response, ChatError> {
    debug!(room = %room, "websocket upgrade requested");

    check_admission(&state, ip.as_deref(), &room).await?;

    // ---- Reservations: everything below must release on failure ----------
    if let Some(ip) = &ip {
        state.connection_pool.add_connection(ip).await?;
    }
    state
        .resource_monitor
        .total_connections
        .fetch_add(1, Ordering::SeqCst);

    let connection_id = Uuid::new_v4().to_string();

    let admitted = admit_user(&state, &room, &connection_id, cookie.as_ref()).await;

    // admit_user already logged the real reason (capacity or the rejoin
    // throttle), so this doesn't restate it. A blanket "room full" here once
    // read as a capacity problem when a fast rejoin was the cause.
    let Some((final_user_id, final_animal_name)) = admitted else {
        release_connection_slot(&state, ip.as_deref()).await;
        return Err(ChatError::RoomFull);
    };

    let (user_id_cookie, animal_name_cookie) =
        create_user_cookies(&final_user_id, &final_animal_name, host);

    let client_ip = ip.clone();
    let mut response = ws
        .on_upgrade(move |socket| {
            handle_websocket(
                room,
                state,
                final_user_id,
                final_animal_name,
                socket,
                connection_id,
                client_ip,
            )
        })
        .into_response();

    // Both cookies, so a reload keeps the same identity.
    attach_cookies(&mut response, &[user_id_cookie, animal_name_cookie]);

    Ok(response)
}

/// Attaches `Set-Cookie` headers, skipping any that can't be encoded.
///
/// A function so a test reaches the failure arm, which production shouldn't:
/// identity is a closed set (a UUID and a roster name, §5.9), so no cookie can
/// hold an invalid header character. The arm catches that set being widened,
/// and dropping the cookie makes a new visitor, not a broken one.
pub(crate) fn attach_cookies(response: &mut Response, cookies: &[String]) {
    for cookie in cookies {
        match cookie.parse() {
            Ok(value) => {
                response.headers_mut().append("Set-Cookie", value);
            }
            Err(e) => error!(error = ?e, "failed to encode Set-Cookie header"),
        }
    }
}

/// Places the visitor in the room, reusing their cookie identity where
/// possible; `None` if the room refuses them. Holds the write lock throughout,
/// since it may create the room and can't split into read-then-write without a
/// race.
pub(crate) async fn admit_user(
    state: &Arc<AppState>,
    room: &str,
    connection_id: &str,
    cookie: Option<&UserCookie>,
) -> Option<(String, String)> {
    let mut rooms = state.rooms.write().await;
    let is_new_room = !rooms.contains_key(room);
    let room_state = rooms.entry(room.to_string()).or_insert_with(|| {
        debug!(room = %room, "creating room");
        create_room()
    });

    // `nova` only, the first time the room is created (`is_new_room`,
    // checked before `entry`; nova is never collected, so this fires once
    // per process): spawns the one order-preserving task that seals
    // broadcasts and republishes them to `state.nova_sealed_sender`
    // (`session/nova.rs::run_seal_loop`). Started here rather than at
    // process start so it works for every caller, including the test
    // suite's `start_ws_server`, which never runs `startup::run`.
    if is_new_room && room == NOVA_ROOM {
        let plaintext_receiver = room_state.sender.subscribe();
        tokio::spawn(super::nova::run_seal_loop(
            state.clone(),
            plaintext_receiver,
        ));
    }

    // Bound the history a joiner is about to receive, in one call. A
    // `preserve_messages` behind two conditions used to precede this and
    // every mutation of them survived, because both trimmed to
    // `MAX_MESSAGES_PER_ROOM`: six mutants, six spellings of dead code
    // (§0.2, §6.6d).
    room_state.trim_to_max_messages(&state.memory_tracker);

    // A cookie is a claim. `HttpOnly` doesn't stop the person sending any
    // `Cookie` header, so the id must be a server-issued UUID and the name
    // passes `claim_animal`; otherwise visitors could choose a name, make
    // it a megabyte long, or take someone else's.
    let cookie_identity = cookie
        .filter(|c| !c.animal_name.is_empty())
        .and_then(|c| Uuid::parse_str(&c.user_id).ok().map(|_| c));

    let candidate_id = cookie_identity.map(|c| c.user_id.as_str());
    let limit = crate::config::room_user_limit(room);
    if !room_state.is_user_allowed(candidate_id.unwrap_or(""), limit) {
        let connected = room_state.connected_user_count();
        warn!(
            room = %room,
            connected,
            limit,
            "refused: room is full, or this visitor is rejoining too fast"
        );
        return None;
    }

    let now = Instant::now();

    // What became of the identity, in one word, on every admission;
    // without it a returning visitor and a renamed one look the same, and
    // the second repeating is `skink left / stinks joined` (§5.9a).
    let outcome = match &cookie_identity {
        Some(c) if room_state.users.contains_key(&c.user_id) => "reclaimed",
        Some(_) => "recognised",
        None => "fresh",
    };
    debug!(room = %room, outcome, "deciding identity");

    let identity = match cookie_identity {
        // Known user reconnecting: reclaim their slot and name.
        Some(c) if room_state.users.contains_key(&c.user_id) => {
            let previous_name = room_state.users.get(&c.user_id)?.animal_name.clone();

            // Constraint #9: a disconnected user's name is free for anyone
            // (`name_taken_by_another_connected_user`), so someone may hold it
            // by the time this visitor reconnects. Restoring it blindly would
            // put two connected users under one name.
            let animal_name =
                if room_state.name_taken_by_another_connected_user(&c.user_id, &previous_name) {
                    room_state.claim_animal(None)
                } else {
                    previous_name
                };

            let user = room_state.users.get_mut(&c.user_id)?;
            user.animal_name = animal_name.clone();
            user.connection_state = ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: connection_id.to_string(),
            };
            user.last_active = now;
            user.last_message_time = now;
            (c.user_id.clone(), animal_name)
        }
        // A valid cookie this room hasn't seen. The id is the identity
        // (identity.rs) and the cookie is `Path=/`, so it must carry across
        // rooms. Minting a new id here was the bug: in a second room, or one
        // that had pruned the entry, the visitor's own earlier messages
        // stopped matching `this.myUserId` and rendered as someone else's,
        // while `claim_animal` often returned the same name, so it looked
        // like an impersonator wearing it.
        Some(c) => {
            let animal_name = room_state.claim_animal(Some(&c.animal_name));
            insert_user(room_state, &c.user_id, &animal_name, connection_id, now);
            (c.user_id.clone(), animal_name)
        }
        // No cookie at all: a genuinely new visitor gets a genuinely new id.
        None => {
            let user_id = Uuid::new_v4().to_string();
            let animal_name = room_state.claim_animal(None);
            insert_user(room_state, &user_id, &animal_name, connection_id, now);
            (user_id, animal_name)
        }
    };

    room_state.broadcast_user_count();

    let connected = room_state.connected_user_count();
    info!(
        room = %room,
        user_id = %identity.0,
        animal_name = %identity.1,
        outcome,
        connected,
        "admitted"
    );

    Some(identity)
}

/// Inserts a fresh, connected `UserData`. `pub(super)` because
/// [`crate::session::lifecycle::join_room`] needs the same construction when
/// the user's entry vanished before the upgrade callback ran.
pub(super) fn insert_user(
    room_state: &mut RoomState,
    user_id: &str,
    animal_name: &str,
    connection_id: &str,
    now: Instant,
) {
    room_state.users.insert(
        user_id.to_string(),
        UserData {
            user_id: user_id.to_string(),
            animal_name: animal_name.to_string(),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: connection_id.to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            last_reaction_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
        },
    );
}
