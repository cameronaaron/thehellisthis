//! `nova` room only: a real, distributed `t`-of-`n` FROST/threshold-decryption
//! quorum for the MPC demo panel — genuinely separate `nova-operator`
//! processes (`nova-operator/`), not function calls inside this server.
//!
//! # What changed from the earlier in-process simulation
//!
//! The MPC demo used to run its whole DKG inside `handle_demo_request`:
//! real math, but every "operator" was a value in one stack frame, so
//! compromising this one process compromised every share at once — exactly
//! the single point of failure `t`-of-`n` threshold cryptography exists to
//! remove. This module is what closes that gap: **this server never
//! constructs a `novachannel_mpc::KeyShare`.** It relays DKG messages
//! between operator processes and combines their *public* outputs (partial
//! decryptions, FROST signature shares — both safe to see), but the one
//! type that would let it reconstruct a secret share never appears
//! anywhere in this file, or anywhere else in this crate. That is a
//! structural claim, not a policy one — `grep -rn KeyShare src/` finding
//! nothing is what makes it true, not a comment promising it.
//!
//! # Hub-and-spoke, not a peer mesh
//!
//! Feldman VSS's security doesn't depend on who *relays* a message, only on
//! who can *read* or *compute* with the secret material inside it.
//! Broadcast values (commitments, hashes) are public by design; the one
//! thing that needs protecting in transit is each dealer's point-to-point
//! *share* to each recipient, which is sealed to that recipient's static
//! key (`nova_operator::crypto`) before it ever reaches this relay — this
//! server can forward it, not read it. So every operator only ever talks
//! to this server, over one outbound WebSocket connection each; there is
//! no operator-to-operator networking to stand up.
//!
//! # Malicious-dealer exclusion, two kinds
//!
//! `novachannel_mpc::identify_faulty_dealers` needs every share in the
//! clear to run, which is exactly what this design refuses to give any
//! single party — so it is never called here. Two failure modes, two
//! resolutions, neither trusting a single party's say-so:
//!
//! - **A dealer's `Reveal` doesn't hash to what it committed to earlier**
//!   (the rushing-bias attack the commit round exists to catch). Both
//!   pieces of data — the commit-round hash and the revealed commitments —
//!   are already public the moment the `Reveal` is relayed, so every
//!   operator *and this server* independently recompute the same check and
//!   silently exclude that dealer. No message needed: nobody's word is
//!   taken for anything, so there's nothing to broadcast.
//! - **One operator's own share from a dealer fails `verify_share`.** Only
//!   the recipient can detect this — nobody else has that share. It
//!   becomes a [`OperatorMessage::Complaint`] that reveals the one
//!   disputed share: safe, because it was only ever meant for the
//!   complaining operator, and it lets every other party (including this
//!   server) independently recompute the identical `verify_share` check
//!   against data that was already public otherwise. A complaint that
//!   doesn't actually reproduce is simply ignored, not trusted.
//!
//! Every honest operator (and this server) converges on the same
//! excluded-dealer set either way, because both checks are deterministic
//! functions of public information — the same "identify, don't just abort"
//! property `identify_faulty_dealers` gives the single-process case,
//! realized here without needing global visibility into anyone's shares.
//!
//! # Reconnecting after the ceremony completes
//!
//! The DKG ceremony itself still runs once — a connection dropping
//! *during* it still aborts that run (replaying a half-finished Feldman
//! reveal isn't a problem this module solves). But once
//! [`CeremonyState::Complete`], an operator whose *connection* merely drops
//! (not its process — it still holds its `KeyShare` in memory) can
//! reconnect: it sends the same `Hello` with the same static public key,
//! this server recognizes it against the completed ceremony's roster, and
//! hands back its original `participant_id` via
//! [`CoordinatorMessage::Reconnected`] rather than rejecting it or forcing
//! a whole new ceremony. An operator whose *process* restarts has genuinely
//! lost its share and cannot rejoin under any id — the whole server still
//! needs restarting for a fresh ceremony then, the same regenerate-on-
//! restart posture `state.rs`'s `nova_dh_identity` already has.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use curve25519_dalek::ristretto::RistrettoPoint;
use futures::{SinkExt, StreamExt};
use http::HeaderMap;
use nova_operator::protocol::{CoordinatorMessage, OperatorMessage, RevealPayload};
use nova_operator::wire;
use novachannel_mpc::frost::{
    SigningCommitment, aggregate, public_verification_share, verify, verify_signature_share,
};
use novachannel_mpc::{
    ParticipantId, combine_partials, derive_symmetric_key, encapsulate, verify_share,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::config::{NOVA_OPERATOR_COUNT, NOVA_OPERATOR_REQUEST_TIMEOUT, NOVA_OPERATOR_THRESHOLD};
use crate::protocol::{OutgoingEvent, encode_broadcast};
use crate::security::is_authorized_for_nova_operator;
use crate::state::AppState;

/// One live operator connection: a channel of `CoordinatorMessage`s to push
/// out to it, and the static public key it announced (needed for the
/// roster every other operator seals shares against).
struct OperatorConnection {
    sender: mpsc::UnboundedSender<CoordinatorMessage>,
    static_public_key_hex: String,
}

/// Where the DKG ceremony is. Driven entirely by relayed messages — this
/// server never evaluates a Feldman share, only counts and forwards them.
enum CeremonyState {
    WaitingForRoster,
    Started {
        /// Presence marks "this dealer's commit-round hash arrived";
        /// broadcast to every operator once complete so each can check a
        /// dealer's later `Reveal` against what it committed to here.
        hashes: BTreeMap<ParticipantId, String>,
        /// Each dealer's *public* commitment vector, needed after the
        /// ceremony completes to verify FROST signature shares
        /// (`public_verification_share`) — safe for this server to hold,
        /// unlike the shares themselves.
        dealer_commitments: BTreeMap<ParticipantId, Vec<RistrettoPoint>>,
        /// Every complaint this server independently verified as real
        /// (`from`'s claimed `disputed_share` genuinely fails
        /// `verify_share` against `against_dealer`'s commitments) —
        /// participant ids of the excluded dealers, deduplicated.
        excluded_dealers: BTreeSet<ParticipantId>,
        /// Who has sent `NoMoreComplaints` — once this reaches every
        /// connected operator, the complaint window closes.
        complaints_done: BTreeSet<ParticipantId>,
        acks: BTreeMap<ParticipantId, RistrettoPoint>,
    },
    Complete {
        dealer_commitments: Vec<Vec<RistrettoPoint>>,
        /// 0-based indices into `dealer_commitments` — the dealers a
        /// verified complaint excluded. Passed to
        /// `public_verification_share` so this server's own FROST-share
        /// verification agrees with what the (equally) excluding operators
        /// actually signed with.
        excluded_dealer_indices: Vec<usize>,
        group_public_key: RistrettoPoint,
        /// Every original participant's static public key, kept after the
        /// roster otherwise stops mattering — this is what lets a
        /// reconnecting operator be recognized as the same participant
        /// instead of rejected (module doc, "Reconnecting after the
        /// ceremony completes").
        roster: BTreeMap<ParticipantId, String>,
    },
    /// The reason is logged at the point of failure (`fail_ceremony`), not
    /// carried here — nothing downstream needs to read it back out of the
    /// state itself.
    Failed,
}

pub(crate) struct NovaOperatorRegistry {
    connections: BTreeMap<ParticipantId, OperatorConnection>,
    /// The next participant id to assign — starts at 1, not 0
    /// (`ParticipantId` is 1-indexed throughout `novachannel-mpc`, e.g.
    /// `Dealer::reveal`'s share map).
    next_id: ParticipantId,
    ceremony: CeremonyState,
    /// One outstanding decrypt/sign request per participant at a time —
    /// enough for this demo's one-request-at-a-time flow
    /// (`session/nova.rs`'s `MPC_DEMO_MIN_INTERVAL` throttle already keeps
    /// requests from overlapping).
    pending_replies: BTreeMap<ParticipantId, oneshot::Sender<OperatorMessage>>,
}

impl Default for NovaOperatorRegistry {
    fn default() -> Self {
        NovaOperatorRegistry {
            connections: BTreeMap::new(),
            next_id: 1,
            ceremony: CeremonyState::WaitingForRoster,
            pending_replies: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// The WebSocket endpoint: `/ws/nova-operator`.
// ---------------------------------------------------------------------------

/// Upgrades a `nova-operator` process's connection. Gated on
/// `NOVA_OPERATOR_TOKEN` (`security::is_authorized_for_nova_operator`) —
/// a real DKG secret share is a far higher-value credential than anything
/// else this server gates, so this fails closed exactly like `/metrics`/
/// `/admin` do with no token configured.
pub(crate) async fn nova_operator_ws_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !is_authorized_for_nova_operator(&headers) {
        return http::StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| handle_operator_connection(state, socket))
}

async fn handle_operator_connection(state: Arc<AppState>, socket: WebSocket) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // First frame must be `Hello` — everything else before it is ignored,
    // and a connection that never says hello is simply dropped.
    let static_public_key_hex = loop {
        match ws_stream.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<OperatorMessage>(&text) {
                Ok(OperatorMessage::Hello { static_public_key }) => break static_public_key,
                _ => continue,
            },
            Some(Ok(_)) => continue,
            _ => return,
        }
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<CoordinatorMessage>();

    enum Admission {
        Fresh,
        Reconnected,
    }

    let (participant_id, admission) = {
        let mut registry = state.nova_operator_registry.write().await;
        match &registry.ceremony {
            CeremonyState::WaitingForRoster => {
                if registry.connections.len() as u32 >= NOVA_OPERATOR_COUNT {
                    warn!("nova-operator connection rejected: roster already full");
                    let _ = ws_sink.close().await;
                    return;
                }
                let id = registry.next_id;
                registry.next_id += 1;
                registry.connections.insert(
                    id,
                    OperatorConnection {
                        sender: tx,
                        static_public_key_hex,
                    },
                );
                (id, Admission::Fresh)
            }
            // Reconnecting after the ceremony completed: recognized by its
            // static public key matching a participant from the original
            // roster who isn't currently connected (module doc). Not a new
            // DKG — this operator still holds the `KeyShare` it derived the
            // first time, in its own process's memory.
            CeremonyState::Complete { roster, .. } => {
                let returning_id = roster
                    .iter()
                    .find(|(id, pubkey)| {
                        **pubkey == static_public_key_hex && !registry.connections.contains_key(id)
                    })
                    .map(|(id, _)| *id);
                let Some(id) = returning_id else {
                    warn!(
                        "nova-operator connection rejected: not a recognized participant from the completed ceremony, or already connected"
                    );
                    let _ = ws_sink.close().await;
                    return;
                };
                registry.connections.insert(
                    id,
                    OperatorConnection {
                        sender: tx,
                        static_public_key_hex,
                    },
                );
                (id, Admission::Reconnected)
            }
            CeremonyState::Started { .. } | CeremonyState::Failed => {
                warn!("nova-operator connection rejected: ceremony already running or failed");
                let _ = ws_sink.close().await;
                return;
            }
        }
    };

    match admission {
        Admission::Fresh => {
            info!(participant_id, "nova-operator connected");
            maybe_start_ceremony(&state).await;
        }
        Admission::Reconnected => {
            info!(participant_id, "nova-operator reconnected");
            let registry = state.nova_operator_registry.read().await;
            if let Some(conn) = registry.connections.get(&participant_id) {
                let _ = conn
                    .sender
                    .send(CoordinatorMessage::Reconnected { participant_id });
            }
        }
    }

    let forward_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).expect("CoordinatorMessage always serializes");
            if ws_sink.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_stream.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(operator_msg) = serde_json::from_str::<OperatorMessage>(&text) else {
            continue;
        };
        handle_operator_message(&state, participant_id, operator_msg).await;
    }

    forward_task.abort();
    let mut registry = state.nova_operator_registry.write().await;
    registry.connections.remove(&participant_id);
    // A dropped connection during an in-progress ceremony aborts it — the
    // ceremony's later phases assume every original participant is still
    // reachable (no reconnect story, module doc). A drop *after* the
    // ceremony completes is fine: `handle_demo_request` already only ever
    // asks a live quorum, so the ceremony's `Complete` state is untouched
    // and this participant simply stops being available for future rounds.
    if !matches!(
        registry.ceremony,
        CeremonyState::WaitingForRoster | CeremonyState::Complete { .. } | CeremonyState::Failed
    ) {
        fail_ceremony(
            &mut registry,
            format!("participant {participant_id} disconnected mid-ceremony"),
        );
    }
    info!(participant_id, "nova-operator disconnected");
}

async fn handle_operator_message(state: &Arc<AppState>, from: ParticipantId, msg: OperatorMessage) {
    match msg {
        OperatorMessage::Hello { .. } => {}
        OperatorMessage::CommitmentHash { hash } => on_commitment_hash(state, from, hash).await,
        OperatorMessage::Reveal(payload) => on_reveal(state, from, payload).await,
        OperatorMessage::Complaint {
            against_dealer,
            disputed_share,
        } => on_complaint(state, from, against_dealer, disputed_share).await,
        OperatorMessage::NoMoreComplaints => on_no_more_complaints(state, from).await,
        OperatorMessage::CeremonyAck { group_public_key } => {
            on_ceremony_ack(state, from, group_public_key).await
        }
        OperatorMessage::CeremonyFailed { reason } => {
            let mut registry = state.nova_operator_registry.write().await;
            fail_ceremony(
                &mut registry,
                format!("participant {from} reported: {reason}"),
            );
        }
        OperatorMessage::PartialDecryptResponse { .. }
        | OperatorMessage::Round1Response(_)
        | OperatorMessage::Round2Response { .. } => {
            let mut registry = state.nova_operator_registry.write().await;
            if let Some(tx) = registry.pending_replies.remove(&from) {
                let _ = tx.send(msg);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The DKG ceremony state machine.
// ---------------------------------------------------------------------------

async fn maybe_start_ceremony(state: &Arc<AppState>) {
    let mut registry = state.nova_operator_registry.write().await;
    if !matches!(registry.ceremony, CeremonyState::WaitingForRoster) {
        return;
    }
    if (registry.connections.len() as u32) < NOVA_OPERATOR_COUNT {
        return;
    }

    let roster: Vec<(ParticipantId, String)> = registry
        .connections
        .iter()
        .map(|(id, conn)| (*id, conn.static_public_key_hex.clone()))
        .collect();

    registry.ceremony = CeremonyState::Started {
        hashes: BTreeMap::new(),
        dealer_commitments: BTreeMap::new(),
        excluded_dealers: BTreeSet::new(),
        complaints_done: BTreeSet::new(),
        acks: BTreeMap::new(),
    };

    info!(
        n = roster.len(),
        "nova-operator roster complete, starting ceremony"
    );
    for (id, conn) in &registry.connections {
        let _ = conn.sender.send(CoordinatorMessage::Welcome {
            participant_id: *id,
            threshold: NOVA_OPERATOR_THRESHOLD,
            roster: roster.clone(),
        });
    }
}

async fn on_commitment_hash(state: &Arc<AppState>, from: ParticipantId, hash: String) {
    let mut registry = state.nova_operator_registry.write().await;
    let total = registry.connections.len();

    let ready_hashes = {
        let CeremonyState::Started { hashes, .. } = &mut registry.ceremony else {
            return;
        };
        hashes.insert(from, hash);
        (hashes.len() >= total).then(|| hashes.clone())
    };

    if let Some(hashes) = ready_hashes {
        // Wire keys are decimal strings, not `ParticipantId` — see
        // `nova_operator::protocol::RevealPayload::sealed_shares`'s doc for
        // why an internally-tagged enum can't carry an integer-keyed map.
        let hashes: BTreeMap<String, String> = hashes
            .into_iter()
            .map(|(id, h)| (id.to_string(), h))
            .collect();
        for conn in registry.connections.values() {
            let _ = conn
                .sender
                .send(CoordinatorMessage::AllCommitmentsReceived {
                    hashes: hashes.clone(),
                });
        }
    }
}

async fn on_reveal(state: &Arc<AppState>, from: ParticipantId, payload: RevealPayload) {
    let mut registry = state.nova_operator_registry.write().await;

    let Some(commitments) = payload
        .commitments
        .iter()
        .map(|h| wire::point_from_hex(h))
        .collect::<Option<Vec<_>>>()
    else {
        fail_ceremony(
            &mut registry,
            format!("participant {from}'s reveal had a malformed commitment"),
        );
        return;
    };

    {
        let CeremonyState::Started {
            hashes,
            dealer_commitments,
            excluded_dealers,
            ..
        } = &mut registry.ceremony
        else {
            return;
        };

        // Both sides of this check are already public (the hash from the
        // commit round, these commitments from this very reveal) — every
        // honest operator recomputes the identical comparison and excludes
        // silently, no complaint needed (module doc, "Malicious-dealer
        // exclusion, two kinds").
        let actual_hash = wire::hex_encode(&wire::commitment_hash(&commitments));
        if hashes.get(&from) != Some(&actual_hash) {
            warn!(
                from,
                "nova-operator reveal did not match its earlier committed hash — excluding"
            );
            excluded_dealers.insert(from);
        }

        dealer_commitments.insert(from, commitments);
    }

    for conn in registry.connections.values() {
        let _ = conn.sender.send(CoordinatorMessage::RevealBroadcast {
            from,
            payload: payload.clone(),
        });
    }
}

/// A complaint's authoritative check, run both here (a spam/garbage filter
/// before relaying) and independently by every operator: does `share`
/// really fail `verify_share` against `against_dealer`'s already-public
/// commitments? Pure function of public data — no reason for the server's
/// answer and an honest operator's to ever disagree.
pub(crate) fn complaint_is_valid(
    dealer_commitments: &BTreeMap<ParticipantId, Vec<RistrettoPoint>>,
    against_dealer: ParticipantId,
    from: ParticipantId,
    share: &curve25519_dalek::scalar::Scalar,
) -> bool {
    dealer_commitments
        .get(&against_dealer)
        .is_some_and(|commitments| !verify_share(commitments, from, share))
}

async fn on_complaint(
    state: &Arc<AppState>,
    from: ParticipantId,
    against_dealer: ParticipantId,
    disputed_share_hex: String,
) {
    let mut registry = state.nova_operator_registry.write().await;

    let Some(disputed_share) = wire::scalar_from_hex(&disputed_share_hex) else {
        warn!(from, "nova-operator sent a malformed complaint, ignoring");
        return;
    };

    let valid = {
        let CeremonyState::Started {
            dealer_commitments, ..
        } = &registry.ceremony
        else {
            return;
        };
        complaint_is_valid(dealer_commitments, against_dealer, from, &disputed_share)
    };

    if !valid {
        warn!(
            from,
            against_dealer, "nova-operator complaint did not reproduce, ignoring"
        );
        return;
    }

    info!(
        from,
        against_dealer, "nova-operator complaint verified — excluding dealer"
    );
    if let CeremonyState::Started {
        excluded_dealers, ..
    } = &mut registry.ceremony
    {
        excluded_dealers.insert(against_dealer);
    }

    for conn in registry.connections.values() {
        let _ = conn.sender.send(CoordinatorMessage::ComplaintBroadcast {
            from,
            against_dealer,
            disputed_share: disputed_share_hex.clone(),
        });
    }
}

async fn on_no_more_complaints(state: &Arc<AppState>, from: ParticipantId) {
    let mut registry = state.nova_operator_registry.write().await;
    let total = registry.connections.len();

    let window_closed = {
        let CeremonyState::Started {
            complaints_done, ..
        } = &mut registry.ceremony
        else {
            return;
        };
        complaints_done.insert(from);
        complaints_done.len() >= total
    };

    if window_closed {
        info!("nova-operator complaint window closed");
        for conn in registry.connections.values() {
            let _ = conn.sender.send(CoordinatorMessage::ComplaintWindowClosed);
        }
    }
}

async fn on_ceremony_ack(state: &Arc<AppState>, from: ParticipantId, group_public_key_hex: String) {
    let mut registry = state.nova_operator_registry.write().await;
    let total = registry.connections.len();

    let Some(group_public_key) = wire::point_from_hex(&group_public_key_hex) else {
        fail_ceremony(
            &mut registry,
            format!("participant {from} sent a malformed group public key"),
        );
        return;
    };

    let outcome = {
        let CeremonyState::Started {
            dealer_commitments,
            excluded_dealers,
            acks,
            ..
        } = &mut registry.ceremony
        else {
            return;
        };
        acks.insert(from, group_public_key);
        if acks.len() < total {
            None
        } else {
            let mut values = acks.values();
            let first = *values.next().expect("at least one ack present");
            if values.all(|&pk| pk == first) {
                let excluded_dealer_indices: Vec<usize> = dealer_commitments
                    .keys()
                    .enumerate()
                    .filter(|(_, id)| excluded_dealers.contains(id))
                    .map(|(index, _)| index)
                    .collect();
                Some(Ok((
                    dealer_commitments.values().cloned().collect::<Vec<_>>(),
                    excluded_dealer_indices,
                    first,
                )))
            } else {
                Some(Err(
                    "operators disagreed on the resulting group public key".to_string()
                ))
            }
        }
    };

    match outcome {
        None => {}
        Some(Ok((dealer_commitments, excluded_dealer_indices, group_public_key))) => {
            let roster: BTreeMap<ParticipantId, String> = registry
                .connections
                .iter()
                .map(|(id, conn)| (*id, conn.static_public_key_hex.clone()))
                .collect();
            registry.ceremony = CeremonyState::Complete {
                dealer_commitments,
                excluded_dealer_indices,
                group_public_key,
                roster,
            };
            let confirmed_hex = wire::point_to_hex(&group_public_key);
            info!(group_public_key = %confirmed_hex, "nova-operator ceremony complete");
            for conn in registry.connections.values() {
                let _ = conn.sender.send(CoordinatorMessage::CeremonyComplete {
                    group_public_key: confirmed_hex.clone(),
                });
            }
        }
        Some(Err(reason)) => fail_ceremony(&mut registry, reason),
    }
}

fn fail_ceremony(registry: &mut NovaOperatorRegistry, reason: String) {
    warn!(reason, "nova-operator ceremony failed");
    registry.ceremony = CeremonyState::Failed;
    for conn in registry.connections.values() {
        let _ = conn.sender.send(CoordinatorMessage::CeremonyFailed {
            reason: reason.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// Demo rounds: threshold decryption + FROST signing against a live quorum.
// ---------------------------------------------------------------------------

/// Sends `msg` to participant `pid` and awaits its one reply, up to
/// [`NOVA_OPERATOR_REQUEST_TIMEOUT`]. `None` on a timeout, a disconnected
/// operator, or any other failure to get a reply — callers treat that as
/// "the quorum is no longer available," not a retry.
async fn request_reply(
    state: &Arc<AppState>,
    pid: ParticipantId,
    msg: CoordinatorMessage,
) -> Option<OperatorMessage> {
    let (tx, rx) = oneshot::channel();
    let sender = {
        let mut registry = state.nova_operator_registry.write().await;
        let sender = registry.connections.get(&pid)?.sender.clone();
        registry.pending_replies.insert(pid, tx);
        sender
    };
    if sender.send(msg).is_err() {
        let mut registry = state.nova_operator_registry.write().await;
        registry.pending_replies.remove(&pid);
        return None;
    }
    tokio::time::timeout(NOVA_OPERATOR_REQUEST_TIMEOUT, rx)
        .await
        .ok()?
        .ok()
}

async fn broadcast_event(state: &Arc<AppState>, room: &str, event: OutgoingEvent) {
    let mut rooms = state.rooms.write().await;
    let Some(room_state) = rooms.get_mut(room) else {
        return;
    };
    let frame = encode_broadcast(&event);
    let _ = room_state.sender.send(frame);
}

async fn broadcast_quorum_unavailable(state: &Arc<AppState>, room: &str, live: u32) {
    broadcast_event(
        state,
        room,
        OutgoingEvent::NovaMpcQuorumUnavailable {
            live,
            needed: NOVA_OPERATOR_THRESHOLD,
        },
    )
    .await;
}

/// `NovaMpcDemoRequest`: runs one real threshold-decryption + FROST-signing
/// round against a live quorum of `nova-operator` processes, and broadcasts
/// the result. See this module's doc for what makes it real: this function
/// never constructs a `KeyShare`, only combines the *public* partials and
/// signature shares each operator computed with its own share, locally, in
/// its own process.
pub(crate) async fn handle_demo_request(state: &Arc<AppState>, room: &str) {
    let (dealer_commitments, excluded_dealer_indices, group_public_key) = {
        let registry = state.nova_operator_registry.read().await;
        match &registry.ceremony {
            CeremonyState::Complete {
                dealer_commitments,
                excluded_dealer_indices,
                group_public_key,
                ..
            } => (
                dealer_commitments.clone(),
                excluded_dealer_indices.clone(),
                *group_public_key,
            ),
            _ => {
                let live = registry.connections.len() as u32;
                drop(registry);
                broadcast_quorum_unavailable(state, room, live).await;
                return;
            }
        }
    };

    let quorum: Vec<ParticipantId> = {
        let registry = state.nova_operator_registry.read().await;
        let live: Vec<ParticipantId> = registry.connections.keys().copied().collect();
        if (live.len() as u32) < NOVA_OPERATOR_THRESHOLD {
            let live_count = live.len() as u32;
            drop(registry);
            broadcast_quorum_unavailable(state, room, live_count).await;
            return;
        }
        live.into_iter()
            .take(NOVA_OPERATOR_THRESHOLD as usize)
            .collect()
    };

    // ---- Threshold decryption: does the live quorum recover what a
    // sender encapsulated to the group's public key? ----------------------
    let (ephemeral_point, expected_key) = encapsulate(&group_public_key);
    let mut partials = Vec::with_capacity(quorum.len());
    for &pid in &quorum {
        let request = CoordinatorMessage::PartialDecryptRequest {
            ephemeral_point: wire::point_to_hex(&ephemeral_point),
        };
        let Some(OperatorMessage::PartialDecryptResponse { point }) =
            request_reply(state, pid, request).await
        else {
            broadcast_quorum_unavailable(state, room, quorum.len() as u32).await;
            return;
        };
        let Some(point) = wire::point_from_hex(&point) else {
            broadcast_quorum_unavailable(state, room, quorum.len() as u32).await;
            return;
        };
        partials.push((pid, point));
    }
    let combined = combine_partials(&partials);
    let recovered_key_matches = derive_symmetric_key(&combined) == expected_key;

    // ---- FROST signing: the same quorum jointly signs the room's current
    // RLN membership root. ---------------------------------------------
    let message = state.nova_rln_group.read().await.root_hex();

    let mut commitments_wire = Vec::with_capacity(quorum.len());
    for &pid in &quorum {
        let Some(OperatorMessage::Round1Response(c)) =
            request_reply(state, pid, CoordinatorMessage::Round1Request).await
        else {
            broadcast_quorum_unavailable(state, room, quorum.len() as u32).await;
            return;
        };
        commitments_wire.push(c);
    }
    let Some(frost_commitments): Option<Vec<SigningCommitment>> = commitments_wire
        .iter()
        .map(|c| {
            Some(SigningCommitment {
                participant_id: c.participant_id,
                hiding: wire::point_from_hex(&c.hiding)?,
                binding: wire::point_from_hex(&c.binding)?,
            })
        })
        .collect()
    else {
        broadcast_quorum_unavailable(state, room, quorum.len() as u32).await;
        return;
    };

    let mut signature_shares = Vec::with_capacity(quorum.len());
    let mut every_share_verified = true;
    for &pid in &quorum {
        let request = CoordinatorMessage::Round2Request {
            message: message.clone(),
            signer_ids: quorum.clone(),
            commitments: commitments_wire.clone(),
        };
        let Some(OperatorMessage::Round2Response { z }) = request_reply(state, pid, request).await
        else {
            broadcast_quorum_unavailable(state, room, quorum.len() as u32).await;
            return;
        };
        let Some(z) = wire::scalar_from_hex(&z) else {
            broadcast_quorum_unavailable(state, room, quorum.len() as u32).await;
            return;
        };
        let verification_share =
            public_verification_share(pid, &dealer_commitments, &excluded_dealer_indices);
        if !verify_signature_share(
            pid,
            &verification_share,
            &z,
            message.as_bytes(),
            &quorum,
            &frost_commitments,
            &group_public_key,
        ) {
            every_share_verified = false;
        }
        signature_shares.push((pid, z));
    }

    let signature = aggregate(
        &group_public_key,
        message.as_bytes(),
        &frost_commitments,
        &signature_shares,
    );
    let signature_valid =
        every_share_verified && verify(&signature, &group_public_key, message.as_bytes());

    broadcast_event(
        state,
        room,
        OutgoingEvent::NovaMpcDemoResult {
            num_operators: NOVA_OPERATOR_COUNT,
            threshold: NOVA_OPERATOR_THRESHOLD,
            quorum,
            group_public_key: wire::point_to_hex(&group_public_key),
            recovered_key_matches,
            signed_message: message,
            signature_valid,
        },
    )
    .await;
}
