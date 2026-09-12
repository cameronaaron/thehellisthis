//! `nova` room only: anonymous, rate-limited posting via `novachannel-rln`,
//! and the ORAM-backed nullifier set that keeps checking for a double-post
//! from leaking which member did it.
//!
//! # Why a path is requested fresh, not cached
//!
//! Proving membership needs this leaf's Merkle authentication path — the
//! sibling hash at every level up to the root. The tree's root (and every
//! path through it) changes whenever *any* member registers, because a
//! Merkle root reflects the full leaf set by construction. A path cached at
//! registration time would go stale the moment a second member joined, and
//! a proof built from a stale path fails root verification with no useful
//! error for the sender. Decoupling registration (once, permanent — it's
//! what assigns a leaf index at all) from path lookup (fresh, immediately
//! before every anonymous post) means staleness is never possible: the path
//! a client proves with is never more than one round trip old.
//!
//! # Why the server recomputes root/epoch/x rather than trusting the wire
//!
//! A STARK proof cryptographically binds the exact public inputs (root,
//! epoch, x, y, nullifier) it was built against — verifying against
//! *different* values than what was proved simply fails, it does not
//! "verify the wrong thing". That means `y` and `nullifier` (the two values
//! only the secret-key holder can compute) are the only fields this module
//! trusts from the wire. `root` is this room's actual current tree root,
//! `epoch` is derived from wall-clock time, and `x` is recomputed from the
//! *text the client is sending right now* — substituting the server's own
//! values is what makes a proof-for-different-content or a stale-epoch
//! replay fail verification, rather than something this module has to
//! separately remember to check.

use std::time::{SystemTime, UNIX_EPOCH};

use novachannel_oram::PathOram;
use novachannel_rln::air::{self, DEPTH, PublicInputs};
use novachannel_rln::merkle::{MerkleTree, PathStep, Side};
use novachannel_rln::share::{Share, recover_secret};
use novachannel_rln::{bytes_to_field, epoch_field};
use tracing::warn;
use winterfell::Proof;
use winterfell::math::StarkField;
use winterfell::math::fields::f128::BaseElement;

use crate::config::NOVA_RLN_EPOCH_SECONDS;
use crate::protocol::{RlnPathStep, RlnSide};

/// `2^DEPTH` — the room's total-ever-registered-member ceiling. Not the
/// same ceiling as [`crate::config::NOVA_MAX_USERS`] (concurrently
/// connected): a disconnected member's commitment is never removed from
/// the tree (removing it would change the root under everyone else, and a
/// larger anonymity set is only more private, never less), so this bounds
/// how many distinct people can ever have posted anonymously in one room's
/// lifetime, not how many are connected at once.
pub(crate) const NOVA_RLN_CAPACITY: usize = 1 << DEPTH;

/// How many `(nullifier -> share)` entries the ORAM-backed set holds before
/// the oldest are simply overwritten by the position map's own churn.
/// Sized generously above [`NOVA_RLN_CAPACITY`] so a full room posting
/// once an epoch for a good while doesn't visibly collide; this is a demo
/// room's memory budget, not a production sizing exercise.
const NULLIFIER_SET_CAPACITY: u64 = 1024;
const NULLIFIER_SET_BUCKET: usize = 4;

fn field_to_hex(value: BaseElement) -> String {
    hex_encode(&value.as_int().to_be_bytes())
}

fn hex_to_field(hex: &str) -> Option<BaseElement> {
    let bytes = hex_decode(hex)?;
    let arr: [u8; 16] = bytes.try_into().ok()?;
    Some(BaseElement::new(u128::from_be_bytes(arr)))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// This room's RLN membership tree and its ORAM-backed nullifier set.
/// Lives on `AppState` (`state.nova_rln_group`), the same precedent as
/// `nova_identity` — nova-specific singleton state, not something every
/// other room's `RoomState` should carry a field for.
pub(crate) struct NovaRlnGroup {
    commitments: Vec<BaseElement>,
    tree: MerkleTree,
    nullifiers: PathOram<StoredShare>,
}

#[derive(Clone)]
struct StoredShare {
    x: BaseElement,
    y: BaseElement,
}

/// What happened when a proof was checked.
pub(crate) enum RlnOutcome {
    /// A fresh nullifier this epoch — the message is authorized.
    Accepted,
    /// The same nullifier, same `x`: the exact same message resent.
    /// Deterministic proving means this is indistinguishable from a
    /// network retry, not a second real message, so it is silently
    /// deduplicated rather than treated as a violation.
    Duplicate,
    /// The same nullifier, a *different* `x`: two distinct messages from
    /// one member in one epoch. RLN's namesake property — the member's
    /// identity secret is recoverable from the two shares alone.
    Slashed { recovered_sk: BaseElement },
}

/// One client's anonymous-message claim, parsed and nothing more.
///
/// Separated from verification because the two have completely different
/// costs and completely different needs. Parsing needs no shared state and
/// takes microseconds; verification needs the room's tree, takes the write
/// lock, and is the most expensive thing a client can ask this server to do
/// (measured: 0.4 ms per `air::verify` on a development machine, and the
/// production container has 1/16 of a vCPU). Splitting them means a
/// malformed frame no longer contends on the lock with real ones, and gives
/// the throttle in `session/nova.rs` a seam at exactly the point where the
/// expense begins: a frame that was never going to verify does not spend a
/// verification's worth of budget, and a frame that will, does.
pub(crate) struct RlnClaim {
    proof: Proof,
    y: BaseElement,
    nullifier: BaseElement,
    /// Derived from the text *as sent*, which is what pins the proof to the
    /// actual message content (module doc) — never taken from the wire.
    x: BaseElement,
}

impl RlnClaim {
    /// Parses the wire form, or `None` with the reason logged.
    ///
    /// `text` is hashed here rather than carried, so there is no way for a
    /// caller to verify a proof against anything but the message being sent.
    pub(crate) fn parse(
        proof_b64: &str,
        y_hex: &str,
        nullifier_hex: &str,
        text: &str,
    ) -> Option<Self> {
        let decode = |what: &str, hex: &str| match hex_to_field(hex) {
            Some(field) => Some(field),
            None => {
                warn!("nova rln: malformed {what}");
                None
            }
        };

        let Ok(proof_bytes) = BASE64.decode(proof_b64) else {
            warn!("nova rln: proof is not valid base64");
            return None;
        };
        let y = decode("y", y_hex)?;
        let nullifier = decode("nullifier", nullifier_hex)?;
        let Ok(proof) = Proof::from_bytes(&proof_bytes) else {
            warn!("nova rln: malformed proof");
            return None;
        };

        Some(RlnClaim {
            proof,
            y,
            nullifier,
            x: bytes_to_field(text.as_bytes()),
        })
    }
}

impl NovaRlnGroup {
    pub(crate) fn new() -> Self {
        NovaRlnGroup {
            commitments: Vec::new(),
            tree: MerkleTree::new(DEPTH, &[]),
            nullifiers: PathOram::new(NULLIFIER_SET_CAPACITY, NULLIFIER_SET_BUCKET),
        }
    }

    /// Registers a new member's commitment (hex-encoded field element),
    /// returning its permanent leaf index — `None` for a malformed
    /// commitment or a room that has hit [`NOVA_RLN_CAPACITY`] distinct
    /// registrations over its lifetime.
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

    /// Verifies one anonymous message's proof and checks it against the
    /// nullifier set.
    ///
    /// Takes an already-parsed [`RlnClaim`] rather than the raw wire strings.
    /// Everything that can refuse a frame *without* running a verification —
    /// base64, the two field elements, the proof's own structure — is cheap,
    /// needs none of this state, and used to happen in here, under the write
    /// lock, where a malformed frame contended with every real one. See
    /// [`RlnClaim::parse`].
    pub(crate) fn verify_and_record(
        &mut self,
        claim: RlnClaim,
    ) -> Result<RlnOutcome, &'static str> {
        let RlnClaim {
            proof,
            y,
            nullifier,
            x,
        } = claim;
        let root = self.tree.root();

        let now = current_epoch();
        // A one-epoch grace window: the client proves against whatever
        // epoch its clock says is current, which may have just rolled over
        // relative to the server's by the time this arrives.
        let accepted = [now, now.saturating_sub(1)].into_iter().find_map(|epoch| {
            let public = PublicInputs {
                root,
                epoch: epoch_field(epoch),
                x,
                y,
                nullifier,
            };
            air::verify(proof.clone(), public).ok()
        });

        if accepted.is_none() {
            return Err("proof did not verify against the current root/epoch/message");
        }

        let new_share = StoredShare { x, y };
        let block_id = nullifier.as_int() as u64;
        let mut rng = rand::rng();

        match self.nullifiers.read(block_id, &mut rng) {
            None => {
                self.nullifiers.write(block_id, new_share, &mut rng);
                Ok(RlnOutcome::Accepted)
            }
            Some(existing) if existing.x == x => Ok(RlnOutcome::Duplicate),
            Some(existing) => {
                let old_share = Share {
                    nullifier,
                    x: existing.x,
                    y: existing.y,
                };
                let new_share = Share { nullifier, x, y };
                self.nullifiers
                    .write(block_id, StoredShare { x, y }, &mut rng);
                match recover_secret(&old_share, &new_share) {
                    Some(recovered_sk) => Ok(RlnOutcome::Slashed { recovered_sk }),
                    // Only reachable if `x` values coincided after all (a
                    // hash collision in the u64 truncation used for the
                    // block id) — treat conservatively as accepted rather
                    // than claim a slash this module cannot substantiate.
                    None => Ok(RlnOutcome::Accepted),
                }
            }
        }
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
// Protocol-facing handlers, called from `session/nova.rs::dispatch`.
//
// These take `&Arc<AppState>` directly rather than a pre-locked guard: each
// touches `state.nova_rln_group` only, never the room lock (`state.rooms`)
// except `handle_message`'s own broadcast on success — the same separation
// of concerns as `session/nova.rs`'s handshake, which is why these live
// beside it rather than inside `session/events.rs` (that module's whole
// premise is "already holding the room lock").
// ---------------------------------------------------------------------------

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use uuid::Uuid;

use crate::protocol::{OutgoingEvent, encode_broadcast};
use crate::state::AppState;
use crate::validation::validate_and_render_message;

/// `RlnRegister`: adds `commitment_hex` to the room's membership tree and
/// replies with the assigned leaf index. `None` on a malformed commitment
/// or a full tree — the caller sends no reply either way, the same
/// "refuse silently" shape the PQ handshake uses.
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

/// `RlnMessage`: verifies the proof, checks the nullifier, and — on success
/// — broadcasts either the anonymous message or, if this member has just
/// been caught posting twice in one epoch, the recovered secret. Every
/// outcome is logged; only `Accepted`/`Slashed` produce a room broadcast —
/// `Duplicate` and a failed verification produce nothing, the same
/// "a refused event did no work worth broadcasting" shape constraint #22
/// already uses for the message rate limiter.
pub(crate) async fn handle_message(
    state: &Arc<AppState>,
    room: &str,
    slot: &super::nova::NovaSlot,
    proof_b64: &str,
    y_hex: &str,
    nullifier_hex: &str,
    text: &str,
) {
    // Parsed before the throttle and before the lock: a frame that cannot
    // verify — bad base64, a field element that is not one, bytes that are
    // not a proof — costs microseconds, contends with nobody, and does not
    // spend this connection's verification budget. See `RlnClaim`.
    let Some(claim) = RlnClaim::parse(proof_b64, y_hex, nullifier_hex, text) else {
        return;
    };

    // Only a frame that is actually about to be verified is rationed.
    // Verification is the expense — 0.4 ms measured, ~6 ms on the
    // production container's 1/16 vCPU, inline on a `current_thread`
    // runtime where it is time no other connection in any room gets — and
    // nothing else stood in for this. RLN's own one-per-epoch rule is
    // enforced by the nullifier set, which is only consulted for proofs
    // that *verify*, so replaying one valid frame in a loop cost the server
    // a verification every time and the sender nothing.
    if !slot
        .check_rln_message_throttle(std::time::Instant::now())
        .await
    {
        tracing::debug!("nova rln: message throttled");
        return;
    }

    let outcome = {
        let mut group = state.nova_rln_group.write().await;
        group.verify_and_record(claim)
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
