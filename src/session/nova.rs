//! `nova` room only: two crypto sessions per connection, plus the room's shared
//! `Group`.
//!
//! - A pairwise X3DH `RatchetedSession`, used only for what the broadcast
//!   doesn't carry to everyone: a connection's own sends and replies owed to it
//!   alone (`RlnRegistered`, `RlnPathResponse`, `apply_client_event`'s reply).
//! - The room's `novachannel::group::Group` (`state.nova_group`), a
//!   TreeKEM-inspired group ratchet, used only for broadcasts: sealed once and
//!   relayed as the same bytes to everyone.
//!
//! # Why both
//!
//! A `Group` has one send chain per sender with a strictly increasing sequence
//! number every member must see in order. That suits broadcasts but can't
//! express "for one recipient": a single-recipient seal would advance the
//! server's shared chain, and every member who never got those bytes would
//! fall behind for good (there's no gap recovery). Like `RequestRoster`
//! (§5.15), a reply to one connection must not travel through a channel
//! everyone shares.
//!
//! # One seal per broadcast
//!
//! `run_seal_loop` is the only consumer of the room's plaintext broadcast
//! channel: it seals each frame once, in publish order, and republishes to
//! `state.nova_sealed_sender`, which `session/lifecycle.rs::join_room`
//! subscribes nova connections to. Being the only broadcast caller of
//! `Group::seal` keeps sequence numbers in publish order; racing
//! per-connection seals would let browsers reject frames as out of turn.
//! `session/tasks.rs::forward_broadcasts` needs no nova awareness.
//!
//! # Membership is committed by the server
//!
//! The server (`state.nova_identity`) is the founding member (leaf 0) and the
//! only caller of `propose_add`/`propose_remove`/`propose_update`. Commits must
//! be applied in production order (`novachannel::group` has no
//! concurrent-commit resolution), and a sole committer makes that the order
//! `state.nova_group`'s mutex serialises.
//!
//! # Lock order: `nova_group` outer, `state.rooms` inner
//!
//! A join or leave needs the group mutex to produce the `Commit` and the room
//! write lock to broadcast it, nested so producing and publishing are atomic;
//! releasing between them would let another commit's broadcast overtake this
//! one, and `Group::apply_commit` rejects out-of-order epochs
//! (`Error::WrongState`). This is the only place the two nest, and it must
//! stay the only place, in this order, or it deadlocks. `run_seal_loop` never
//! touches `state.rooms`.
//!
//! # Server-mediated, not a blind relay
//!
//! The server is a real member and decrypts every message to render Markdown
//! and run ammonia (constraint #8). This is encryption to a trusted server, not
//! end to end between browsers; a blind relay would need rendering and
//! sanitising to move into the browser.
//!
//! # Peer authentication is TOFU
//!
//! There are no accounts to pin a browser against (as constraint #20 says of
//! names). The pairwise keys (`state.nova_dh_identity`, `nova_signed_prekey`)
//! and the group's founding key (`state.nova_identity`) are regenerated every
//! process start; none is a long-term host key.
//!
//! # What this can't defend against
//!
//! The client side runs in `nova-wasm`, in the page's own JS context, so the
//! scheme assumes an honest browser:
//!
//! - A malicious or compromised extension with page access can read WASM
//!   memory, hook `NovaClient.seal`/`open`, or read plaintext after decryption.
//!   CSP and Trusted Types (`security/headers.rs`) guard this page's own code,
//!   not privileged code the browser already trusts.
//! - A compromised OS or physical access sees plaintext as it is decrypted;
//!   moving the crypto to a native process changes which sandbox holds the
//!   keys, not whether a compromised endpoint can read them.
//!
//! This module, `nova_rln.rs` and `nova_operator.rs` are real, checkable
//! cryptography (`client.js`'s `novaLog(...)` output exists so the wire values
//! can be verified), but that is a different claim from defending a
//! compromised endpoint, which isn't made.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use novachannel::group::LeafKeyPackage;
use novachannel::prekey::{OneTimePreKeyStore, PreKeyBundle};
use novachannel::ratchet::{Opened, RatchetedSession};
use novachannel::x3dh::respond;
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, error, info, warn};

use crate::config::NOVA_GROUP_CAPACITY;
use crate::protocol::{ClientEvent, OutgoingEvent, encode_broadcast};
use crate::state::AppState;

use super::{apply_client_event, encode_event, nova_operator, nova_rln};

/// How often one connection may trigger the MPC demo
/// (`session/nova_operator.rs`): not to save CPU, but so a bored click can't
/// flood the room with repeat broadcasts, like the reaction and roster
/// throttles.
const MPC_DEMO_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// Whether this connection's pairwise session exists. X3DH's responder finishes
/// in one call, so there is no half-established state.
enum PairwiseState {
    AwaitingSession,
    // Boxed: an upstream bump grew `RatchetedSession` past clippy's
    // large-enum-variant threshold, and every value paid for it.
    Established(Box<RatchetedSession>),
}

/// Per-connection nova state, shared by the receive and forward tasks and
/// allocated only in the `nova` room: the pairwise session and, once joined,
/// this connection's `Group` leaf.
pub(crate) struct NovaSlot {
    pairwise: Mutex<PairwiseState>,
    leaf_index: Mutex<Option<usize>>,
    /// When this connection last triggered the MPC demo, for
    /// [`MPC_DEMO_MIN_INTERVAL`]. A separate mutex — unrelated concern.
    last_mpc_demo: Mutex<Option<Instant>>,
}

impl NovaSlot {
    pub(crate) fn new() -> Self {
        NovaSlot {
            pairwise: Mutex::new(PairwiseState::AwaitingSession),
            leaf_index: Mutex::new(None),
            last_mpc_demo: Mutex::new(None),
        }
    }

    pub(crate) async fn leaf_index(&self) -> Option<usize> {
        *self.leaf_index.lock().await
    }

    async fn set_leaf_index(&self, leaf: usize) {
        *self.leaf_index.lock().await = Some(leaf);
    }

    /// Handles `NovaPreKeyBundleRequest` by serving the one published bundle.
    /// Stateless: fetching a bundle establishes nothing.
    fn handle_bundle_request(bundle: &PreKeyBundle) -> OutgoingEvent {
        OutgoingEvent::NovaPreKeyBundleResponse {
            bundle: BASE64.encode(bundle.to_bytes()),
        }
    }

    /// Handles `NovaX3dhInit`: runs the client's init message against the server's
    /// pairwise keys and, on success, establishes the session. `false` on a
    /// malformed or rejected message or a second attempt; the client gets no
    /// reply either way, refusing silently rather than half-establishing.
    async fn handle_x3dh_init(&self, state: &Arc<AppState>, message_b64: &str) -> bool {
        let mut slot_state = self.pairwise.lock().await;
        if !matches!(*slot_state, PairwiseState::AwaitingSession) {
            warn!("nova: x3dh init received out of phase");
            return false;
        }

        let Ok(message_bytes) = BASE64.decode(message_b64) else {
            warn!("nova: x3dh init message is not valid base64");
            return false;
        };

        // Never populated (no one-time prekeys; see `state.rs` on
        // `nova_signed_prekey`), so a fresh empty store is equivalent to a
        // persistent one.
        let mut opks = OneTimePreKeyStore::new();
        match respond(
            &state.nova_dh_identity,
            &state.nova_signed_prekey,
            &mut opks,
            &message_bytes,
        ) {
            Ok(responded) => {
                *slot_state = PairwiseState::Established(Box::new(RatchetedSession::new(
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

    /// Opens a pairwise `ClientEvent::Sealed { data }` into the event it wraps.
    /// `None` before the session exists, on bad base64 or ciphertext, or on a
    /// ratchet-control record, which the client never sends.
    async fn open_pairwise(&self, data_b64: &str) -> Option<ClientEvent> {
        let mut state = self.pairwise.lock().await;
        let PairwiseState::Established(session) = &mut *state else {
            warn!("nova: sealed frame received before the pairwise session was established");
            return None;
        };

        let record = BASE64.decode(data_b64).ok()?;
        let plaintext = match session.open(&record) {
            Ok(Opened::Application(bytes)) => {
                // The full ciphertext, so this line can be diffed against the
                // browser console's `novaLog` entry: the same base64 in two
                // independent logs is what makes "the AEAD tag verified"
                // checkable.
                info!(
                    ciphertext_bytes = record.len(),
                    plaintext_bytes = bytes.len(),
                    ciphertext_base64 = %data_b64,
                    "nova: pairwise frame opened — AEAD tag verified against the sender's ratchet key, plaintext recovered"
                );
                bytes
            }
            Ok(Opened::RatchetAdvanced { .. }) => {
                warn!("nova: unexpected ratchet-control record");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "nova: failed to open pairwise frame");
                return None;
            }
        };

        serde_json::from_slice::<ClientEvent>(&plaintext).ok()
    }

    /// Seals an already-encoded `OutgoingEvent` for this connection's pairwise
    /// session as `OutgoingEvent::Sealed`. `None` before the session exists.
    async fn seal_pairwise(&self, plaintext_json: &str) -> Option<OutgoingEvent> {
        let mut state = self.pairwise.lock().await;
        let PairwiseState::Established(session) = &mut *state else {
            return None;
        };

        let record = session.seal(plaintext_json.as_bytes()).ok()?;
        let data_b64 = BASE64.encode(&record);
        info!(
            plaintext_bytes = plaintext_json.len(),
            ciphertext_bytes = record.len(),
            ciphertext_base64 = %data_b64,
            "nova: pairwise reply sealed — this exact base64 is what left the server; \
             compare it against the browser console's matching `open` entry"
        );
        Some(OutgoingEvent::Sealed { data: data_b64 })
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

/// Handles `NovaJoinRequest`: admits `key_package_b64` to the `Group`,
/// broadcasts the `Commit` to existing members (atomically; see the module
/// doc's lock order) and returns the joiner's `NovaWelcome`. `None` on bad
/// bytes, a second join, or a full group ([`NOVA_GROUP_CAPACITY`];
/// `room_user_limit` turns most of these away earlier, but the group's limit
/// is the binding one).
async fn handle_join_request(
    state: &Arc<AppState>,
    room: &str,
    slot: &NovaSlot,
    key_package_b64: &str,
) -> Option<OutgoingEvent> {
    if slot.leaf_index().await.is_some() {
        warn!("nova: join requested twice on the same connection");
        return None;
    }

    let key_package_bytes = BASE64.decode(key_package_b64).ok()?;
    let key_package = match LeafKeyPackage::from_bytes(&key_package_bytes) {
        Ok(kp) => kp,
        Err(e) => {
            warn!(error = %e, "nova: malformed leaf key package");
            return None;
        }
    };

    let (welcome_b64, commit_b64, assigned_leaf) = {
        let mut group = state.nova_group.lock().await;

        // `propose_add` takes the first blank leaf; this mirrors it with
        // the public `is_member` API, since `Commit`'s `op` field is
        // private (callers pass its bytes along, never parse them).
        let assigned_leaf = (0..NOVA_GROUP_CAPACITY).find(|&leaf| !group.is_member(leaf))?;

        let (commit, welcome) = match group.propose_add(&state.nova_identity, key_package) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "nova: group add rejected");
                return None;
            }
        };
        let commit_b64 = BASE64.encode(commit.to_bytes());
        let welcome_b64 = BASE64.encode(welcome.to_bytes());
        let new_epoch = group.epoch();

        // Broadcast while still holding `nova_group`'s lock — see the
        // module doc's lock-ordering note.
        let mut rooms = state.rooms.write().await;
        if let Some(room_state) = rooms.get_mut(room) {
            let _ = room_state
                .sender
                .send(encode_broadcast(&OutgoingEvent::NovaCommit {
                    commit: commit_b64.clone(),
                }));
        }

        info!(
            room = %room,
            leaf = assigned_leaf,
            epoch = new_epoch,
            commit_base64 = %commit_b64,
            welcome_base64 = %welcome_b64,
            "nova: group commit produced and signed by the server's founding identity — \
             broadcast to every other member, full bytes above so the signature and the \
             per-node path-secret ciphertexts inside it can be inspected independently"
        );

        (welcome_b64, commit_b64, assigned_leaf)
    };

    slot.set_leaf_index(assigned_leaf).await;
    info!(room = %room, leaf = assigned_leaf, "nova: group join committed");
    Some(OutgoingEvent::NovaWelcome {
        welcome: welcome_b64,
        commit: commit_b64,
    })
}

/// Removes `leaf` from the `Group` and broadcasts the `Commit`. Called from
/// teardown: a departure isn't a `ClientEvent`.
pub(crate) async fn remove_member(state: &Arc<AppState>, room: &str, leaf: usize) {
    let mut group = state.nova_group.lock().await;
    let commit = match group.propose_remove(&state.nova_identity, leaf) {
        Ok(commit) => commit,
        Err(e) => {
            // Already removed (a duplicate teardown) or the group
            // disagrees about this leaf; nothing to broadcast.
            debug!(error = %e, leaf, "nova: group remove skipped");
            return;
        }
    };
    let commit_b64 = BASE64.encode(commit.to_bytes());
    let new_epoch = group.epoch();

    // Broadcast while still holding `nova_group`'s lock — same reasoning
    // as `handle_join_request`.
    let mut rooms = state.rooms.write().await;
    if let Some(room_state) = rooms.get_mut(room) {
        let _ = room_state
            .sender
            .send(encode_broadcast(&OutgoingEvent::NovaCommit {
                commit: commit_b64.clone(),
            }));
    }
    info!(
        room = %room,
        leaf,
        epoch = new_epoch,
        commit_base64 = %commit_b64,
        "nova: group remove committed — this leaf's ancestor path is blanked; it cannot \
         decrypt anything sealed from this epoch onward, which is checkable, not asserted: \
         its own console will start failing to open subsequent GroupSealed frames"
    );
}

/// Runs for the process's life once the `nova` room exists
/// (`startup.rs::spawn_nova_seal_loop`): the only, order-preserving consumer of
/// the room's plaintext broadcasts, sealing each once under the `Group` and
/// republishing to `state.nova_sealed_sender` (module doc: one seal per
/// broadcast).
pub(crate) async fn run_seal_loop(
    state: Arc<AppState>,
    mut plaintext: tokio::sync::broadcast::Receiver<Arc<str>>,
) {
    loop {
        let frame = match plaintext.recv().await {
            Ok(frame) => frame,
            Err(RecvError::Closed) => return,
            Err(RecvError::Lagged(skipped)) => {
                warn!(
                    skipped,
                    "nova: seal loop lagged — some broadcasts were not sealed"
                );
                continue;
            }
        };

        // `NovaCommit` is deliberately unsealed (see `protocol.rs`);
        // sealing it would double-wrap it and waste a sequence slot.
        if matches!(
            serde_json::from_str::<OutgoingEvent>(&frame),
            Ok(OutgoingEvent::NovaCommit { .. })
        ) {
            let _ = state.nova_sealed_sender.send(frame);
            continue;
        }

        let (sealed, epoch) = {
            let mut group = state.nova_group.lock().await;
            match group.seal(frame.as_bytes()) {
                Ok(record) => (record, group.epoch()),
                Err(e) => {
                    error!(error = %e, "nova: seal loop failed to seal a broadcast");
                    continue;
                }
            }
        };
        let data_b64 = BASE64.encode(&sealed);
        info!(
            plaintext_bytes = frame.len(),
            ciphertext_bytes = sealed.len(),
            epoch,
            ciphertext_base64 = %data_b64,
            "nova: broadcast sealed once under the shared group — every current member's \
             browser console will show this identical base64 string on its way in"
        );
        let wire = encode_event(&OutgoingEvent::GroupSealed { data: data_b64 });
        let _ = state.nova_sealed_sender.send(Arc::from(wire));
    }
}

/// Entry point for one parsed `ClientEvent` on a `nova` connection.
///
/// `NovaPreKeyBundleRequest`/`NovaX3dhInit` set up the pairwise session and
/// `NovaJoinRequest` joins the `Group`. `Sealed` is unwrapped and dispatched
/// through the normal room-locked `apply_client_event`, as if unsealed: nova
/// changes the transport, not what a message does. Anything else (a plaintext
/// `Message`, or a transport frame nested in `Sealed`) is rejected; every
/// application frame on a nova connection must be sealed.
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
                    "nova: X3DH handshake complete — pairwise session established"
                );
            }
            None
        }
        ClientEvent::NovaJoinRequest { key_package } => {
            handle_join_request(state, room, slot, &key_package).await
        }
        ClientEvent::Sealed { data } => {
            let inner = slot.open_pairwise(&data).await?;
            if matches!(
                inner,
                ClientEvent::NovaPreKeyBundleRequest
                    | ClientEvent::NovaX3dhInit { .. }
                    | ClientEvent::NovaJoinRequest { .. }
                    | ClientEvent::Sealed { .. }
            ) {
                warn!(user_id = %user_id, "nova: sealed frame nested a transport frame");
                return None;
            }

            match inner {
                ClientEvent::RlnRegister { commitment } => {
                    let reply = nova_rln::handle_register(state, &commitment).await?;
                    slot.seal_pairwise(&encode_event(&reply)).await
                }
                ClientEvent::RlnPathRequest { leaf_index } => {
                    let reply = nova_rln::handle_path_request(state, leaf_index).await?;
                    slot.seal_pairwise(&encode_event(&reply)).await
                }
                ClientEvent::RlnMessage {
                    proof,
                    y,
                    nullifier,
                    text,
                } => {
                    // No reply owed: the sender sees its own message through
                    // the room broadcast, sealed once by `run_seal_loop`, like
                    // any other room.
                    nova_rln::handle_message(state, room, &proof, &y, &nullifier, &text).await;
                    None
                }
                // Cover traffic, discarded before anything treats it as
                // content: no room lock, broadcast or reply; `padding` is
                // never read.
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
                    slot.seal_pairwise(&encode_event(&reply)).await
                }
            }
        }
        _ => {
            warn!(user_id = %user_id, room = %room, "nova: rejecting an unsealed application frame");
            None
        }
    }
}
