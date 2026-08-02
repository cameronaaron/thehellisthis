//! Joining the room, racing the four connection tasks, and the teardown
//! wrapper that guarantees exactly one of it ever runs.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::WebSocket;
use futures::StreamExt;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};
use uuid::Uuid;

use crate::config::MAX_PAYLOAD_SIZE;
use crate::protocol::{ClientEvent, OutgoingEvent, OutgoingMessage, Reaction, SystemEvent};
use crate::room::{ConnectionState, RoomState};
use crate::state::AppState;

use super::admission::insert_user;
use super::teardown::cleanup_user;
use super::{
    FrameSink, Message, apply_client_event, beat_and_evict_idle, encode_event, forward_broadcasts,
    send_history, send_pings,
};

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
    tokio::sync::broadcast::Receiver<Arc<str>>,
    Vec<(Arc<OutgoingMessage>, Vec<Reaction>)>,
)> {
    let mut rooms = state.rooms.write().await;
    let room_state: &mut RoomState = rooms.get_mut(room).or_else(|| {
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
    let heartbeat_task = beat_and_evict_idle(
        state.clone(),
        room.clone(),
        user_id.clone(),
        connection_id.clone(),
        ws_tx.clone(),
    );

    tokio::select! {
        _ = forward_task => trace!(user_id = %user_id, "forward task ended"),
        _ = receive_task => trace!(user_id = %user_id, "receive task ended"),
        _ = ping_task => trace!(user_id = %user_id, "ping task ended"),
        _ = heartbeat_task => trace!(user_id = %user_id, "heartbeat task ended"),
    }
}
