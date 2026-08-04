//! `nova` room only: the novachannel handshake and the sealed-transport
//! wrapper layered under this one connection's frames.
//!
//! Not a new event *shape* — `protocol.rs`'s `NovaHandshakeInit`/
//! `NovaHandshakeResponse`/`NovaHandshakeComplete`/`Sealed` variants are the
//! only new wire surface, and `Sealed` decrypts to an ordinary
//! `ClientEvent`/`OutgoingEvent`. This module is what turns those variants
//! into calls against `novachannel`, kept out of `events.rs` because none of
//! it touches room state or needs the room write lock: the handshake is
//! pure connection-local crypto against `state.nova_identity`, and sealing
//! is a transform on bytes the existing broadcast path already produced.
//!
//! State lives per-connection, not per-user: `NovaSlot` is created once in
//! `run_session` (only when `room == NOVA_ROOM`) and shared, behind its own
//! small mutex, between the receive task (which opens incoming `Sealed`
//! frames and drives the handshake) and the forward task (which seals
//! outgoing broadcasts). That mutex is scoped to one connection — it is not
//! the room lock, and holding it never blocks another user (§2).
//!
//! Peer authentication is TOFU (`responder_respond(.., None, ..)`): there is
//! no accounts system to pin a browser's identity against ahead of time, the
//! same reasoning constraint #20 already applies to animal names one layer
//! up. `state.nova_identity` itself is generated fresh every process start
//! (state.rs) for the matching reason — nothing here is meant to be a
//! long-term pinned host key, only a session's own authentication.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use novachannel::{Identity, Opened, RatchetedSession, ResponderHandshakeState, responder_respond};
use tokio::sync::Mutex;
use tracing::warn;

use crate::protocol::{ClientEvent, OutgoingEvent};
use crate::state::AppState;

use super::{apply_client_event, encode_event, nova_rln};

/// Where this connection is in the handshake, or the established session
/// once it completes. `AwaitingHandshake` is the only reachable state for
/// every room except `nova`.
enum NovaState {
    AwaitingHandshake,
    HandshakeStarted(ResponderHandshakeState),
    Established(RatchetedSession),
}

/// Per-connection handshake/ratchet state, shared between the receive and
/// forward tasks. Created once per `nova` connection; every other room never
/// allocates one.
pub(crate) struct NovaSlot(Mutex<NovaState>);

impl NovaSlot {
    pub(crate) fn new() -> Self {
        NovaSlot(Mutex::new(NovaState::AwaitingHandshake))
    }

    /// Handles `ClientEvent::NovaHandshakeInit`: msg1 in, the reply to send
    /// back to this connection alone out. `None` on a malformed message or
    /// a handshake started twice — the client gets no reply either way, the
    /// same "refuse silently rather than half-establish" shape admission
    /// uses for a room at capacity.
    pub(crate) async fn handle_init(
        &self,
        identity: &Identity,
        msg1_b64: &str,
    ) -> Option<OutgoingEvent> {
        let mut state = self.0.lock().await;
        if !matches!(*state, NovaState::AwaitingHandshake) {
            warn!("nova: handshake init received out of phase");
            return None;
        }

        let msg1 = BASE64.decode(msg1_b64).ok()?;
        let (responder_state, msg2) = match responder_respond(identity, None, &msg1) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "nova: handshake init rejected");
                return None;
            }
        };

        *state = NovaState::HandshakeStarted(responder_state);
        Some(OutgoingEvent::NovaHandshakeResponse {
            msg2: BASE64.encode(msg2),
        })
    }

    /// Handles `ClientEvent::NovaHandshakeComplete`: msg3 in, establishing
    /// the ratcheted session on success. No reply — the client already has
    /// everything it needs from its own local `complete()` call.
    pub(crate) async fn handle_complete(&self, msg3_b64: &str) -> bool {
        let mut state = self.0.lock().await;
        let NovaState::HandshakeStarted(_) = &*state else {
            warn!("nova: handshake complete received out of phase");
            return false;
        };
        // `ResponderHandshakeState::complete` consumes `self`; taking the
        // slot's current value is the only way to hand it over without a
        // second field to hold it in.
        let NovaState::HandshakeStarted(responder_state) =
            std::mem::replace(&mut *state, NovaState::AwaitingHandshake)
        else {
            unreachable!("just matched this arm above");
        };

        let Ok(msg3) = BASE64.decode(msg3_b64) else {
            warn!("nova: handshake complete: msg3 is not valid base64");
            return false;
        };

        match responder_state.complete(&msg3) {
            Ok(established) => {
                *state = NovaState::Established(RatchetedSession::new(&established, false));
                true
            }
            Err(e) => {
                warn!(error = %e, "nova: handshake complete rejected");
                false
            }
        }
    }

    /// Opens an incoming `ClientEvent::Sealed { data }` into the
    /// `ClientEvent` it wraps. `None` before the handshake has completed, on
    /// bad base64/ciphertext, or on a ratchet-control record — Phase 1 never
    /// sends one, so receiving one is treated as malformed rather than
    /// handled.
    pub(crate) async fn open(&self, data_b64: &str) -> Option<ClientEvent> {
        let mut state = self.0.lock().await;
        let NovaState::Established(session) = &mut *state else {
            warn!("nova: sealed frame received before the handshake completed");
            return None;
        };

        let record = BASE64.decode(data_b64).ok()?;
        let plaintext = match session.open(&record) {
            Ok(Opened::Application(bytes)) => bytes,
            Ok(Opened::RatchetAdvanced { .. }) => {
                warn!("nova: unexpected ratchet-control record");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "nova: failed to open sealed frame");
                return None;
            }
        };

        serde_json::from_slice::<ClientEvent>(&plaintext).ok()
    }

    /// Seals `plaintext_json` (an already-`encode_event`d `OutgoingEvent`)
    /// for this connection, wrapped as `OutgoingEvent::Sealed`. `None`
    /// before the handshake has completed — the caller's job is deciding
    /// what to do about a `nova` connection that isn't sealed yet
    /// (`forward_broadcasts` simply does not forward until it is).
    pub(crate) async fn seal(&self, plaintext_json: &str) -> Option<OutgoingEvent> {
        let mut state = self.0.lock().await;
        let NovaState::Established(session) = &mut *state else {
            return None;
        };

        let record = session.seal(plaintext_json.as_bytes()).ok()?;
        Some(OutgoingEvent::Sealed {
            data: BASE64.encode(record),
        })
    }
}

/// Entry point for one parsed `ClientEvent` arriving on a `nova` connection.
///
/// The handshake variants are handled directly against `slot`. `Sealed` is
/// unwrapped into the `ClientEvent` it carries and re-dispatched through the
/// normal room-locked path (`apply_client_event`) exactly as if it had
/// arrived unsealed — nova changes the transport, not what a message *does*
/// once decrypted. Anything else (a plaintext `Message`/`Typing`/... sent
/// directly, or a nested transport frame inside `Sealed`) is rejected: once
/// a `nova` connection exists, every application frame must be sealed.
///
/// Any reply this produces is already sealed where the protocol requires
/// it — the handshake response is not (nothing is established yet to seal
/// it with); everything else is, because by the time there is a reply to an
/// unwrapped `Sealed` frame, the session is established by definition.
pub(crate) async fn dispatch(
    state: &Arc<AppState>,
    room: &str,
    user_id: &str,
    animal_name: &str,
    slot: &NovaSlot,
    event: ClientEvent,
) -> Option<OutgoingEvent> {
    match event {
        ClientEvent::NovaHandshakeInit { msg1 } => {
            slot.handle_init(&state.nova_identity, &msg1).await
        }
        ClientEvent::NovaHandshakeComplete { msg3 } => {
            slot.handle_complete(&msg3).await;
            None
        }
        ClientEvent::Sealed { data } => {
            let inner = slot.open(&data).await?;
            if matches!(
                inner,
                ClientEvent::NovaHandshakeInit { .. }
                    | ClientEvent::NovaHandshakeComplete { .. }
                    | ClientEvent::Sealed { .. }
            ) {
                warn!(user_id = %user_id, "nova: sealed frame nested a transport frame");
                return None;
            }

            match inner {
                ClientEvent::RlnRegister { commitment } => {
                    let reply = nova_rln::handle_register(state, &commitment).await?;
                    slot.seal(&encode_event(&reply)).await
                }
                ClientEvent::RlnPathRequest { leaf_index } => {
                    let reply = nova_rln::handle_path_request(state, leaf_index).await?;
                    slot.seal(&encode_event(&reply)).await
                }
                ClientEvent::RlnMessage {
                    proof,
                    y,
                    nullifier,
                    text,
                } => {
                    // No reply owed to the asker: on success the sender
                    // sees its own message the same way everyone else
                    // does, through the room broadcast this call already
                    // sends — that is *this connection's* identity leaking
                    // nothing extra, since a `nova` sender already gets its
                    // own broadcasts back like any other room.
                    nova_rln::handle_message(state, room, &proof, &y, &nullifier, &text).await;
                    None
                }
                // Cover traffic: discarded, on purpose, before it reaches
                // anything that would treat it as content — no room lock,
                // no broadcast, no reply. `padding` is never even read.
                ClientEvent::Dummy { .. } => None,
                inner => {
                    let reply =
                        apply_client_event(state, room, user_id, animal_name, inner).await?;
                    slot.seal(&encode_event(&reply)).await
                }
            }
        }
        _ => {
            warn!(user_id = %user_id, room = %room, "nova: rejecting an unsealed application frame");
            None
        }
    }
}
