//! Joining the room, racing the four connection tasks, and the teardown
//! wrapper that guarantees exactly one of it ever runs.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::WebSocket;
use futures::StreamExt;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};
use uuid::Uuid;

use crate::config::{MAX_PAYLOAD_SIZE, NOVA_ROOM};
use crate::protocol::{ClientEvent, OutgoingEvent, OutgoingMessage, Reaction, SystemEvent};
use crate::room::{ConnectionState, RoomState};
use crate::state::AppState;

use super::admission::insert_user;
use super::nova::NovaSlot;
use super::teardown::cleanup_user;
use super::{
    FrameSink, Message, apply_client_event, beat_and_evict_idle, encode_event, forward_broadcasts,
    send_history, send_pings,
};

/// Takes the connection's place in the room and returns what it needs.
///
/// A function so both unhappy paths, races in the microseconds between
/// `admit_user` and the upgrade callback, are testable with a room in the
/// right state and no socket (§6.1c). `None` when the room is gone; the
/// caller's teardown still runs.
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

    // Subscribe before announcing the join, so this user sees their own
    // arrival and misses nothing in between. Nova connections subscribe
    // to the sealed channel instead: `session/nova.rs::run_seal_loop` alone
    // reads the plaintext one and republishes every frame, this
    // announcement included, sealed.
    let receiver = if room == NOVA_ROOM {
        state.nova_sealed_sender.subscribe()
    } else {
        room_state.sender.subscribe()
    };
    room_state.broadcast_user_count();
    room_state.broadcast_system_event(SystemEvent::UserJoined {
        user_id: user_id.to_string(),
        animal_name: animal_name.to_string(),
    });

    // Reactions resolved for this viewer as history is copied, so each
    // client learns which buckets it's in, never who else is.
    Some((receiver, room_state.history_for(user_id)))
}

/// Runs one connection and then releases it, however it ended.
///
/// Teardown wraps the session instead of sitting at each exit. When it was
/// each `return`'s job, the "room vanished between admission and upgrade" path
/// skipped it and held that connection's global and per-address slots for the
/// process's life: constraint #1 in `AGENTS.md`, broken one `return` at a time.
/// One teardown call can't be routed around.
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

/// The session itself: admission bookkeeping, the four raced tasks and every
/// early exit. It may `return` anywhere, since [`handle_websocket`] releases the
/// connection. The tasks are raced, not joined: a dead socket shows up in one,
/// and the first to notice ends the session.
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

    // `nova` only: this connection's handshake and ratchet state, shared
    // by the forward and receive tasks; no other room allocates it.
    let nova_slot = (room == NOVA_ROOM).then(|| Arc::new(NovaSlot::new()));

    // ---- Task 1: room broadcasts → this client ---------------------------
    let forward_task = forward_broadcasts(receiver, ws_tx.clone());

    // ---- Task 2: this client → the room ----------------------------------
    let receive_task = {
        let state = state.clone();
        let room = room.clone();
        let user_id = user_id.clone();
        let animal_name = animal_name.clone();
        let ws_tx = ws_tx.clone();
        let nova_slot = nova_slot.clone();

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

                        // A reply owed to this connection alone (`RequestRoster` and
                        // the nova handshake) goes out here, on this connection's
                        // own sink; the shared `room_state.sender` can't reach it.
                        let reply = if let Some(slot) = &nova_slot {
                            super::nova::dispatch(
                                &state,
                                &room,
                                &user_id,
                                &animal_name,
                                slot,
                                event,
                            )
                            .await
                        } else {
                            apply_client_event(&state, &room, &user_id, &animal_name, event).await
                        };

                        if let Some(reply) = reply {
                            let json = encode_event(&reply);
                            let mut tx = ws_tx.lock().await;
                            if tx.send_frame(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
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

    // `nova` only: if this connection joined the `Group`, remove it now
    // rather than leaving a dead leaf's keys live until restart: constraint
    // #1's release-on-every-exit applied to group membership. Here, not in
    // `teardown.rs::cleanup_user`, because the leaf index is
    // connection-local.
    if let Some(slot) = &nova_slot
        && let Some(leaf) = slot.leaf_index().await
    {
        super::nova::remove_member(&state, &room, leaf).await;
    }
}
