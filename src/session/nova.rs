//! `nova` room only: the novachannel X3DH handshake and the sealed-transport
//! wrapper layered under this one connection's frames.
//!
//! Not a new event *shape* — `protocol.rs`'s `NovaPreKeyBundleRequest`/
//! `NovaPreKeyBundleResponse`/`NovaX3dhInit`/`Sealed` variants are the only
//! new wire surface, and `Sealed` decrypts to an ordinary
//! `ClientEvent`/`OutgoingEvent`. This module is what turns those variants
//! into calls against `novachannel`, kept out of `events.rs` because none of
//! it touches room state or needs the room write lock: the handshake is
//! pure connection-local crypto against `state.nova_*` keys, and sealing is
//! a transform on bytes the existing broadcast path already produced.
//!
//! State lives per-connection, not per-user: `NovaSlot` is created once in
//! `run_session` (only when `room == NOVA_ROOM`) and shared, behind its own
//! small mutex, between the receive task (which opens incoming `Sealed`
//! frames and drives the handshake) and the forward task (which seals
//! outgoing broadcasts). That mutex is scoped to one connection — it is not
//! the room lock, and holding it never blocks another user (§2).
//!
//! # Why X3DH instead of the synchronous 3-message handshake
//!
//! This used to run `novachannel::handshake`'s signed-transcript exchange.
//! X3DH replaces it for one property nova's always-online server doesn't
//! actually need (asynchrony) and one it does want: deniability. The old
//! handshake authenticates by signing the full transcript with each
//! party's long-term identity — a real, checkable proof that "this
//! identity authenticated this specific exchange," which is exactly what
//! makes a leaked transcript non-repudiable evidence. X3DH's only
//! signature is over a medium-term prekey reused across every session
//! established against it (`state.nova_signed_prekey`), never over
//! anything session-specific, so a completed session here could have been
//! fabricated end-to-end by either party alone — not evidence the other
//! participated. For private messaging that is the better property, and
//! it costs nothing extra: X3DH's initiator completes in one message with
//! no reply to wait for, which is fewer round trips than the handshake it
//! replaced, not more.
//!
//! No one-time prekey (`novachannel::prekey::OneTimePreKeyStore`) — see
//! `state.rs`'s doc on `nova_signed_prekey` for why that's a documented
//! simplification, not an oversight.
//!
//! Peer authentication is still TOFU: there is no accounts system to pin a
//! browser's identity against ahead of time, the same reasoning
//! constraint #20 already applies to animal names one layer up.
//! `state.nova_dh_identity`/`nova_signed_prekey` are both generated fresh
//! every process start (state.rs), for the matching reason — nothing here
//! is meant to be a long-term pinned host key, only a session's own
//! authentication.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use novachannel::prekey::{OneTimePreKeyStore, PreKeyBundle};
use novachannel::ratchet::{Opened, RatchetedSession};
use novachannel::x3dh::respond;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::protocol::{ClientEvent, OutgoingEvent};
use crate::state::AppState;

use super::{apply_client_event, encode_event, nova_operator, nova_rln};

/// How often one connection may trigger the MPC demo (`session/nova_operator.rs`).
/// The demo is cheap CPU, not a resource worth rationing for its own sake —
/// this exists so a bored click doesn't flood the room with repeats of the
/// same broadcast, the same reasoning as the reaction/roster throttles in
/// `session/events.rs`.
const MPC_DEMO_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// Whether this connection has an established sealed session yet. Just two
/// states — X3DH's responder completes in one call, unlike the old
/// handshake's three-message back-and-forth, so there is no in-between
/// "started but not finished" state to represent here.
enum NovaState {
    AwaitingSession,
    // Boxed: `RatchetedSession` grew past clippy's large-enum-variant
    // threshold on an upstream dependency bump (novachannel-watch,
    // 2026-08-04) — every `NovaState` value was paying its stack space
    // regardless of which variant it actually held.
    Established(Box<RatchetedSession>),
}

/// Per-connection session state, shared between the receive and forward
/// tasks. Created once per `nova` connection; every other room never
/// allocates one.
pub(crate) struct NovaSlot {
    session: Mutex<NovaState>,
    /// When this connection last triggered the MPC demo, for
    /// [`MPC_DEMO_MIN_INTERVAL`]. A separate mutex from `session` —
    /// unrelated concerns, and the demo throttle has nothing to do with
    /// session establishment.
    last_mpc_demo: Mutex<Option<Instant>>,
}

impl NovaSlot {
    pub(crate) fn new() -> Self {
        NovaSlot {
            session: Mutex::new(NovaState::AwaitingSession),
            last_mpc_demo: Mutex::new(None),
        }
    }

    /// Handles `ClientEvent::NovaPreKeyBundleRequest`: serves the server's
    /// one published bundle. Stateless — this doesn't touch `session` at
    /// all, since fetching a bundle doesn't establish anything by itself
    /// (the client still has to build and send an X3DH init message).
    pub(crate) fn handle_bundle_request(bundle: &PreKeyBundle) -> OutgoingEvent {
        OutgoingEvent::NovaPreKeyBundleResponse {
            bundle: BASE64.encode(bundle.to_bytes()),
        }
    }

    /// Handles `ClientEvent::NovaX3dhInit`: processes the client's X3DH
    /// init message against this server's long-term X3DH keys, and — on
    /// success — establishes the ratcheted session in one step. `false` on
    /// a malformed message, a session started twice, or a rejected init
    /// message (bad signature, replayed one-time-prekey id, ...); the
    /// client gets no reply either way, the same "refuse silently rather
    /// than half-establish" shape admission uses for a room at capacity.
    pub(crate) async fn handle_x3dh_init(&self, state: &Arc<AppState>, message_b64: &str) -> bool {
        let mut slot_state = self.session.lock().await;
        if !matches!(*slot_state, NovaState::AwaitingSession) {
            warn!("nova: x3dh init received out of phase");
            return false;
        }

        let Ok(message_bytes) = BASE64.decode(message_b64) else {
            warn!("nova: x3dh init message is not valid base64");
            return false;
        };

        // Never populated (state.rs's doc on `nova_signed_prekey` — no
        // one-time prekeys), so a fresh empty store here is exactly
        // equivalent to a persistent one: nothing to remember between
        // calls when nothing is ever added.
        let mut opks = OneTimePreKeyStore::new();
        match respond(
            &state.nova_dh_identity,
            &state.nova_signed_prekey,
            &mut opks,
            &message_bytes,
        ) {
            Ok(responded) => {
                *slot_state = NovaState::Established(Box::new(RatchetedSession::new(
                    &responded.session,
                    false,
                )));
                true
            }
            Err(e) => {
                warn!(error = %e, "nova: x3dh init rejected");
                false
            }
        }
    }

    /// Opens an incoming `ClientEvent::Sealed { data }` into the
    /// `ClientEvent` it wraps. `None` before the session is established, on
    /// bad base64/ciphertext, or on a ratchet-control record — Phase 1 never
    /// sends one, so receiving one is treated as malformed rather than
    /// handled.
    pub(crate) async fn open(&self, data_b64: &str) -> Option<ClientEvent> {
        let mut state = self.session.lock().await;
        let NovaState::Established(session) = &mut *state else {
            warn!("nova: sealed frame received before the session was established");
            return None;
        };

        let record = BASE64.decode(data_b64).ok()?;
        let plaintext = match session.open(&record) {
            Ok(Opened::Application(bytes)) => {
                debug!(
                    ciphertext_bytes = record.len(),
                    plaintext_bytes = bytes.len(),
                    "nova: sealed frame opened — ratchet decryption verified"
                );
                bytes
            }
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
    /// before the session is established — the caller's job is deciding
    /// what to do about a `nova` connection that isn't sealed yet
    /// (`forward_broadcasts` simply does not forward until it is).
    pub(crate) async fn seal(&self, plaintext_json: &str) -> Option<OutgoingEvent> {
        let mut state = self.session.lock().await;
        let NovaState::Established(session) = &mut *state else {
            return None;
        };

        let record = session.seal(plaintext_json.as_bytes()).ok()?;
        debug!(
            plaintext_bytes = plaintext_json.len(),
            ciphertext_bytes = record.len(),
            "nova: outgoing frame sealed — ratchet encryption applied"
        );
        Some(OutgoingEvent::Sealed {
            data: BASE64.encode(record),
        })
    }

    /// `true` and records `now` if this connection last ran the MPC demo
    /// longer than [`MPC_DEMO_MIN_INTERVAL`] ago (or never has).
    pub(crate) async fn check_mpc_demo_throttle(&self, now: Instant) -> bool {
        let mut last = self.last_mpc_demo.lock().await;
        if last.is_some_and(|t| now.duration_since(t) < MPC_DEMO_MIN_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// Entry point for one parsed `ClientEvent` arriving on a `nova` connection.
///
/// The bundle-request and X3DH-init variants are handled directly against
/// `slot`. `Sealed` is unwrapped into the `ClientEvent` it carries and
/// re-dispatched through the normal room-locked path (`apply_client_event`)
/// exactly as if it had arrived unsealed — nova changes the transport, not
/// what a message *does* once decrypted. Anything else (a plaintext
/// `Message`/`Typing`/... sent directly, or a nested transport frame inside
/// `Sealed`) is rejected: once a `nova` connection exists, every
/// application frame must be sealed.
///
/// Any reply this produces is already sealed where the protocol requires
/// it — the bundle response is not (nothing is established yet to seal it
/// with); everything else is, because by the time there is a reply to an
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
        ClientEvent::NovaPreKeyBundleRequest => {
            Some(NovaSlot::handle_bundle_request(&state.nova_prekey_bundle))
        }
        ClientEvent::NovaX3dhInit { message } => {
            if slot.handle_x3dh_init(state, &message).await {
                info!(
                    user_id = %user_id,
                    room = %room,
                    "nova: X3DH handshake complete — sealed session established, \
                     server holds no long-term identity key for this session"
                );
            }
            None
        }
        ClientEvent::Sealed { data } => {
            let inner = slot.open(&data).await?;
            if matches!(
                inner,
                ClientEvent::NovaPreKeyBundleRequest
                    | ClientEvent::NovaX3dhInit { .. }
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
                ClientEvent::NovaMpcDemoRequest => {
                    if !slot.check_mpc_demo_throttle(Instant::now()).await {
                        return None;
                    }
                    nova_operator::handle_demo_request(state, room).await;
                    None
                }
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
