//! `nova` room only: a real, distributed `t`-of-`n` FROST and
//! threshold-decryption quorum for the MPC demo, run by separate `nova-operator`
//! processes (`nova-operator/`), not by function calls in this server.
//!
//! # Why separate processes
//!
//! The demo once ran the whole DKG inside `handle_demo_request`: real math, but
//! every "operator" lived in one stack frame, so compromising this process
//! compromised every share, the single point of failure threshold cryptography
//! exists to remove. **This server never constructs a
//! `novachannel_mpc::KeyShare`.** It relays DKG messages and combines public
//! outputs (partial decryptions, FROST signature shares); no code in this crate
//! names the type that would let it reconstruct a share. That is structural,
//! not a policy.
//!
//! # Hub and spoke, not a mesh
//!
//! Feldman VSS doesn't care who relays a message, only who can read the secret
//! material. Broadcast values (commitments, hashes) are public; each dealer's
//! point-to-point share is sealed to its recipient's static key
//! (`nova_operator::crypto`) before it reaches this relay, which can forward it
//! but not read it. Each operator holds one outbound WebSocket to this server
//! and never talks to another operator.
//!
//! # Excluding malicious dealers
//!
//! `novachannel_mpc::identify_faulty_dealers` needs every share in the clear,
//! which this design gives no single party, so it isn't called. Instead:
//!
//! - **A dealer's `Reveal` doesn't match its committed hash** (the rushing-bias
//!   attack the commit round catches). Both values are public once relayed, so
//!   every operator and this server recompute the check and exclude the dealer
//!   silently; nobody's word is needed.
//! - **An operator's own share fails `verify_share`.** Only the recipient can
//!   tell, so it sends an [`OperatorMessage::Complaint`] revealing that one
//!   share (meant only for it anyway), and everyone recomputes `verify_share`
//!   against public data. A complaint that doesn't reproduce is ignored.
//!
//! Both checks are deterministic over public data, so honest parties converge
//! on the same excluded set: `identify_faulty_dealers`'s "identify, don't just
//! abort" without anyone seeing others' shares.
//!
//! # Reconnecting after the ceremony
//!
//! The ceremony runs once, and a drop during it aborts that run. Once
//! [`CeremonyState::Complete`], an operator whose connection drops (its process,
//! and `KeyShare`, intact) can send the same `Hello` with the same static key;
//! this server matches it against the roster and returns its original
//! `participant_id` in [`CoordinatorMessage::Reconnected`]. An operator whose
//! process restarted has lost its share and can't rejoin under any id; a fresh
//! ceremony then needs a server restart, the same regenerate-on-restart posture
//! as `state.rs`'s `nova_dh_identity`.

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

/// One live operator connection: its outbound message channel and the static
/// public key it announced, which the roster others seal shares to needs.
struct OperatorConnection {
    sender: mpsc::UnboundedSender<CoordinatorMessage>,
    static_public_key_hex: String,
}

/// Where the DKG ceremony is. Driven entirely by relayed messages — this
/// server never evaluates a Feldman share, only counts and forwards them.
enum CeremonyState {
    WaitingForRoster,
    Started {
        /// A dealer's commit-round hash, once it arrives; broadcast when complete
        /// so each operator can check that dealer's later `Reveal`.
        hashes: BTreeMap<ParticipantId, String>,
        /// Each dealer's public commitment vector, kept to verify FROST
        /// signature shares after the ceremony (`public_verification_share`).
        /// Public, unlike the shares.
        dealer_commitments: BTreeMap<ParticipantId, Vec<RistrettoPoint>>,
        /// Dealers excluded by complaints this server verified itself (the
        /// disputed share really fails `verify_share`), deduplicated.
        excluded_dealers: BTreeSet<ParticipantId>,
        /// Who has sent `NoMoreComplaints` — once this reaches every
        /// connected operator, the complaint window closes.
        complaints_done: BTreeSet<ParticipantId>,
        acks: BTreeMap<ParticipantId, RistrettoPoint>,
    },
    Complete {
        dealer_commitments: Vec<Vec<RistrettoPoint>>,
        /// 0-based indices into `dealer_commitments` of excluded dealers, passed
        /// to `public_verification_share` so this server's FROST-share check
        /// agrees with what the operators signed with.
        excluded_dealer_indices: Vec<usize>,
        group_public_key: RistrettoPoint,
        /// Every original participant's static public key, kept so a
        /// reconnecting operator is recognised (module doc).
        roster: BTreeMap<ParticipantId, String>,
    },
    /// Logged where it happens (`fail_ceremony`); nothing reads it back.
    Failed,
}

pub(crate) struct NovaOperatorRegistry {
    connections: BTreeMap<ParticipantId, OperatorConnection>,
    /// The next participant id: 1-based, as throughout `novachannel-mpc`
    /// (e.g. `Dealer::reveal`'s share map).
    next_id: ParticipantId,
    ceremony: CeremonyState,
    /// One outstanding request per participant, enough for this demo:
    /// `session/nova.rs`'s `MPC_DEMO_MIN_INTERVAL` keeps requests from
    /// overlapping.
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

/// Upgrades a `nova-operator` connection, gated on `NOVA_OPERATOR_TOKEN`
/// (`security::is_authorized_for_nova_operator`). A DKG share is worth more
/// than anything else this server gates, so it fails closed without a token,
/// like `/metrics` and `/admin`.
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
            // A reconnect after the ceremony: its static key matches an
            // original participant who isn't connected (module doc). Not a
            // new DKG; the operator still holds its `KeyShare` in memory.
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
    // A drop during a ceremony aborts it, since later phases assume
    // every participant is reachable. After completion it's fine:
    // `handle_demo_request` only asks a live quorum, and this
    // participant just stops being available.
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
        // Wire keys are decimal strings, not `ParticipantId`; see
        // `RevealPayload::sealed_shares` on why an internally tagged enum
        // can't carry an integer-keyed map.
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

        // Both sides are public (the commit-round hash and this reveal's
        // commitments), so every honest operator recomputes this and
        // excludes silently, with no complaint (module doc).
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

/// A complaint's check, run here (to filter garbage before relaying) and by
/// every operator: does `share` really fail `verify_share` against
/// `against_dealer`'s public commitments? A pure function of public data, so
/// honest answers can't disagree.
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

/// Sends `msg` to `pid` and awaits one reply, up to
/// [`NOVA_OPERATOR_REQUEST_TIMEOUT`]. `None` on a timeout, a disconnect or any
/// other failure, which callers treat as "the quorum is gone", not a retry.
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

/// `NovaMpcDemoRequest`: one threshold-decryption and FROST-signing round
/// against the live quorum, broadcast to the room. It never constructs a
/// `KeyShare`; it combines the public partials and signature shares each
/// operator computed with its own share, in its own process.
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

    info!(
        quorum = ?quorum,
        threshold = NOVA_OPERATOR_THRESHOLD,
        num_operators = NOVA_OPERATOR_COUNT,
        group_public_key = %wire::point_to_hex(&group_public_key),
        recovered_key_matches,
        signature_valid,
        "nova-operator MPC/FROST demo round complete — this server combined only public \
         partials and signature shares, never a KeyShare"
    );

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
