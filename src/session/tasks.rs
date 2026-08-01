//! The four things raced for the life of a connection — room broadcasts in,
//! protocol pings, history replay, and the application heartbeat — plus the
//! frame encoding they all share.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{CloseFrame, WebSocket};
use bytes::Bytes;
use futures::SinkExt;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::config::{HEARTBEAT_INTERVAL, IDLE_CLOSE_CODE, USER_IDLE_MESSAGE_TIMEOUT};
use crate::protocol::{HistoryEvent, HistoryMessage, OutgoingEvent, OutgoingMessage, Reaction};
use crate::room::{ConnectionState, user_idle_for_too_long};
use crate::state::AppState;

use super::Message;

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
