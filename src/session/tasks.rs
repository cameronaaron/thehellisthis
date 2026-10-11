//! The four things raced for the life of a connection — room broadcasts in,
//! protocol pings, history replay, and the application heartbeat — plus the
//! frame encoding they all share.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{CloseFrame, WebSocket};
use bytes::Bytes;
use futures::SinkExt;
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::config::{
    HEARTBEAT_INTERVAL, IDLE_CLOSE_CODE, SUPERSEDED_CLOSE_CODE, USER_IDLE_MESSAGE_TIMEOUT,
};
use crate::protocol::{
    HistoryEvent, HistoryMessage, OutgoingEvent, OutgoingMessage, Reaction, encode_event,
};
use crate::room::{ConnectionState, user_idle_for_too_long};
use crate::state::AppState;

use super::Message;

/// A sink this session can write frames to. The four tasks are generic over
/// it so a test can supply one that fails and reach the "client is gone" arms,
/// which otherwise need the peer to vanish between two exact frames (§6.1c).
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

/// Room broadcasts to this client, until the socket stops accepting them.
///
/// `recv` clones the `Arc<str>`, a refcount bump, so `encode_broadcast`'s work
/// is shared by every connection. Nova needs nothing special here: its
/// connections receive from `state.nova_sealed_sender`
/// (`session/lifecycle.rs::join_room`), so whatever arrives is already what to
/// relay, sealed once by `session/nova.rs::run_seal_loop`.
pub(crate) async fn forward_broadcasts<S: FrameSink>(
    mut receiver: tokio::sync::broadcast::Receiver<Arc<str>>,
    sink: Arc<Mutex<S>>,
) {
    while let Ok(frame) = receiver.recv().await {
        let mut tx = sink.lock().await;
        if tx
            .send_frame(Message::Text(frame.as_ref().into()))
            .await
            .is_err()
        {
            break;
        }
    }
}

/// WebSocket-level pings, until the socket stops accepting them.
pub(crate) async fn send_pings<S: FrameSink>(sink: Arc<Mutex<S>>) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut alive = true;

    // `while` rather than `loop`/`break`: the condition is what ends
    // this task, so it's stated once.
    while alive {
        interval.tick().await;
        let mut tx = sink.lock().await;
        alive = tx.send_frame(Message::Ping(Bytes::new())).await.is_ok();
    }
}

/// The frame that removes an idle user. A function so it's assertable without
/// waiting ten minutes; it must carry `IDLE_CLOSE_CODE`, since a bare close
/// looks like a drop and the client would reconnect straight back (§7).
pub(crate) fn idle_close_frame() -> Message {
    Message::Close(Some(CloseFrame {
        code: IDLE_CLOSE_CODE,
        reason: "idle".into(),
    }))
}

/// Replays history to a joining client. `false` if the client left partway,
/// which is ordinary (people close tabs); sink-generic so that path is
/// reachable without a real peer vanishing.
pub(crate) async fn send_history<S: FrameSink>(
    history: &[(Arc<OutgoingMessage>, Vec<Reaction>)],
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

/// The application heartbeat, the eviction that lets a room empty, and the
/// close that ends a superseded connection. Sink-generic so all three endings
/// (the client stops accepting frames, the user goes quiet, another connection
/// takes the identity) are testable without real timing races.
pub(crate) async fn beat_and_evict_idle<S: FrameSink>(
    state: Arc<AppState>,
    room: String,
    user_id: String,
    connection_id: String,
    sink: Arc<Mutex<S>>,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut running = true;

    while running {
        interval.tick().await;

        // Checked first: a superseded socket has nothing to be idle about,
        // and touching its heartbeat would only delay noticing. Which
        // connection speaks for a user is a claim to check, like a cookie
        // (§5.9).
        if is_superseded(&state, &room, &user_id, &connection_id).await {
            info!(user_id = %user_id, room = %room, "disconnecting: superseded by a newer connection");
            let mut tx = sink.lock().await;
            let _ = tx.send_frame(superseded_close_frame()).await;
            running = false;
            continue;
        }

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

/// Marks the user as still connected and says whether they've gone quiet. A
/// function of room state rather than of the timer loop; `false` if the room
/// or user is gone, which is a session ending for another reason, not
/// idleness.
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

/// Whether a newer connection under this identity has taken this user's slot,
/// usually a second tab with the same cookie.
///
/// `admit_user` rightly lets that connection reclaim the identity by moving
/// `connection_id` to it, but doesn't tell the old one, whose tasks would keep
/// forwarding broadcasts, answering pings and refreshing its idle timer while
/// `connected_user_count` no longer counts it. A `Disconnected` user isn't
/// superseded; this only catches someone else being current.
pub(crate) async fn is_superseded(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    connection_id: &str,
) -> bool {
    let rooms = state.rooms.read().await;
    let Some(user) = rooms
        .get(room)
        .and_then(|room_state| room_state.users.get(user_id))
    else {
        return false;
    };

    matches!(
        &user.connection_state,
        ConnectionState::Connected { connection_id: current, .. } if current != connection_id
    )
}

/// The frame that closes a superseded connection, carrying
/// [`SUPERSEDED_CLOSE_CODE`]; a function, like [`idle_close_frame`], so it's
/// assertable without racing two connections onto one identity.
pub(crate) fn superseded_close_frame() -> Message {
    Message::Close(Some(CloseFrame {
        code: SUPERSEDED_CLOSE_CODE,
        reason: "superseded".into(),
    }))
}
