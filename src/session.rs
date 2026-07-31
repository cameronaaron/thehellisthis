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

pub(crate) use axum::extract::ws::Message;
use axum::extract::ws::{CloseFrame, WebSocket};
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
    DUPLICATE_MESSAGE_WINDOW, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, IDLE_CLOSE_CODE,
    MAX_CONCURRENT_CONNECTIONS_PER_IP, MAX_CONCURRENT_USERS, MAX_MESSAGE_LEN, MAX_PAYLOAD_SIZE,
    MAX_ROOM_NAME_LEN, MAX_USERS_PER_ROOM, REACTION_MIN_INTERVAL, READ_RECEIPT_MIN_INTERVAL,
    TYPING_EVENT_MIN_INTERVAL, USER_IDLE_MESSAGE_TIMEOUT,
};
use crate::emoji::is_reaction_emoji;
use crate::error::ChatError;
use crate::identity::{OptionalUserCookie, UserCookie, create_user_cookies};
use crate::limits::RateLimiter;
use crate::protocol::{
    ClientEvent, HistoryEvent, HistoryMessage, OutgoingEvent, OutgoingMessage, SystemEvent,
};
use crate::room::{ConnectionState, RoomState, UserData, create_room, user_idle_for_too_long};
use crate::security::is_allowed_origin;
use crate::state::AppState;
use crate::validation::{
    extract_client_ip, hash_client_address, render_message_html, sanitize_attachment,
    sanitize_reply, validate_input, validate_message,
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
/// [`cleanup_user`] instead. Missing this is a slow leak that ends with the
/// server refusing all connections while idle.
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

    let Some((final_user_id, final_animal_name)) = admitted else {
        warn!(room = %room, limit = MAX_USERS_PER_ROOM, "refused: room full");
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
    let room_state = rooms.entry(room.to_string()).or_insert_with(|| {
        debug!(room = %room, "creating room");
        create_room()
    });

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
    if !room_state.is_user_allowed(candidate_id.unwrap_or("")) {
        let connected = room_state.connected_user_count();
        warn!(
            room = %room,
            connected,
            limit = MAX_USERS_PER_ROOM,
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
            last_reaction_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
        },
    );
}

/// Serialises a frame the server is about to send.
///
/// Every outgoing type is a plain struct or enum of owned strings, numbers and
/// UUIDs — no map with non-string keys, no float that could be NaN — so this
/// cannot fail. Saying that in one place, once, is better than an unreachable
/// error arm at each of the four call sites, each of which a reader has to
/// work out is unreachable for themselves.
pub(crate) fn encode_event<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| {
        // Reached only if an outgoing type gains a field that cannot be
        // represented. An empty frame is dropped by the client; a panic here
        // would take the whole connection task with it (§5.1).
        error!(error = %e, "failed to serialise an outgoing frame");
        String::new()
    })
}

/// A sink this session can write frames to.
///
/// The four tasks are generic over this rather than tied to a `WebSocket`, so a
/// test can hand them a sink that fails and reach the "the client is gone"
/// arms — which otherwise need the peer to vanish between two exact frames, a
/// race no test can win reliably (§6.1c).
pub(crate) trait FrameSink: Send {
    fn send_frame(
        &mut self,
        message: Message,
    ) -> impl std::future::Future<Output = Result<(), ()>> + Send;
}

impl FrameSink for futures::stream::SplitSink<WebSocket, Message> {
    async fn send_frame(&mut self, message: Message) -> Result<(), ()> {
        SinkExt::send(self, message).await.map_err(|_| ())
    }
}

/// Room broadcasts → this client, until the socket stops accepting them.
pub(crate) async fn forward_broadcasts<S: FrameSink>(
    mut receiver: tokio::sync::broadcast::Receiver<OutgoingEvent>,
    sink: Arc<Mutex<S>>,
) {
    while let Ok(event) = receiver.recv().await {
        let json = encode_event(&event);

        let mut tx = sink.lock().await;
        if tx.send_frame(Message::Text(json.into())).await.is_err() {
            break;
        }
    }
}

/// WebSocket-level pings, until the socket stops accepting them.
pub(crate) async fn send_pings<S: FrameSink>(sink: Arc<Mutex<S>>) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut alive = true;

    // A `while` rather than `loop`/`break`: the condition *is* the thing that
    // ends this task, so saying it once reads better than a jump out of the
    // middle — and there is no unreachable exit left for a reader to wonder at.
    while alive {
        interval.tick().await;
        let mut tx = sink.lock().await;
        alive = tx.send_frame(Message::Ping(Bytes::new())).await.is_ok();
    }
}

/// The frame that removes an idle user, sent with the code the client reads.
///
/// A function so the frame itself is assertable without waiting ten minutes for
/// a real eviction: what matters is that it carries `IDLE_CLOSE_CODE`, because
/// a bare close is indistinguishable from a dropped connection and the client
/// would reconnect straight back into the room (§7).
pub(crate) fn idle_close_frame() -> Message {
    Message::Close(Some(CloseFrame {
        code: IDLE_CLOSE_CODE,
        reason: "idle".into(),
    }))
}

/// Replays the room's history to a joining client.
///
/// Returns false if the client left part-way through, which is common enough to
/// be ordinary: people open a room and close the tab. Sink-generic so that path
/// is reachable without having to make a real peer vanish mid-replay.
pub(crate) async fn send_history<S: FrameSink>(
    history: &[(Arc<OutgoingMessage>, Vec<crate::protocol::Reaction>)],
    sink: &Arc<Mutex<S>>,
    user_id: &str,
) -> bool {
    debug!(count = history.len(), user_id = %user_id, "sending history");

    for (message, reactions) in history {
        // Serialised from borrows: the history was handed over as refcounts,
        // and nothing here copies a message just to put a `type` around it.
        let json = encode_event(&HistoryEvent::Message {
            message: HistoryMessage { message, reactions },
        });

        let mut tx = sink.lock().await;
        if tx.send_frame(Message::Text(json.into())).await.is_err() {
            debug!(user_id = %user_id, "client left during history send");
            return false;
        }
    }

    true
}

/// The application heartbeat, and the eviction that lets a room empty.
///
/// Sink-generic like the others, so both ways this ends — the client stopping
/// accepting frames, and the user going quiet long enough to be removed — are
/// reachable without a real socket dying at an exact instant.
pub(crate) async fn beat_and_evict_idle<S: FrameSink>(
    state: Arc<AppState>,
    room: String,
    user_id: String,
    sink: Arc<Mutex<S>>,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut running = true;

    while running {
        interval.tick().await;

        let delivered = {
            let mut tx = sink.lock().await;
            tx.send_frame(Message::Text(
                encode_event(&OutgoingEvent::Heartbeat).into(),
            ))
            .await
            .is_ok()
        };

        if !delivered {
            running = false;
            continue;
        }

        if touch_and_check_idle(&state, &room, &user_id).await {
            let timeout_s = USER_IDLE_MESSAGE_TIMEOUT.as_secs();
            info!(user_id = %user_id, room = %room, timeout_s, "disconnecting idle user");
            let mut tx = sink.lock().await;
            let _ = tx.send_frame(idle_close_frame()).await;
            running = false;
        }
    }
}

/// Marks this user as still connected, and says whether they have gone quiet.
///
/// Extracted so the decision is a function of the room's state rather than
/// something only reachable from inside a timer loop. Returns false when the
/// room or the user has gone, which is not idleness — the session is ending
/// for another reason and the heartbeat should not claim otherwise.
pub(crate) async fn touch_and_check_idle(state: &Arc<AppState>, room: &str, user_id: &str) -> bool {
    let mut rooms = state.rooms.write().await;
    let Some(user) = rooms
        .get_mut(room)
        .and_then(|room_state| room_state.users.get_mut(user_id))
    else {
        return false;
    };

    let now = Instant::now();
    if let ConnectionState::Connected {
        ref mut last_heartbeat,
        ..
    } = user.connection_state
    {
        *last_heartbeat = now;
    }

    user_idle_for_too_long(user, now)
}

/// Takes the connection's place in the room and returns what it needs to run.
///
/// A function rather than a block inside `run_session` so both of its unhappy
/// paths can be exercised. Both are races against the microseconds between
/// `admit_user` returning and the upgrade callback running, which a test cannot
/// win reliably — but neither needs a socket to reach, only a room in the right
/// state (§6.1c: an untestable line is usually a misplaced line).
///
/// Returns `None` when the room is gone, in which case there is nothing to
/// join and the caller's teardown still runs.
pub(crate) async fn join_room(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    animal_name: &str,
    connection_id: &str,
) -> Option<(
    tokio::sync::broadcast::Receiver<OutgoingEvent>,
    Vec<(Arc<OutgoingMessage>, Vec<crate::protocol::Reaction>)>,
)> {
    let mut rooms = state.rooms.write().await;
    let room_state = rooms.get_mut(room).or_else(|| {
        error!(room = %room, "room vanished between admission and upgrade");
        None
    })?;

    if let Some(user) = room_state.users.get_mut(user_id) {
        user.connection_state = ConnectionState::Connected {
            last_heartbeat: Instant::now(),
            connection_id: connection_id.to_string(),
        };
    } else {
        warn!(user_id = %user_id, room = %room, "user missing at upgrade; recreating");
        insert_user(
            room_state,
            user_id,
            animal_name,
            connection_id,
            Instant::now(),
        );
    }

    // Subscribe *before* announcing the join, so this user sees their own
    // arrival and no broadcast between the two is missed.
    let receiver = room_state.sender.subscribe();
    room_state.broadcast_user_count();
    room_state.broadcast_system_event(SystemEvent::UserJoined {
        user_id: user_id.to_string(),
        animal_name: animal_name.to_string(),
    });

    // Reactions are resolved for *this* viewer as the history is copied, so
    // each client learns which buckets it is in without ever being told who
    // else is in them.
    Some((receiver, room_state.history_for(user_id)))
}

/// Runs one connection and then releases it, whatever ended it.
///
/// The teardown is here, wrapped around the session, rather than at each of the
/// session's exits. It used to be the caller's duty at every `return`, and one
/// of them — the "room vanished between admission and upgrade" path — did not
/// do it, so that connection's global slot and per-IP slot were held for the
/// life of the process. That is the same failure constraint #1 in `CLAUDE.md`
/// describes, reintroduced one `return` at a time. A wrapper cannot forget:
/// there is exactly one teardown call and no exit that can route around it.
pub async fn handle_websocket(
    room: String,
    state: Arc<AppState>,
    user_id: String,
    animal_name: String,
    socket: WebSocket,
    connection_id: String,
    client_ip: Option<String>,
) {
    let (ws_tx, ws_rx) = socket.split();
    run_session(
        room.clone(),
        state.clone(),
        user_id.clone(),
        animal_name,
        ws_tx,
        ws_rx,
        connection_id.clone(),
    )
    .await;

    cleanup_user(
        &state,
        &room,
        &user_id,
        &connection_id,
        client_ip.as_deref(),
    )
    .await;
}

/// The session itself: admission bookkeeping, the four raced tasks, and every
/// early exit. Releasing the connection is [`handle_websocket`]'s job, so this
/// function is free to `return` from anywhere.
///
/// The tasks are raced rather than joined: a dead socket shows up in exactly
/// one of them, and the first to notice should tear the whole session down.
pub(crate) async fn run_session<S, R>(
    room: String,
    state: Arc<AppState>,
    user_id: String,
    animal_name: String,
    ws_tx: S,
    mut ws_rx: R,
    connection_id: String,
) where
    S: FrameSink + 'static,
    R: futures::Stream<Item = Result<Message, axum::Error>> + Unpin + Send,
{
    let joined = join_room(&state, &room, &user_id, &animal_name, &connection_id).await;
    let (receiver, chat_history) = match joined {
        Some(joined) => joined,
        // The room went between admission and the upgrade. There is nothing to
        // join; the caller's teardown still runs.
        None => return,
    };

    let ws_tx = Arc::new(Mutex::new(ws_tx));

    // Identity first: the client needs to know which messages are its own
    // before it renders any history.
    {
        let mut tx = ws_tx.lock().await;
        let json = encode_event(&OutgoingEvent::Welcome {
            user_id: user_id.clone(),
            animal_name: animal_name.clone(),
        });
        let _ = tx.send_frame(Message::Text(json.into())).await;
    }

    if !send_history(&chat_history, &ws_tx, &user_id).await {
        return;
    }

    {
        let mut tx = ws_tx.lock().await;
        let json = encode_event(&OutgoingEvent::ReconnectToken {
            token: Uuid::new_v4().to_string(),
        });
        let _ = tx.send_frame(Message::Text(json.into())).await;
    }

    // ---- Task 1: room broadcasts → this client ---------------------------
    let forward_task = forward_broadcasts(receiver, ws_tx.clone());

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
                        if tx.send_frame(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Pong(_) => trace!(user_id = %user_id, "pong"),
                    Message::Binary(_) => debug!(user_id = %user_id, "ignoring binary frame"),
                    Message::Close(_) => break,
                }
            }
        }
    };

    // ---- Task 3: protocol-level keepalive --------------------------------
    let ping_task = send_pings(ws_tx.clone());

    // ---- Task 4: application heartbeat + idle eviction --------------------
    let heartbeat_task =
        beat_and_evict_idle(state.clone(), room.clone(), user_id.clone(), ws_tx.clone());

    tokio::select! {
        _ = forward_task => trace!(user_id = %user_id, "forward task ended"),
        _ = receive_task => trace!(user_id = %user_id, "receive task ended"),
        _ = ping_task => trace!(user_id = %user_id, "ping task ended"),
        _ = heartbeat_task => trace!(user_id = %user_id, "heartbeat task ended"),
    }
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
    apply_client_event_at(state, room, user_id, animal_name, event, Instant::now()).await;
}

/// The same, with the instant supplied.
///
/// Six throttles here compare against `now`, and every one of them survived
/// mutation as `<=`: the clock was read *inside* this function, so a test could
/// set a user's last event to exactly one interval ago and the reading would
/// have moved past it by the time the comparison ran. The boundary was not
/// unimportant, it was unreachable. Taking the instant is the same edge
/// injection `cleanup_rooms_at` and `cleanup_stale_at` use, and it needs no
/// injectable clock (§9.4, §6.6f).
pub async fn apply_client_event_at(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    animal_name: &str,
    event: ClientEvent,
    now: Instant,
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

    match event {
        ClientEvent::Message {
            text,
            reply_to,
            attachment,
        } => {
            // The message budget is spent by messages, and by nothing else.
            // Every event used to be charged here, but the client sends a
            // `Typing{true}` on the first keystroke and a `Typing{false}` when
            // the message goes — so each message cost three units of a
            // thirty-unit window and the real limit was ten a minute, not the
            // thirty `MAX_MESSAGES_PER_WINDOW` advertises. Typing and read
            // receipts carry their own O(1) throttles below; those are what
            // bound them.
            if !user.rate_limiter.can_send_message() {
                debug!(user_id = %user_id, "rate limited");
                return;
            }

            let attachment = match attachment.map(sanitize_attachment) {
                Some(Ok(attachment)) => Some(attachment),
                Some(Err(e)) => {
                    debug!(user_id = %user_id, error = %e, "attachment rejected");
                    return;
                }
                None => None,
            };

            // Same text twice in quick succession is a double-send, not intent —
            // but only when the text is all there is. Two images share the empty
            // caption, and picking two photographs is two decisions; treating
            // the second as a stutter would silently drop it.
            if attachment.is_none()
                && let Some((last_text, last_time)) = &user.last_message_text
                && text == *last_text
                && now.duration_since(*last_time) < DUPLICATE_MESSAGE_WINDOW
            {
                debug!(user_id = %user_id, "dropping duplicate message");
                return;
            }

            // An image on its own is a message. Text is required only when
            // there is nothing else to carry, which is why this is not simply
            // `validate_message(&text)?` — an empty caption under a photograph
            // is not an empty message.
            if (attachment.is_none() || !text.trim().is_empty())
                && let Err(e) = validate_message(&text)
            {
                debug!(user_id = %user_id, error = %e, "message rejected");
                return;
            }

            user.last_message_text = Some((text.clone(), now));

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
                attachment,
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

        ClientEvent::RequestRoster => {
            // Answered to the asker alone: the room's broadcast channel would
            // send it to everybody, and this is a panel one person opened.
            // Throttled with the reaction clock, which is the same "a person
            // clicked something" cadence.
            if let Some(last) = user.last_reaction_event
                && now.duration_since(last) < REACTION_MIN_INTERVAL
            {
                return;
            }
            user.last_reaction_event = Some(now);

            let roster = room_state.roster();
            debug!(room = %room, user_id = %user_id, size = roster.len(), "roster requested");
            let _ = room_state
                .sender
                .send(OutgoingEvent::Roster { users: roster });
        }

        ClientEvent::React { message_id, emoji } => {
            // Validate before spending anything, the same ordering the
            // admission path uses (§5.2): every check that can fail runs while
            // nothing has been mutated yet.
            //
            // The throttle used to be stamped first, so a malformed event
            // consumed the budget belonging to the *next* one — click two
            // reactions in quick succession, or let one bad frame through, and
            // a perfectly good reaction vanished with no error anywhere. That
            // is the same defect as typing indicators spending the message
            // budget (constraint #22): a budget must be spent by the thing it
            // is for, and a refused event did no work worth rationing.

            // A closed set, checked rather than sanitised: a reaction is stored
            // and rebroadcast to the room, so an arbitrary string here would be
            // the same defect as an arbitrary animal name (§5.9).
            if !is_reaction_emoji(&emoji) {
                debug!(user_id = %user_id, "reaction is not on the roster");
                return;
            }

            let Ok(msg_id) = Uuid::parse_str(&message_id) else {
                debug!(user_id = %user_id, "invalid message id in reaction");
                return;
            };

            if let Some(last) = user.last_reaction_event
                && now.duration_since(last) < REACTION_MIN_INTERVAL
            {
                return;
            }
            user.last_reaction_event = Some(now);

            let uid = user.user_id.clone();
            let Some((active, count)) = room_state.toggle_reaction(msg_id, &emoji, &uid) else {
                return;
            };

            room_state.broadcast_system_event(SystemEvent::Reaction {
                message_id: msg_id,
                user_id: uid,
                emoji,
                active,
                count,
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

    state.resource_monitor.release_connection();

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

    let (uid, animal) = (user.user_id.clone(), user.animal_name.clone());
    let _ = room_state.sender.send(OutgoingEvent::System {
        event: SystemEvent::UserLeft {
            user_id: uid.clone(),
            animal_name: animal.clone(),
        },
    });

    user.connection_state = ConnectionState::Disconnected {
        since: Instant::now(),
    };
    room_state.broadcast_user_count();

    let remaining = room_state.connected_user_count();
    info!(
        room = %room,
        user_id = %uid,
        animal_name = %animal,
        remaining,
        "departed"
    );
}
