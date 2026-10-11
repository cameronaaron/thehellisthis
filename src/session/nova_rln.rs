//! `nova` room only: anonymous, rate-limited posting via `novachannel-rln`, and
//! the ORAM-backed nullifier set that checks for double posts without leaking
//! which member posted.
//!
//! # The path is fetched fresh, not cached
//!
//! A membership proof needs the leaf's Merkle path, and the root and every
//! path change whenever anyone registers. A path cached at registration would
//! go stale at the next join and fail root verification with no useful error.
//! Registration (once; it assigns the leaf) is separate from path lookup
//! (immediately before every post), so a proof's path is at most one round trip
//! old.
//!
//! # The server recomputes root, epoch and x
//!
//! A STARK proof binds its exact public inputs (root, epoch, x, y, nullifier);
//! verifying against other values fails. Only `y` and `nullifier`, which need
//! the secret key, are taken from the wire. `root` is the room's current root,
//! `epoch` comes from the clock, and `x` is recomputed from the text being sent,
//! so a proof for other content or a stale epoch fails verification without a
//! separate check.

use std::time::{SystemTime, UNIX_EPOCH};

use nova_operator::wire::{hex_decode, hex_encode};
use novachannel_oram::PathOram;
use novachannel_rln::air::{self, DEPTH, PublicInputs};
use novachannel_rln::merkle::{MerkleTree, PathStep, Side};
use novachannel_rln::share::{Share, recover_secret};
use novachannel_rln::{bytes_to_field, epoch_field};
use tracing::warn;
use winterfell::Proof;
use winterfell::math::StarkField;
use winterfell::math::fields::f64::BaseElement;

use crate::config::NOVA_RLN_EPOCH_SECONDS;
use crate::protocol::{RlnPathStep, RlnSide};

/// `2^DEPTH`: the most members a room can ever register. Unlike
/// [`crate::config::NOVA_MAX_USERS`] (connected at once), commitments are never
/// removed, since that would change the root under everyone and a larger
/// anonymity set only helps, so this bounds distinct anonymous posters over the
/// room's lifetime.
pub(crate) const NOVA_RLN_CAPACITY: usize = 1 << DEPTH;

/// Leaf capacity of each epoch's nullifier ORAM. One epoch holds at most one
/// entry per member ([`NOVA_RLN_CAPACITY`]), so this is generous; the ORAM
/// never evicts, which is why [`NullifierSet`] drops whole epochs instead.
const NULLIFIER_SET_CAPACITY: u64 = 1024;
const NULLIFIER_SET_BUCKET: usize = 4;

fn field_to_hex(value: BaseElement) -> String {
    hex_encode(&value.as_int().to_be_bytes())
}

/// Decodes a canonical field element. A value at or above the modulus is
/// rejected rather than reduced: reducing would let `v` and `v + p` name the
/// same element, the collision class upstream fixed in `bytes_to_field`.
fn hex_to_field(hex: &str) -> Option<BaseElement> {
    let bytes = hex_decode(hex)?;
    let arr: [u8; 8] = bytes.try_into().ok()?;
    let value = u64::from_be_bytes(arr);
    (value < BaseElement::MODULUS).then(|| BaseElement::new(value))
}

/// This room's RLN membership tree and nullifier set, held on `AppState`
/// (`state.nova_rln_group`) like `nova_identity`, rather than on every room's
/// `RoomState`.
pub(crate) struct NovaRlnGroup {
    commitments: Vec<BaseElement>,
    tree: MerkleTree,
    nullifiers: NullifierSet,
}

#[derive(Clone)]
struct StoredShare {
    x: BaseElement,
    y: BaseElement,
}

/// The nullifiers of the two epochs a proof can still verify against (the
/// current one and the grace epoch before it), one ORAM per epoch parity.
/// A nullifier is per member and epoch, so older entries can never match
/// again; the ORAM never evicts, so without dropping them the set would grow
/// with every anonymous post for the life of the process (§3.5).
pub(crate) struct NullifierSet {
    slots: [(Option<u64>, PathOram<StoredShare>); 2],
}

impl NullifierSet {
    pub(crate) fn new() -> Self {
        let fresh = || {
            (
                None,
                PathOram::new(NULLIFIER_SET_CAPACITY, NULLIFIER_SET_BUCKET),
            )
        };
        NullifierSet {
            slots: [fresh(), fresh()],
        }
    }

    /// Records the share `(x, y)` under `nullifier` in `epoch`, which must be
    /// one a proof just verified for.
    pub(crate) fn record(
        &mut self,
        epoch: u64,
        nullifier: BaseElement,
        x: BaseElement,
        y: BaseElement,
    ) -> RlnOutcome {
        let slot = &mut self.slots[(epoch % 2) as usize];
        if slot.0 != Some(epoch) {
            *slot = (
                Some(epoch),
                PathOram::new(NULLIFIER_SET_CAPACITY, NULLIFIER_SET_BUCKET),
            );
        }
        let oram = &mut slot.1;
        let block_id = nullifier.as_int();
        let mut rng = rand::rng();

        match oram.read(block_id, &mut rng) {
            None => {
                oram.write(block_id, StoredShare { x, y }, &mut rng);
                RlnOutcome::Accepted
            }
            Some(existing) if existing.x == x => RlnOutcome::Duplicate,
            Some(existing) => {
                let old_share = Share {
                    nullifier,
                    x: existing.x,
                    y: existing.y,
                };
                let new_share = Share { nullifier, x, y };
                oram.write(block_id, StoredShare { x, y }, &mut rng);
                match recover_secret(&old_share, &new_share) {
                    Some(recovered_sk) => RlnOutcome::Slashed { recovered_sk },
                    // Same nullifier, same `x`: the same message resent. Proving is
                    // deterministic, so this is a network retry and is deduplicated, not
                    // punished.
                    None => RlnOutcome::Accepted,
                }
            }
        }
    }
}

/// What happened when a proof was checked.
pub(crate) enum RlnOutcome {
    /// A fresh nullifier this epoch — the message is authorized.
    Accepted,
    /// Same nullifier, different `x`: two messages from one member in one
    /// epoch, which reveals the member's identity secret from the two shares.
    Duplicate,
    /// The same nullifier, a *different* `x`: two distinct messages from
    /// one member in one epoch. RLN's namesake property — the member's
    /// identity secret is recoverable from the two shares alone.
    Slashed { recovered_sk: BaseElement },
}

impl NovaRlnGroup {
    pub(crate) fn new() -> Self {
        NovaRlnGroup {
            commitments: Vec::new(),
            tree: MerkleTree::new(DEPTH, &[]),
            nullifiers: NullifierSet::new(),
        }
    }

    /// Registers a member's commitment (hex field element) and returns its
    /// permanent leaf index; `None` for a malformed commitment or once the room has
    /// registered [`NOVA_RLN_CAPACITY`] members.
    pub(crate) fn register(&mut self, commitment_hex: &str) -> Option<usize> {
        if self.commitments.len() >= NOVA_RLN_CAPACITY {
            warn!("nova rln: membership tree is full");
            return None;
        }
        let commitment = hex_to_field(commitment_hex)?;
        self.commitments.push(commitment);
        self.tree = MerkleTree::new(DEPTH, &self.commitments);
        let leaf_index = self.commitments.len() - 1;
        tracing::info!(
            leaf_index,
            room_members = self.commitments.len(),
            merkle_root = %field_to_hex(self.tree.root()),
            "nova rln: member registered into the anonymity set"
        );
        Some(leaf_index)
    }

    /// This leaf's *current* Merkle path — see the module doc on why this
    /// is looked up fresh rather than cached at registration.
    pub(crate) fn path_for(&self, leaf_index: usize) -> Option<Vec<RlnPathStep>> {
        if leaf_index >= self.commitments.len() {
            return None;
        }
        Some(
            self.tree
                .path(leaf_index)
                .into_iter()
                .map(wire_path_step)
                .collect(),
        )
    }

    pub(crate) fn root_hex(&self) -> String {
        field_to_hex(self.tree.root())
    }

    /// Verifies an anonymous message's proof and records it in the nullifier set.
    /// `proof_b64`/`y_hex`/`nullifier_hex` are the client's claims; `text` is
    /// checked as sent, which pins `x` to the real content (module doc).
    pub(crate) fn verify_and_record(
        &mut self,
        proof_bytes: &[u8],
        y_hex: &str,
        nullifier_hex: &str,
        text: &str,
    ) -> Result<RlnOutcome, &'static str> {
        let y = hex_to_field(y_hex).ok_or("malformed y")?;
        let nullifier = hex_to_field(nullifier_hex).ok_or("malformed nullifier")?;
        let x = bytes_to_field(text.as_bytes());
        let root = self.tree.root();

        let proof = Proof::from_bytes(proof_bytes).map_err(|_| "malformed proof")?;

        let now = current_epoch();
        // One epoch of grace: the client proves for its clock's epoch, which
        // may have rolled over relative to the server's by arrival.
        let accepted = [now, now.saturating_sub(1)].into_iter().find_map(|epoch| {
            let public = PublicInputs {
                root,
                epoch: epoch_field(epoch),
                x,
                y,
                nullifier,
            };
            air::verify(proof.clone(), public).ok().map(|_| epoch)
        });

        let Some(epoch) = accepted else {
            return Err("proof did not verify against the current root/epoch/message");
        };
        Ok(self.nullifiers.record(epoch, nullifier, x, y))
    }
}

fn wire_path_step(step: PathStep) -> RlnPathStep {
    RlnPathStep {
        sibling: field_to_hex(step.sibling),
        side: match step.side {
            Side::Left => RlnSide::Left,
            Side::Right => RlnSide::Right,
        },
    }
}

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / NOVA_RLN_EPOCH_SECONDS
}

pub(crate) fn recovered_sk_hex(sk: BaseElement) -> String {
    field_to_hex(sk)
}

// ---------------------------------------------------------------------------
// Protocol handlers, called from `session/nova.rs::dispatch`.
//
// They take `&Arc<AppState>` rather than a locked guard because they touch
// only `state.nova_rln_group`, never `state.rooms`, except for
// `handle_message`'s broadcast on success; `session/events.rs` assumes the
// room lock is already held, so these live beside the nova handshake instead.
// ---------------------------------------------------------------------------

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use uuid::Uuid;

use crate::protocol::{OutgoingEvent, encode_broadcast};
use crate::state::AppState;
use crate::validation::validate_and_render_message;

/// `RlnRegister`: adds the commitment to the tree and replies with the leaf
/// index. On a malformed commitment or a full tree nothing is sent, refusing
/// silently like the PQ handshake.
pub(crate) async fn handle_register(
    state: &Arc<AppState>,
    commitment_hex: &str,
) -> Option<OutgoingEvent> {
    let mut group = state.nova_rln_group.write().await;
    let leaf_index = group.register(commitment_hex)?;
    Some(OutgoingEvent::RlnRegistered { leaf_index })
}

/// `RlnPathRequest`: this leaf's current Merkle path (module doc on why
/// it's computed fresh rather than cached).
pub(crate) async fn handle_path_request(
    state: &Arc<AppState>,
    leaf_index: usize,
) -> Option<OutgoingEvent> {
    let group = state.nova_rln_group.read().await;
    let path = group.path_for(leaf_index)?;
    let root = group.root_hex();
    Some(OutgoingEvent::RlnPathResponse { path, root })
}

/// `RlnMessage`: verifies the proof, checks the nullifier and broadcasts either
/// the anonymous message or, if the member just posted twice in one epoch, the
/// recovered secret. Every outcome is logged; `Duplicate` and failed
/// verification broadcast nothing (a refused event does no work worth
/// broadcasting, as with constraint #22).
pub(crate) async fn handle_message(
    state: &Arc<AppState>,
    room: &str,
    proof_b64: &str,
    y_hex: &str,
    nullifier_hex: &str,
    text: &str,
) {
    let Ok(proof_bytes) = BASE64.decode(proof_b64) else {
        warn!("nova rln: proof is not valid base64");
        return;
    };

    let outcome = {
        let mut group = state.nova_rln_group.write().await;
        group.verify_and_record(&proof_bytes, y_hex, nullifier_hex, text)
    };

    let event = match outcome {
        Ok(RlnOutcome::Duplicate) => {
            tracing::debug!("nova rln: duplicate anonymous message, ignored");
            return;
        }
        Ok(RlnOutcome::Accepted) => {
            let merkle_root = state.nova_rln_group.read().await.root_hex();
            tracing::info!(
                y = y_hex,
                nullifier = nullifier_hex,
                proof_base64 = proof_b64,
                merkle_root,
                "nova rln: anonymous message accepted — STARK proof independently verified \
                 against the room's real membership root and current epoch (winterfell::air::verify, \
                 not a client-supplied flag); no sender identity was checked or is recoverable \
                 from this proof alone"
            );
            let rendered = match validate_and_render_message(text) {
                Ok(html) => html,
                Err(e) => {
                    tracing::debug!(error = %e, "nova rln: message rejected");
                    return;
                }
            };
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .to_string();
            OutgoingEvent::NovaAnonymousMessage {
                message_id: Uuid::new_v4(),
                text: rendered,
                timestamp,
            }
        }
        Ok(RlnOutcome::Slashed { recovered_sk }) => {
            let recovered_secret = recovered_sk_hex(recovered_sk);
            tracing::warn!(
                nullifier = nullifier_hex,
                recovered_secret_hex = %recovered_secret,
                "nova rln: rate-limit violation — two distinct messages from the same member \
                 in one epoch, Shamir-style share recombination recovered their identity secret \
                 (the same value about to be broadcast to the room, not a separate claim)"
            );
            OutgoingEvent::NovaRlnSlashed { recovered_secret }
        }
        Err(e) => {
            warn!(error = e, "nova rln: proof failed verification");
            return;
        }
    };

    let mut rooms = state.rooms.write().await;
    let Some(room_state) = rooms.get_mut(room) else {
        return;
    };
    let frame = encode_broadcast(&event);
    let _ = room_state.sender.send(frame);
}
