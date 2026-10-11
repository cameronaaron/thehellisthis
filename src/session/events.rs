//! Applying one parsed client event under the room write lock: rendering,
//! throttling, and the four non-`Message` events.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tracing::{debug, error, trace, warn};
use uuid::Uuid;

use crate::config::{
    DUPLICATE_MESSAGE_WINDOW, HEARTBEAT_TIMEOUT, MAX_MESSAGE_LEN, REACTION_MIN_INTERVAL,
    READ_RECEIPT_MIN_INTERVAL, TYPING_EVENT_MIN_INTERVAL,
};
use crate::emoji::is_reaction_emoji;
use crate::error::ChatError;
use crate::protocol::{ClientEvent, OutgoingEvent, OutgoingMessage, SystemEvent, encode_broadcast};
use crate::room::ConnectionState;
use crate::state::AppState;
use crate::validation::{
    render_message_html, sanitize_attachment, sanitize_reply, validate_and_render_message,
};

/// Runs CPU-bound work (Markdown, sanitising) on Tokio's blocking pool.
///
/// The runtime is `current_thread` (`main.rs`): every connection's heartbeat,
/// ping and receive loop shares one OS thread, so an inline slow render would
/// stall them all (§2.1a). Generic over `f`, like the connection tasks over
/// their sink, so the failure path (a panic, or shutdown mid-render) is a value
/// a test can produce (§6.1c).
pub(crate) async fn render_off_thread<F, T>(f: F) -> Option<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(value) => Some(value),
        Err(e) => {
            error!(error = %e, "render task did not complete");
            None
        }
    }
}

/// Folds `render_off_thread`'s "didn't complete" case into the rejection an
/// invalid message already gets. A separate function so the call site has one
/// failure arm, exercised by every rejection test, instead of a second arm only
/// a real panic reaches.
pub(crate) fn resolve_render(
    rendered: Option<Result<String, ChatError>>,
) -> Result<String, ChatError> {
    rendered.unwrap_or_else(|| {
        Err(ChatError::InvalidMessage(
            "message could not be rendered".to_string(),
        ))
    })
}

/// Whether an event at `now` comes too soon after `last` for its interval.
///
/// The shape shared by the typing, read-receipt, reaction and roster
/// throttles, as a plain function a test can call. It doesn't store `now`;
/// only the caller knows which `UserData` field to update.
pub(crate) fn within_throttle(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last.is_some_and(|last| now.duration_since(last) < interval)
}

/// Applies one parsed client event under the room write lock. Split out of
/// the receive loop, whose ten-level nesting had interleaved the duplicate and
/// heartbeat checks with rendering.
pub async fn apply_client_event(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    animal_name: &str,
    event: ClientEvent,
) -> Option<OutgoingEvent> {
    apply_client_event_at(state, room, user_id, animal_name, event, Instant::now()).await
}

/// The same, with the instant supplied.
///
/// Six throttles compare against `now`, and all survived mutation to `<=`
/// while the clock was read inside: a test couldn't land on the boundary.
/// Taking the instant is the edge injection `cleanup_rooms_at` and
/// `cleanup_stale_at` use, with no injectable clock (§9.4, §6.6f).
///
/// Returns a reply owed to the caller alone (only `RequestRoster` has one).
/// Everything else is broadcast in its own arm; the return value exists so a
/// one-connection reply can never go through the shared `room_state.sender`.
pub async fn apply_client_event_at(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    animal_name: &str,
    event: ClientEvent,
    now: Instant,
) -> Option<OutgoingEvent> {
    let mut rooms = state.rooms.write().await;

    let Some(room_state) = rooms.get_mut(room) else {
        warn!(room = %room, "event for unknown room");
        return None;
    };
    let Some(user) = room_state.users.get_mut(user_id) else {
        warn!(user_id = %user_id, room = %room, "event for unknown user");
        return None;
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
            return None;
        }
        *last_heartbeat = now;
    }

    match event {
        ClientEvent::Message {
            text,
            reply_to,
            attachment,
        } => {
            // The message budget is spent by messages only. Charging every
            // event made typing on/off cost two extra units per message, so the
            // real limit was ten a minute, not `MAX_MESSAGES_PER_WINDOW`'s
            // thirty. Typing and read receipts have their own O(1) throttles.
            if !user.rate_limiter.can_send_message() {
                debug!(user_id = %user_id, "rate limited");
                return None;
            }

            let attachment = match attachment.map(sanitize_attachment) {
                Some(Ok(attachment)) => Some(attachment),
                Some(Err(e)) => {
                    debug!(user_id = %user_id, error = %e, "attachment rejected");
                    return None;
                }
                None => None,
            };

            // Same text twice in quick succession is a double send, but only
            // when text is all there is: two images share an empty caption,
            // and two photographs are two decisions.
            if attachment.is_none()
                && let Some((last_text, last_time)) = &user.last_message_text
                && text == *last_text
                && now.duration_since(*last_time) < DUPLICATE_MESSAGE_WINDOW
            {
                debug!(user_id = %user_id, "dropping duplicate message");
                return None;
            }

            // An image alone is a message, so text is required only when
            // there's nothing else; an empty caption under a photo isn't an
            // empty message.
            let needs_text = attachment.is_none() || !text.trim().is_empty();

            // Real CPU work (comrak then ammonia over up to
            // `MAX_RENDERED_MESSAGE_LEN` chars), moved off the async thread so
            // it can't stall other connections' heartbeats. The room lock is
            // still held across the wait; this changes who else can make
            // progress, not how many rooms can change at once.
            let render_text = text.clone();
            let rendered = render_off_thread(move || {
                if needs_text {
                    validate_and_render_message(&render_text)
                } else {
                    Ok(render_message_html(&render_text))
                }
            })
            .await;

            let rendered_html = match resolve_render(rendered) {
                Ok(html) => html,
                Err(e) => {
                    debug!(user_id = %user_id, error = %e, "message rejected");
                    return None;
                }
            };

            user.last_message_text = Some((text.clone(), now));

            if text.len() > MAX_MESSAGE_LEN {
                warn!(user_id = %user_id, len = text.len(), "message too long");
                return None;
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
                text: rendered_html,
                timestamp,
                // Never store what the client sent verbatim: these fields are
                // as attacker-controlled as the message body.
                reply_to: reply_to.and_then(sanitize_reply),
                attachment,
            };

            user.last_message_time = now;
            let message_id = outgoing.message_id;
            // Encoded once here, not once per receiver: see `encode_broadcast`.
            let frame = encode_broadcast(&OutgoingEvent::Message {
                message: outgoing.clone(),
            });
            room_state.add_message(outgoing, &state.memory_tracker);
            let _ = room_state.sender.send(frame);
            trace!(%message_id, user_id = %user_id, room = %room, "message broadcast");
            None
        }

        ClientEvent::Typing { is_typing } => {
            if within_throttle(user.last_typing_event, now, TYPING_EVENT_MIN_INTERVAL) {
                trace!(user_id = %user_id, "typing event throttled");
                return None;
            }
            user.last_typing_event = Some(now);
            user.is_typing = is_typing;

            let (uid, animal) = (user.user_id.clone(), user.animal_name.clone());
            trace!(user_id = %user_id, is_typing, "typing event broadcast");
            room_state.broadcast_system_event(SystemEvent::Typing {
                user_id: uid,
                animal_name: animal,
                is_typing,
            });
            None
        }

        ClientEvent::ReadReceipt { message_id } => {
            if within_throttle(user.last_read_receipt_event, now, READ_RECEIPT_MIN_INTERVAL) {
                trace!(user_id = %user_id, "read receipt throttled");
                return None;
            }
            user.last_read_receipt_event = Some(now);

            let Ok(msg_id) = Uuid::parse_str(&message_id) else {
                debug!(user_id = %user_id, "invalid message id in read receipt");
                return None;
            };

            user.last_read_message = Some(msg_id);
            let (uid, animal) = (user.user_id.clone(), user.animal_name.clone());
            trace!(user_id = %user_id, %msg_id, "read receipt broadcast");
            room_state.broadcast_system_event(SystemEvent::ReadReceipt {
                user_id: uid,
                animal_name: animal,
                message_id: msg_id,
            });
            None
        }

        ClientEvent::RequestRoster => {
            // Answered to the asker alone. This once went through the
            // shared `room_state.sender`, contradicting this very comment, so
            // one person opening the panel sent every client a `Roster` it
            // never asked for. It is returned for the caller, which holds the
            // one per-connection sink (§5.15). Throttled on the reaction
            // clock, the same "someone clicked" cadence.
            if within_throttle(user.last_reaction_event, now, REACTION_MIN_INTERVAL) {
                trace!(user_id = %user_id, "roster request throttled");
                return None;
            }
            user.last_reaction_event = Some(now);

            let roster = room_state.roster();
            debug!(room = %room, user_id = %user_id, size = roster.len(), "roster requested");
            Some(OutgoingEvent::Roster { users: roster })
        }

        ClientEvent::React { message_id, emoji } => {
            // Validate before spending, as admission does (§5.2). The
            // throttle used to be stamped first, so a malformed event spent
            // the next one's budget and a good reaction vanished with no error:
            // the same defect as typing spending the message budget
            // (constraint #22). A refused event did no work worth rationing.

            // A closed set, checked rather than sanitised: reactions are
            // stored and rebroadcast, so an arbitrary string would be the
            // animal-name defect again (§5.9).
            if !is_reaction_emoji(&emoji) {
                debug!(user_id = %user_id, "reaction is not on the roster");
                return None;
            }

            let Ok(msg_id) = Uuid::parse_str(&message_id) else {
                debug!(user_id = %user_id, "invalid message id in reaction");
                return None;
            };

            if within_throttle(user.last_reaction_event, now, REACTION_MIN_INTERVAL) {
                trace!(user_id = %user_id, "reaction throttled");
                return None;
            }
            user.last_reaction_event = Some(now);

            let uid = user.user_id.clone();
            let (active, count) = room_state.toggle_reaction(msg_id, &emoji, &uid)?;

            room_state.broadcast_system_event(SystemEvent::Reaction {
                message_id: msg_id,
                user_id: uid,
                emoji,
                active,
                count,
            });
            None
        }

        // `nova` only. These never get here normally:
        // `session/lifecycle.rs` routes nova connections through
        // `nova::dispatch`, which only forwards the inner event of an
        // unwrapped `Sealed`, never these variants. Reaching this arm means
        // one arrived in another room, or nova's routing has a bug; either
        // way there is nothing to do under the room lock.
        ClientEvent::NovaPreKeyBundleRequest
        | ClientEvent::NovaX3dhInit { .. }
        | ClientEvent::NovaJoinRequest { .. }
        | ClientEvent::Sealed { .. }
        | ClientEvent::RlnRegister { .. }
        | ClientEvent::RlnPathRequest { .. }
        | ClientEvent::RlnMessage { .. }
        | ClientEvent::Dummy { .. }
        | ClientEvent::NovaMpcDemoRequest => {
            warn!(user_id = %user_id, room = %room, "nova transport frame reached room-event dispatch");
            None
        }
    }
}
