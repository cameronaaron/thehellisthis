//! The WebSocket session: admission, the four per-connection tasks, and
//! teardown.
//!
//! Admission ordering matters more than it looks. Every check that can fail
//! runs *before* any counter is incremented, so a rejected connection cannot
//! leave a reservation behind. The one admission decision that must happen
//! under the room write lock releases its slot explicitly on the way out.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{ConnectInfo, Path, State, WebSocketUpgrade},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use http::{HeaderMap, header};
use std::time::SystemTime;
use tokio::sync::Mutex;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

use crate::config::{
    DUPLICATE_MESSAGE_WINDOW, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, MAX_MESSAGE_LEN,
    MAX_MESSAGES_PER_ROOM, MAX_PAYLOAD_SIZE, MAX_ROOM_NAME_LEN, READ_RECEIPT_MIN_INTERVAL,
    TYPING_EVENT_MIN_INTERVAL, USER_IDLE_MESSAGE_TIMEOUT,
};
use crate::error::ChatError;
use crate::identity::{OptionalUserCookie, UserCookie, create_user_cookies};
use crate::limits::RateLimiter;
use crate::protocol::{ClientEvent, OutgoingEvent, OutgoingMessage, SystemEvent};
use crate::room::{ConnectionState, RoomState, UserData, create_room, user_idle_for_too_long};
use crate::security::is_allowed_origin;
use crate::state::AppState;
use crate::validation::{
    extract_client_ip, render_message_html, sanitize_reply, validate_input, validate_message,
};

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

    let ip = extract_client_ip(&headers, Some(&conn_info));

    match ws_handler_inner(state, ip, cookie, room, ws).await {
        Ok(response) => response.into_response(),
        Err(e) => e.into_response(),
    }
}

/// Releases the two connection reservations taken during admission.
///
/// Called on every path that fails *after* the counters were incremented but
/// *before* the socket was upgraded — an upgraded socket releases via
/// [`cleanup_user`] instead. Missing this is a slow leak that ends with the
/// server refusing all connections while idle.
async fn release_connection_slot(state: &Arc<AppState>, ip: Option<&str>) {
    if let Some(ip) = ip {
        state.connection_pool.remove_connection(ip).await;
    }
    state
        .resource_monitor
        .total_connections
        .fetch_sub(1, Ordering::SeqCst);
}

pub async fn ws_handler_inner(
    state: Arc<AppState>,
    ip: Option<String>,
    cookie: Option<UserCookie>,
    room: String,
    ws: WebSocketUpgrade,
) -> Result<Response, ChatError> {
    debug!(room = %room, ip = ?ip, "websocket upgrade requested");

    // ---- Checks that mutate nothing --------------------------------------
    if let Some(ip) = &ip {
        state.security_manager.check_ip(ip).await?;

        if !state.connection_pool.can_accept(ip).await {
            let _ = state.security_manager.record_suspicious_activity(ip).await;
            return Err(ChatError::RateLimitError(
                "Too many connections from your IP".to_string(),
            ));
        }
    }

    if !state.resource_monitor.can_accept_connection() {
        return Err(ChatError::ResourceLimit(
            "Server is at capacity".to_string(),
        ));
    }

    validate_input(&room, MAX_ROOM_NAME_LEN)
        .map_err(|e| ChatError::InvalidMessage(e.to_string()))?;

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

    let Some((final_user_id, final_animal_name)) = admitted else {
        release_connection_slot(&state, ip.as_deref()).await;
        return Err(ChatError::RoomFull);
    };

    let (user_id_cookie, animal_name_cookie) =
        create_user_cookies(&final_user_id, &final_animal_name);

    info!(
        user_id = %final_user_id,
        animal_name = %final_animal_name,
        room = %room,
        "session admitted"
    );

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
    for cookie in [user_id_cookie, animal_name_cookie] {
        match cookie.parse() {
            Ok(value) => {
                response.headers_mut().append("Set-Cookie", value);
            }
            Err(e) => error!(error = ?e, "failed to encode Set-Cookie header"),
        }
    }

    Ok(response)
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
    let room_state = rooms.entry(room.to_string()).or_insert_with(|| {
        debug!(room = %room, "creating room");
        create_room()
    });

    // Bound the history a joining user is about to be sent.
    if room_state.chat_history.is_empty()
        || room_state.chat_history.len() > MAX_MESSAGES_PER_ROOM * 2
    {
        room_state.preserve_messages(&state.memory_tracker);
    }
    if room_state.chat_history.len() > MAX_MESSAGES_PER_ROOM {
        room_state.trim_to_max_messages(&state.memory_tracker);
    }

    let cookie_identity = cookie
        .filter(|c| !c.user_id.is_empty() && !c.animal_name.is_empty())
        .map(|c| (c.user_id.clone(), c.animal_name.clone()));

    let candidate_id = cookie_identity.as_ref().map(|(id, _)| id.as_str());
    if !room_state.is_user_allowed(candidate_id.unwrap_or("")) {
        return None;
    }

    let now = Instant::now();

    let identity = match cookie_identity {
        // Known user reconnecting: reclaim their slot and name.
        Some((user_id, _)) if room_state.users.contains_key(&user_id) => {
            let user = room_state.users.get_mut(&user_id)?;
            user.connection_state = ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: connection_id.to_string(),
            };
            user.last_active = now;
            user.last_message_time = now;
            (user_id, user.animal_name.clone())
        }
        // Cookie from a room they were never in: new slot, name they know.
        Some((_, animal_name)) => {
            let user_id = Uuid::new_v4().to_string();
            insert_user(room_state, &user_id, &animal_name, connection_id, now);
            (user_id, animal_name)
        }
        // Brand-new visitor.
        None => {
            let user_id = Uuid::new_v4().to_string();
            let animal_name = room_state.assign_animal();
            insert_user(room_state, &user_id, &animal_name, connection_id, now);
            (user_id, animal_name)
        }
    };

    room_state.broadcast_user_count();
    Some(identity)
}

fn insert_user(
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
            rate_limiter: RateLimiter::new(),
            last_sanitized_message: None,
        },
    );
}

/// Runs one connection until any of its four tasks ends.
///
/// The tasks are raced rather than joined: a dead socket shows up in exactly
/// one of them, and the first to notice should tear the whole session down.
pub async fn handle_websocket(
    room: String,
    state: Arc<AppState>,
    user_id: String,
    animal_name: String,
    socket: WebSocket,
    connection_id: String,
    client_ip: Option<String>,
) {
    let (ws_tx, mut ws_rx) = socket.split();

    let (mut receiver, chat_history) = {
        let mut rooms = state.rooms.write().await;
        let Some(room_state) = rooms.get_mut(&room) else {
            error!(room = %room, "room vanished between admission and upgrade");
            return;
        };

        if let Some(user) = room_state.users.get_mut(&user_id) {
            user.connection_state = ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: connection_id.clone(),
            };
        } else {
            warn!(user_id = %user_id, room = %room, "user missing at upgrade; recreating");
            insert_user(
                room_state,
                &user_id,
                &animal_name,
                &connection_id,
                Instant::now(),
            );
        }

        // Subscribe *before* announcing the join, so this user sees their own
        // arrival and no broadcast between the two is missed.
        let receiver = room_state.sender.subscribe();
        room_state.broadcast_user_count();
        room_state.broadcast_system_event(SystemEvent::UserJoined {
            user_id: user_id.clone(),
            animal_name: animal_name.clone(),
        });

        (receiver, room_state.chat_history.clone())
    };

    let ws_tx = Arc::new(Mutex::new(ws_tx));

    // Identity first: the client needs to know which messages are its own
    // before it renders any history.
    {
        let mut tx = ws_tx.lock().await;
        if let Ok(json) = serde_json::to_string(&OutgoingEvent::Welcome {
            user_id: user_id.clone(),
            animal_name: animal_name.clone(),
        }) {
            let _ = tx.send(Message::Text(json.into())).await;
        }
    }

    debug!(count = chat_history.len(), user_id = %user_id, "sending history");
    for message in &chat_history {
        let Ok(json) = serde_json::to_string(&OutgoingEvent::Message {
            message: message.clone(),
        }) else {
            continue;
        };

        let mut tx = ws_tx.lock().await;
        if tx.send(Message::Text(json.into())).await.is_err() {
            debug!(user_id = %user_id, "client left during history send");
            drop(tx);
            cleanup_user(
                &state,
                &room,
                &user_id,
                &connection_id,
                client_ip.as_deref(),
            )
            .await;
            return;
        }
    }

    {
        let mut tx = ws_tx.lock().await;
        if let Ok(json) = serde_json::to_string(&OutgoingEvent::ReconnectToken {
            token: Uuid::new_v4().to_string(),
        }) {
            let _ = tx.send(Message::Text(json.into())).await;
        }
    }

    // ---- Task 1: room broadcasts → this client ---------------------------
    let forward_task = {
        let ws_tx = ws_tx.clone();
        async move {
            while let Ok(event) = receiver.recv().await {
                let Ok(json) = serde_json::to_string(&event) else {
                    error!("failed to serialise outgoing event");
                    continue;
                };

                let mut tx = ws_tx.lock().await;
                if tx.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    };

    // ---- Task 2: this client → the room ----------------------------------
    let receive_task = {
        let state = state.clone();
        let room = room.clone();
        let user_id = user_id.clone();
        let animal_name = animal_name.clone();
        let ws_tx = ws_tx.clone();

        async move {
            while let Some(msg_result) = ws_rx.next().await {
                let Ok(msg) = msg_result else {
                    debug!(user_id = %user_id, "socket read error");
                    break;
                };

                match msg {
                    Message::Text(text) => {
                        if text.len() > MAX_PAYLOAD_SIZE {
                            warn!(user_id = %user_id, len = text.len(), "oversized payload");
                            continue;
                        }

                        let Ok(event) = serde_json::from_str::<ClientEvent>(text.as_str()) else {
                            debug!(user_id = %user_id, "unparseable client event");
                            continue;
                        };

                        apply_client_event(&state, &room, &user_id, &animal_name, event).await;
                    }
                    Message::Ping(payload) => {
                        let mut tx = ws_tx.lock().await;
                        if tx.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Pong(_) => trace!(user_id = %user_id, "pong"),
                    Message::Binary(_) => {
                        debug!(user_id = %user_id, "ignoring binary frame")
                    }
                    Message::Close(_) => break,
                }
            }
        }
    };

    // ---- Task 3: protocol-level keepalive --------------------------------
    let ping_task = {
        let ws_tx = ws_tx.clone();
        async move {
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;
                let mut tx = ws_tx.lock().await;
                if tx.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    };

    // ---- Task 4: application heartbeat + idle eviction --------------------
    let heartbeat_task = {
        let state = state.clone();
        let room = room.clone();
        let user_id = user_id.clone();
        let ws_tx = ws_tx.clone();

        async move {
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;

                {
                    let mut tx = ws_tx.lock().await;
                    let Ok(beat) = serde_json::to_string(&OutgoingEvent::Heartbeat) else {
                        break;
                    };
                    if tx.send(Message::Text(beat.into())).await.is_err() {
                        break;
                    }
                }

                let idle_too_long = {
                    let mut rooms = state.rooms.write().await;
                    rooms
                        .get_mut(&room)
                        .and_then(|rs| rs.users.get_mut(&user_id))
                        .is_some_and(|user| {
                            let now = Instant::now();
                            if let ConnectionState::Connected {
                                ref mut last_heartbeat,
                                ..
                            } = user.connection_state
                            {
                                *last_heartbeat = now;
                            }
                            user_idle_for_too_long(user, now)
                        })
                };

                if idle_too_long {
                    info!(
                        user_id = %user_id,
                        room = %room,
                        timeout_s = USER_IDLE_MESSAGE_TIMEOUT.as_secs(),
                        "disconnecting idle user"
                    );
                    let mut tx = ws_tx.lock().await;
                    let _ = tx.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = forward_task => trace!(user_id = %user_id, "forward task ended"),
        _ = receive_task => trace!(user_id = %user_id, "receive task ended"),
        _ = ping_task => trace!(user_id = %user_id, "ping task ended"),
        _ = heartbeat_task => trace!(user_id = %user_id, "heartbeat task ended"),
    }

    cleanup_user(
        &state,
        &room,
        &user_id,
        &connection_id,
        client_ip.as_deref(),
    )
    .await;
}

/// Applies one parsed client event under the room write lock.
///
/// Split out of the receive loop because the inline version nested ten levels
/// deep, which is how the duplicate-message and heartbeat checks came to be
/// interleaved with message rendering.
pub async fn apply_client_event(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    animal_name: &str,
    event: ClientEvent,
) {
    let mut rooms = state.rooms.write().await;

    let Some(room_state) = rooms.get_mut(room) else {
        warn!(room = %room, "event for unknown room");
        return;
    };
    let Some(user) = room_state.users.get_mut(user_id) else {
        warn!(user_id = %user_id, room = %room, "event for unknown user");
        return;
    };

    let now = Instant::now();
    user.last_active = now;

    // A connection whose heartbeat has already lapsed is treated as gone; its
    // messages are dropped rather than raced against teardown.
    if let ConnectionState::Connected {
        ref mut last_heartbeat,
        ..
    } = user.connection_state
    {
        if now.duration_since(*last_heartbeat) > HEARTBEAT_TIMEOUT {
            warn!(user_id = %user_id, "heartbeat lapsed; ignoring event");
            return;
        }
        *last_heartbeat = now;
    }

    if !user.rate_limiter.can_send_message() {
        debug!(user_id = %user_id, "rate limited");
        return;
    }

    match event {
        ClientEvent::Message { text, reply_to } => {
            // Same text twice in quick succession is a double-send, not intent.
            if let Some((last_text, last_time)) = &user.last_sanitized_message
                && text == *last_text
                && now.duration_since(*last_time) < DUPLICATE_MESSAGE_WINDOW
            {
                debug!(user_id = %user_id, "dropping duplicate message");
                return;
            }

            if let Err(e) = validate_message(&text) {
                debug!(user_id = %user_id, error = %e, "message rejected");
                return;
            }

            user.last_sanitized_message = Some((text.clone(), now));

            if text.len() > MAX_MESSAGE_LEN {
                warn!(user_id = %user_id, len = text.len(), "message too long");
                return;
            }

            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .to_string();

            let outgoing = OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: user_id.to_string(),
                animal_name: animal_name.to_string(),
                text: render_message_html(&text),
                timestamp,
                // Never store what the client sent verbatim: these fields are
                // as attacker-controlled as the message body.
                reply_to: reply_to.and_then(sanitize_reply),
            };

            user.last_message_time = now;
            room_state.add_message(outgoing.clone(), &state.memory_tracker);
            let _ = room_state
                .sender
                .send(OutgoingEvent::Message { message: outgoing });
        }

        ClientEvent::Typing { is_typing } => {
            if let Some(last) = user.last_typing_event
                && now.duration_since(last) < TYPING_EVENT_MIN_INTERVAL
            {
                return;
            }
            user.last_typing_event = Some(now);
            user.is_typing = is_typing;

            let (uid, animal) = (user.user_id.clone(), user.animal_name.clone());
            room_state.broadcast_system_event(SystemEvent::Typing {
                user_id: uid,
                animal_name: animal,
                is_typing,
            });
        }

        ClientEvent::ReadReceipt { message_id } => {
            if let Some(last) = user.last_read_receipt_event
                && now.duration_since(last) < READ_RECEIPT_MIN_INTERVAL
            {
                return;
            }
            user.last_read_receipt_event = Some(now);

            let Ok(msg_id) = Uuid::parse_str(&message_id) else {
                debug!(user_id = %user_id, "invalid message id in read receipt");
                return;
            };

            user.last_read_message = Some(msg_id);
            let (uid, animal) = (user.user_id.clone(), user.animal_name.clone());
            room_state.broadcast_system_event(SystemEvent::ReadReceipt {
                user_id: uid,
                animal_name: animal,
                message_id: msg_id,
            });
        }
    }
}

/// Releases a session's resources and announces the departure.
///
/// Guarded on `connection_id`: a user who reconnected before this teardown ran
/// already has a *newer* live connection, and marking them disconnected here
/// would evict the session that is currently working.
pub async fn cleanup_user(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    connection_id: &str,
    ip: Option<&str>,
) {
    if let Some(ip) = ip {
        state.connection_pool.remove_connection(ip).await;
    }

    state
        .resource_monitor
        .total_connections
        .fetch_sub(1, Ordering::SeqCst);

    let mut rooms = state.rooms.write().await;
    let Some(room_state) = rooms.get_mut(room) else {
        return;
    };
    let Some(user) = room_state.users.get_mut(user_id) else {
        return;
    };

    let ConnectionState::Connected {
        connection_id: current_id,
        ..
    } = &user.connection_state
    else {
        return;
    };

    if current_id != connection_id {
        debug!(
            stale = %connection_id,
            current = %current_id,
            "ignoring teardown for a superseded connection"
        );
        return;
    }

    let _ = room_state.sender.send(OutgoingEvent::System {
        event: SystemEvent::UserLeft {
            user_id: user.user_id.clone(),
            animal_name: user.animal_name.clone(),
        },
    });

    user.connection_state = ConnectionState::Disconnected {
        since: Instant::now(),
    };
    room_state.broadcast_user_count();
}
