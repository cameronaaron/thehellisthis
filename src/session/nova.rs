//! `nova` room only: two independent crypto sessions per connection, plus
//! the room's shared `Group`.
//!
//! - A **pairwise** X3DH-established `RatchetedSession`, exactly as this
//!   module used to run for everything, now used only for content the
//!   room's broadcast doesn't already carry to everyone: a connection's
//!   own outgoing sends, and any reply owed to that connection alone
//!   (`RlnRegistered`, `RlnPathResponse`, the generic `apply_client_event`
//!   fallback reply).
//! - The room's shared `novachannel::group::Group` (`state.nova_group`), a
//!   TreeKEM-inspired group ratchet, used only for **broadcast** content —
//!   the server seals a message once and every connection relays the same
//!   bytes, instead of the pairwise design's one reseal per recipient.
//!
//! # Why both, not just the `Group`
//!
//! A `Group`'s `seal`/`open` model has exactly one send chain per sender,
//! with a strictly-increasing sequence number every current member must
//! see in order to keep decrypting. That's perfect for broadcasts — every
//! member gets every message. It cannot express "seal this for one
//! recipient only": sealing a single-recipient reply under it would still
//! advance the server's one shared send chain, and every *other* member
//! who never receives those specific bytes falls one sequence number
//! behind forever — there is no gap-recovery, so their very next real
//! broadcast fails to decrypt too. `RequestRoster`'s own history
//! (`session/events.rs`) already established that a reply-to-asker frame
//! must never travel through a channel everyone shares; a `Group`'s send
//! chain turns out to be exactly that kind of channel. So single-recipient
//! content keeps using the pairwise session, and only genuine broadcasts
//! move to the `Group`.
//!
//! # One seal call per broadcast, not one per recipient
//!
//! `run_seal_loop` is the single, dedicated consumer of the room's
//! plaintext broadcast channel: it reads each frame in the exact order the
//! room published it, seals it once under `state.nova_group`, and
//! republishes to `state.nova_sealed_sender` — the channel
//! `session/lifecycle.rs::join_room` actually subscribes `nova` connections
//! to. Being the *only* caller of `Group::seal` for broadcast content is
//! what keeps the `Group`'s sequence numbers in the same order the room
//! actually published frames in; letting multiple per-connection tasks
//! race to seal the same or different frames would let that order scramble
//! (two frames sealed out of publish order still decrypt individually, but
//! every browser's per-sender sequence check would then reject whichever
//! one arrives "out of turn" for it). `session/tasks.rs::forward_broadcasts`
//! needs no nova-awareness at all as a result — it relays whatever's on
//! its channel verbatim, exactly like every other room.
//!
//! # `Group` membership: server-committed, never a browser
//!
//! The server (`state.nova_identity`) is the room's founding `Group`
//! member — leaf 0 — and the only party that ever calls
//! `propose_add`/`propose_remove`/`propose_update`. A `Group` commit must
//! be applied by every member in the exact order it was produced (module
//! doc on `novachannel::group` — "no concurrent-commit resolution"), and
//! having the server be the sole committer means that order is just "the
//! order `state.nova_group`'s mutex serializes calls in," the same
//! guarantee every other room already gets from the room write lock.
//!
//! # Lock ordering: `nova_group` outer, `state.rooms` inner
//!
//! Committing a membership change (join/leave) needs both: the `Group`
//! mutex to produce the `Commit`, and the room write lock to broadcast it.
//! They're nested — `nova_group` first, `rooms` second — specifically so
//! the commit is produced *and* published as one atomic step; releasing
//! `nova_group` in between would let a second join/leave race ahead of
//! this one's broadcast, delivering commits to browsers out of the order
//! their epochs actually advanced in, which `Group::apply_commit` rejects
//! (`Error::WrongState`). This is the only place in the codebase that
//! nests these two locks, and it must stay the only place, in this order —
//! reversing it anywhere would deadlock against this function.
//! `run_seal_loop` never touches `state.rooms` at all, so it never
//! participates in this ordering question.
//!
//! # This is server-mediated, not a blind relay
//!
//! The server is a real `Group` member and decrypts every message the
//! same way it always has, to render Markdown and run `ammonia`
//! (constraint #8) — nothing here changes what "is this end-to-end
//! encrypted between two browsers" answers to (still no: it's
//! encrypted-to-a-trusted-server, the same as before, just cheaper now for
//! broadcast fan-out). A blind-relay redesign would need markdown
//! rendering and sanitisation to move into the browser — out of scope
//! here.
//!
//! # Peer authentication is still TOFU
//!
//! There is no accounts system to pin a browser's identity against ahead
//! of time, the same reasoning constraint #20 already applies to animal
//! names one layer up. `state.nova_dh_identity`/`nova_signed_prekey`
//! (pairwise) and `state.nova_identity` (the `Group`'s founding key) are
//! all generated fresh every process start, for the matching reason —
//! nothing here is meant to be a long-term pinned host key.
//!
//! # What this does not, and cannot, defend against
//!
//! Every operation this module performs completes inside the browser that
//! holds the room open — `nova-wasm` runs the pairwise/group-ratchet math
//! in WASM linear memory, in the same JS execution context as the page's
//! own DOM. That means the whole scheme above assumes an honest browser,
//! and stops being true the moment that assumption fails:
//!
//! - **A malicious or compromised browser extension** with page-access
//!   permissions can read WASM linear memory, hook `NovaClient.seal`/`open`,
//!   or read plaintext after this module's server-side counterpart already
//!   decrypted it — before or after the fact, not in transit. No CSP
//!   directive constrains an already-installed extension's content script;
//!   `script-src`/`trusted-types` (`security/headers.rs`) raise the bar
//!   against *this page's own* code being hijacked (a bypassed sanitiser, a
//!   sink some future edit adds), not against privileged code the browser
//!   already grants a different trust level.
//! - **A compromised OS or physical access to the device** sees plaintext
//!   the instant it's decrypted, in this browser tab or any other program
//!   running alongside it — moving the crypto into a native process or
//!   extension changes which sandbox holds the keys, not whether an
//!   already-compromised endpoint can read them.
//!
//! Nothing here claims otherwise. This module (and `nova_rln.rs`,
//! `nova_operator.rs`) is real, checkable cryptography — the client-side
//! console logging described in each `novaLog(...)` call in `client.js`
//! exists so that claim can be verified against actual wire values rather
//! than taken on faith — but "real cryptography" and "defends against a
//! compromised endpoint" are different claims, and only the first one is
//! made here.

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

use crate::config::{NOVA_GROUP_CAPACITY, NOVA_RLN_MESSAGE_MIN_INTERVAL};
use crate::protocol::{ClientEvent, OutgoingEvent, encode_broadcast};
use crate::state::AppState;

use super::{apply_client_event, encode_event, nova_operator, nova_rln};

/// How often one connection may trigger the MPC demo (`session/nova_operator.rs`).
/// The demo is cheap CPU, not a resource worth rationing for its own sake —
/// this exists so a bored click doesn't flood the room with repeats of the
/// same broadcast, the same reasoning as the reaction/roster throttles in
/// `session/events.rs`.
const MPC_DEMO_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// Whether this connection's pairwise session is established yet. X3DH's
/// responder completes in one call, so there is no in-between "started but
/// not finished" state to represent here.
enum PairwiseState {
    AwaitingSession,
    // Boxed: `RatchetedSession` grew past clippy's large-enum-variant
    // threshold on an upstream dependency bump (novachannel-watch,
    // 2026-08-04) — every `PairwiseState` value was paying its stack space
    // regardless of which variant it actually held.
    Established(Box<RatchetedSession>),
}

/// Per-connection state, shared between the receive and forward tasks.
/// Created once per `nova` connection; every other room never allocates
/// one. Two independent pieces — the pairwise session and this
/// connection's `Group` leaf, once it's joined — see the module doc for
/// why both exist.
pub(crate) struct NovaSlot {
    pairwise: Mutex<PairwiseState>,
    leaf_index: Mutex<Option<usize>>,
    /// When this connection last triggered the MPC demo, for
    /// [`MPC_DEMO_MIN_INTERVAL`]. A separate mutex — unrelated concern.
    last_mpc_demo: Mutex<Option<Instant>>,
    /// When this connection last submitted an `RlnMessage`, for
    /// [`NOVA_RLN_MESSAGE_MIN_INTERVAL`]. Its own mutex for the same reason: a
    /// throttle on proof verification has nothing to do with the demo's.
    last_rln_message: Mutex<Option<Instant>>,
}

impl NovaSlot {
    pub(crate) fn new() -> Self {
        NovaSlot {
            pairwise: Mutex::new(PairwiseState::AwaitingSession),
            leaf_index: Mutex::new(None),
            last_mpc_demo: Mutex::new(None),
            last_rln_message: Mutex::new(None),
        }
    }

    pub(crate) async fn leaf_index(&self) -> Option<usize> {
        *self.leaf_index.lock().await
    }

    async fn set_leaf_index(&self, leaf: usize) {
        *self.leaf_index.lock().await = Some(leaf);
    }

    /// Handles `ClientEvent::NovaPreKeyBundleRequest`: serves the server's
    /// one published pairwise bundle. Stateless — this doesn't touch
    /// `pairwise` at all, since fetching a bundle doesn't establish
    /// anything by itself.
    fn handle_bundle_request(bundle: &PreKeyBundle) -> OutgoingEvent {
        OutgoingEvent::NovaPreKeyBundleResponse {
            bundle: BASE64.encode(bundle.to_bytes()),
        }
    }

    /// Handles `ClientEvent::NovaX3dhInit`: processes the client's X3DH
    /// init message against this server's long-term pairwise keys, and —
    /// on success — establishes the ratcheted session in one step. `false`
    /// on a malformed message, a session started twice, or a rejected init
    /// message; the client gets no reply either way, the same
    /// "refuse silently rather than half-establish" shape admission uses
    /// for a room at capacity.
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

    /// Opens an incoming `ClientEvent::Sealed { data }` (pairwise) into the
    /// `ClientEvent` it wraps. `None` before the session is established, on
    /// bad base64/ciphertext, or on a ratchet-control record — Phase 1
    /// never sends one, so receiving one is treated as malformed.
    async fn open_pairwise(&self, data_b64: &str) -> Option<ClientEvent> {
        let mut state = self.pairwise.lock().await;
        let PairwiseState::Established(session) = &mut *state else {
            warn!("nova: sealed frame received before the pairwise session was established");
            return None;
        };

        let record = BASE64.decode(data_b64).ok()?;
        let plaintext = match session.open(&record) {
            Ok(Opened::Application(bytes)) => {
                // Full ciphertext (not just its length) so this line can be
                // diffed byte-for-byte against the matching `novaLog` entry
                // in the browser's own console — the same base64 string
                // appearing in two independently-produced logs, one of them
                // this server's, is what makes "the AEAD tag verified"
                // checkable rather than asserted.
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

    /// Seals `plaintext_json` (an already-`encode_event`d `OutgoingEvent`)
    /// for this connection's pairwise session, wrapped as
    /// `OutgoingEvent::Sealed`. `None` before the session is established.
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

    /// `true` and records `now` if this connection last submitted an
    /// `RlnMessage` longer than [`NOVA_RLN_MESSAGE_MIN_INTERVAL`] ago (or never
    /// has).
    ///
    /// Checked *before* the proof is verified, which is the whole point: a
    /// refused frame must not have cost a verification, the same
    /// validate-before-you-spend ordering the reaction path uses
    /// (`session/events.rs`, constraint #22).
    pub(crate) async fn check_rln_message_throttle(&self, now: Instant) -> bool {
        let mut last = self.last_rln_message.lock().await;
        if last.is_some_and(|t| now.duration_since(t) < NOVA_RLN_MESSAGE_MIN_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// Handles `ClientEvent::NovaJoinRequest`: admits `key_package_b64` to the
/// room's `Group` and broadcasts the resulting `Commit`. Returns the
/// `NovaWelcome` reply owed to the joiner alone; every other currently
/// connected member gets the same `Commit` via the room broadcast this
/// already sends (see the module doc's lock-ordering note for why the two
/// happen atomically). `None` on bad base64/bytes, a second join attempt
/// on the same connection, or a group already at [`NOVA_GROUP_CAPACITY`]
/// (`room_user_limit` already turns most of that last case away before a
/// connection gets this far, but the `Group`'s own ceiling is the one that
/// actually matters).
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

        // `propose_add` picks the same first-blank-leaf internally; this
        // mirrors that with only the public `is_member` API so this
        // module learns which leaf was assigned without `Group` needing
        // to expose one (`Commit`'s own `op` field, which carries it, is
        // deliberately private — nothing outside the crate is meant to
        // parse a `Commit`, only pass its bytes along).
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

/// Removes `leaf` from the room's `Group` and broadcasts the resulting
/// `Commit`. Called from connection teardown, not from `dispatch` — a
/// departure isn't a `ClientEvent` a browser sends.
pub(crate) async fn remove_member(state: &Arc<AppState>, room: &str, leaf: usize) {
    let mut group = state.nova_group.lock().await;
    let commit = match group.propose_remove(&state.nova_identity, leaf) {
        Ok(commit) => commit,
        Err(e) => {
            // Already removed (e.g. a duplicate teardown call) or the
            // group somehow disagrees about this leaf — either way,
            // nothing to broadcast.
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

/// Runs for the life of the process once the `nova` room exists
/// (`startup.rs::spawn_nova_seal_loop`): the single, order-preserving
/// consumer of the room's plaintext broadcast channel, sealing each frame
/// under the shared `Group` once and republishing it to
/// `state.nova_sealed_sender`. See the module doc's "one seal call per
/// broadcast" section for why this must be the *only* caller of
/// `Group::seal` for broadcast content.
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

        // `NovaCommit` frames are already correctly shaped, unsealed, by
        // design (`protocol.rs`'s doc on that variant) — sealing one again
        // would double-wrap it for no reason and spend a `Group::seal`
        // call (and a sequence slot) on a frame that already reached its
        // destination through this exact channel.
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

/// Entry point for one parsed `ClientEvent` arriving on a `nova` connection.
///
/// `NovaPreKeyBundleRequest`/`NovaX3dhInit` establish the pairwise session;
/// `NovaJoinRequest` admits this connection to the `Group`. `Sealed` (the
/// pairwise-wrapped variant) is unwrapped into the `ClientEvent` it carries
/// and re-dispatched through the normal room-locked path
/// (`apply_client_event`) exactly as if it had arrived unsealed — nova
/// changes the transport, not what a message *does* once decrypted.
/// Anything else (a plaintext `Message`/`Typing`/... sent directly, or a
/// nested transport frame inside `Sealed`) is rejected: once a `nova`
/// connection exists, every application frame must be sealed.
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
                    // No reply owed to the asker: on success the sender
                    // sees its own message the same way everyone else
                    // does, through the room broadcast this call already
                    // sends (now sealed once, via `run_seal_loop`, not
                    // per-connection) — that is *this connection's*
                    // identity leaking nothing extra, since a `nova`
                    // sender already gets its own broadcasts back like
                    // any other room.
                    // The throttle lives inside `handle_message`, at the
                    // seam where the expense actually starts — see
                    // `nova_rln::RlnClaim`.
                    nova_rln::handle_message(state, room, slot, &proof, &y, &nullifier, &text)
                        .await;
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
