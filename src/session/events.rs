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
    render_message_html, sanitize_attachment, sanitize_reply, validate_message,
};

/// Runs CPU-bound work — Markdown parsing, HTML sanitisation — on Tokio's
/// blocking pool instead of inline.
///
/// This server runs a single-threaded (`current_thread`) Tokio runtime
/// (`main.rs`), where nothing else gets a turn until a synchronous call
/// returns: every other connection's heartbeat, ping and receive loop shares
/// this one OS thread. A slow render run inline would stall all of them, not
/// just the message it belongs to.
///
/// Generic over `f` rather than tied to a specific render call, the same
/// reason the four connection tasks are generic over their sink: it makes
/// the failure path — the task panicking, or the runtime shutting down
/// mid-render — a value a test can produce directly, instead of an
/// "untestable" line waiting to be found testable after all (§6.1c).
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

/// Folds `render_off_thread`'s "the task did not complete" case into the same
/// rejection a message already takes for failing validation.
///
/// A pure, synchronous function rather than inlined at the one call site: the
/// alternative left the call site with two failure arms, one of them reachable
/// only by an actual panic mid-render, which is exactly the shape of
/// "untestable line" this module's `render_off_thread` doc comment already
/// argues against creating. This way there is one arm, exercised by every
/// existing test that rejects a message, and this function's own `None` case
/// is trivial to construct directly without needing one.
pub(crate) fn resolve_render(
    rendered: Option<Result<String, ChatError>>,
) -> Result<String, ChatError> {
    rendered.unwrap_or_else(|| {
        Err(ChatError::InvalidMessage(
            "message could not be rendered".to_string(),
        ))
    })
}

/// Whether an event arriving at `now` is too soon after `last`, given the
/// event type's own minimum interval.
///
/// The shared shape behind four per-user throttles below (typing, read
/// receipts, reactions, roster requests) — each used to repeat this exact
/// `if let Some(last) = ... && now.duration_since(last) < INTERVAL` check
/// inline. Pulled out once, it is a plain function of three values a test can
/// hand it directly, instead of the same three-line pattern reappearing a
/// fifth time the next event type needs a throttle.
///
/// Does not update the stored timestamp itself: only the caller knows which
/// field on `UserData` to write `now` into once the event is allowed through.
pub(crate) fn within_throttle(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last.is_some_and(|last| now.duration_since(last) < interval)
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
) -> Option<OutgoingEvent> {
    apply_client_event_at(state, room, user_id, animal_name, event, Instant::now()).await
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
///
/// Returns a reply owed to the caller alone, not the room — only
/// `RequestRoster` has one. Everything else this function does is broadcast
/// from inside the match arm that does it; this return value exists
/// specifically so a reply meant for one connection is never reachable
/// through `room_state.sender`, which every connection shares.
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
                return None;
            }

            // An image on its own is a message. Text is required only when
            // there is nothing else to carry, which is why this is not simply
            // `validate_message(&text)?` — an empty caption under a photograph
            // is not an empty message.
            let needs_text = attachment.is_none() || !text.trim().is_empty();

            // See `render_off_thread`: this is real CPU work (comrak, then
            // ammonia over up to `MAX_RENDERED_MESSAGE_LEN` chars), moved off
            // the async thread so it cannot stall every other connection's
            // heartbeat and ping while it runs. The room lock is still held
            // across the wait — this does not change how many rooms can be
            // mutated at once, only who else can make progress while one of
            // them is busy.
            let render_text = text.clone();
            let rendered = render_off_thread(move || {
                if needs_text {
                    validate_message(&render_text)?;
                }
                Ok::<String, ChatError>(render_message_html(&render_text))
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
            // Answered to the asker alone. This used to go through
            // `room_state.sender` — the room's broadcast channel, shared by
            // every connection — which contradicted this comment rather than
            // implementing it: opening the panel sent every connected user's
            // client a Roster frame it never asked for and quietly applied,
            // O(users) data fanned out to O(users) recipients for one panel
            // one person opened. Returned instead, for the caller — the one
            // connection that actually asked, which is the only place a
            // direct, per-connection sink exists — to send.
            //
            // Throttled with the reaction clock, which is the same "a person
            // clicked something" cadence.
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
    }
}
