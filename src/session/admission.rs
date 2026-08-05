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
    // WebSocket upgrades are not subject to the same-origin policy: any page on
    // the internet can open a socket here, and the browser reports where it
    // came from but leaves the decision to us. Rejecting foreign origins is
    // what stops a third-party page driving a visitor's browser into these
    // rooms and spending the connection budget.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());

    if !is_allowed_origin(origin, host) {
        warn!(?origin, "rejected websocket upgrade from foreign origin");
        return ChatError::SecurityError("Origin not allowed".to_string()).into_response();
    }

    // Hashed at the boundary: everything downstream compares addresses for
    // equality only, so the real address never needs to exist past this line.
    // It is never stored in a map, never logged, and absent from a memory dump.
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
/// A function rather than a preamble inside the handler for two reasons. It is
/// the ordering §5.2 is about — nothing here mutates, so a refusal cannot leave
/// a reservation behind — and it is reachable from a test without a live
/// upgrade, which is what lets the *logging* be asserted. A refusal used to be
/// entirely silent: the visitor saw an error and the server recorded nothing,
/// so "why can nobody connect" had no answer in the log.
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
        // level is enabled, so as arguments these do not run under a test with
        // no subscriber and read as uncovered inside a branch that definitely
        // took (§6.1d).
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

/// Releases the two connection reservations taken during admission.
///
/// Called on every path that fails *after* the counters were incremented but
/// *before* the socket was upgraded — an upgraded socket releases via
/// [`crate::session::cleanup_user`] instead. Missing this is a slow leak that
/// ends with the server refusing all connections while idle.
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

    // admit_user already logged the actual reason — capacity or the rejoin
    // throttle, which it is careful to distinguish — so this layer does not
    // restate it. It used to log its own "room full" here regardless of which
    // one actually happened, which read as a capacity problem even when the
    // room had one user in it and a fast rejoin was the real cause.
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

/// Attaches `Set-Cookie` headers, skipping any that cannot be encoded.
///
/// A function rather than a loop inside the handler so the failure arm is
/// reachable from a test. It should not be reachable in production: since
/// identity became a closed set — a UUID and a roster name (§5.9) — there is no
/// longer a way for one of these to contain a character invalid in a header
/// value. It stays as the thing that catches that closed set being widened,
/// and dropping the cookie is the right response, because a visitor with no
/// cookie is a new visitor rather than a broken one.
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

/// Places the connecting visitor in the room, reusing their cookie identity
/// where possible. Returns `None` if the room refuses them.
///
/// Holds the room write lock for the whole call — it creates the room if
/// needed, so it cannot be split into read-then-write without a race.
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

    // `nova` only, and only the first time the room is ever created
    // (`is_new_room`, checked before the `entry` call above so this can
    // never fire twice — `nova` is never garbage-collected, so "first
    // created" and "created at all, ever, this process" are the same
    // event): spawns the single, order-preserving task that seals every
    // broadcast once and republishes it to `state.nova_sealed_sender`,
    // the channel every `nova` connection actually subscribes to
    // (`session/lifecycle.rs::join_room`). Triggered from here rather
    // than at process startup so it works identically for every caller —
    // including the lighter-weight router the test suite's own
    // `start_ws_server` builds, which never calls `startup::run` at all.
    // See `session/nova.rs::run_seal_loop`'s doc for why it must be the
    // only caller of `Group::seal` for broadcast content.
    if is_new_room && room == NOVA_ROOM {
        let plaintext_receiver = room_state.sender.subscribe();
        tokio::spawn(super::nova::run_seal_loop(
            state.clone(),
            plaintext_receiver,
        ));
    }

    // Bound the history a joining user is about to be sent.
    //
    // One call, not three. There used to be a `preserve_messages` behind two
    // conditions and then this, and every mutation of those conditions survived
    // — `||` for `&&`, `*` for `+`, four comparisons — because the block could
    // not change anything: `preserve_messages` trims to `MAX_MESSAGES_PER_ROOM`
    // and so does this, so whatever the first pass did the second reached the
    // same length. Six unkillable mutants were six spellings of dead code
    // (§0.2, §6.6d), and `trim_to_max_messages` makes its own comparison.
    room_state.trim_to_max_messages(&state.memory_tracker);

    // A cookie is a *claim*, not a fact. `HttpOnly` keeps a page's script away
    // from it; it does nothing about the person driving the browser, who can
    // send whatever `Cookie` header they like. So the id must be a server-issued
    // UUID and the name is checked against the roster by `claim_animal` —
    // otherwise a visitor could pick their own display name, make it a megabyte
    // long, or take one somebody in the room is already using.
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

    // What happened to this visitor's identity, in one word, on every
    // admission. Without it the difference between "came back as themselves"
    // and "was given a new name" is invisible, and the second one repeating is
    // what a room experiences as `skink left / stinks joined` (§5.9a).
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

            // Constraint #9: a disconnected user's name is free for anyone to
            // draw (`name_taken_by_another_connected_user`'s doc comment), so
            // somebody else may hold it by the time this visitor reconnects.
            // Reinstating it unconditionally would put two connected users
            // under the same name — the exact bug the roster check exists to
            // prevent, reintroduced through the one path that skipped it.
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
        // A valid identity cookie this room has not seen yet: the id is the
        // identity (identity.rs), and the cookie is sent with `Path=/` to
        // every room, so it must carry across rooms, not just reconnects
        // within one. Minting a new id here used to be the whole bug: a
        // browser that was still "the same animal" by every cookie it held
        // would get a fresh, unrelated id the moment it joined a second room
        // or came back to one whose entry had already been pruned, so its own
        // older messages in that room's history stopped matching
        // `this.myUserId` client-side and rendered as somebody else's — while
        // `claim_animal` below often handed back the same display name,
        // making it look like an impersonator wearing your name rather than
        // what it was: you, under a new id nothing else agreed to use.
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

/// Inserts a fresh, connected `UserData` into a room.
///
/// `pub(super)` rather than private: [`crate::session::lifecycle::join_room`]
/// needs the exact same construction on the (rare) path where a room's user
/// entry has vanished by the time the socket upgrade callback runs.
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
