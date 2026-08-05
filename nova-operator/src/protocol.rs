//! The wire protocol between one `nova-operator` process and the chat
//! server's `/ws/nova-operator` endpoint (`session/nova_operator.rs` in the
//! main crate). Shared by both sides via this crate so the two can never
//! drift the way two independently-authored JSON shapes could — the
//! coordinator depends on this crate as a library (see the root
//! `Cargo.toml`'s `nova-operator = { path = "nova-operator" }`), the same
//! way it depends on `novachannel`'s own types for the client-facing
//! protocol.
//!
//! All curve values (`RistrettoPoint`, `Scalar`) travel as hex strings via
//! `wire::{point,scalar}_{to,from}_hex` — `novachannel-mpc`'s own types
//! don't derive `Serialize`, and reaching for a generic point/scalar serde
//! shim would be more machinery than encoding the handful of values this
//! protocol actually moves.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ParticipantId = u32;

/// One dealer's public Feldman commitment vector plus the sealed,
/// per-recipient shares from its reveal round. `sealed_shares[id]` is
/// `crypto::seal`'s output, base64-encoded — opaque to the coordinator,
/// openable only by participant `id`.
///
/// Keyed by the *decimal string* of the `ParticipantId`, not the integer
/// itself: `serde_json` cannot deserialize an integer-keyed map nested
/// inside an internally-tagged enum (`#[serde(tag = "type")]` on
/// [`OperatorMessage`]/[`CoordinatorMessage`]) — confirmed directly against
/// `serde_json` before choosing this over fighting the tagging strategy.
/// Every caller converts at the boundary (`ParticipantId::to_string`/
/// `str::parse`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevealPayload {
    /// Hex-encoded `RistrettoPoint`s, one per Feldman coefficient.
    pub commitments: Vec<String>,
    pub sealed_shares: BTreeMap<String, String>,
}

/// One signer's round-1 FROST output — safe to broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningCommitmentWire {
    pub participant_id: ParticipantId,
    pub hiding: String,
    pub binding: String,
}

/// Operator → coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OperatorMessage {
    /// First frame after the WebSocket upgrade: this operator's static
    /// X25519 public key (hex), so the coordinator can include it in the
    /// roster it hands every other operator.
    Hello {
        static_public_key: String,
    },
    /// Commit round: a hash binding this operator's Feldman commitments
    /// without revealing them yet (`Dealer::commitment_hash`).
    CommitmentHash {
        hash: String,
    },
    /// Reveal round: this operator's own dealing.
    Reveal(RevealPayload),
    /// This operator finished assembling its `KeyShare` locally and
    /// computed a group public key from the (public) per-dealer
    /// commitments it collected — sent so the coordinator can confirm
    /// every operator converged on the same one before calling the
    /// ceremony complete.
    CeremonyAck {
        group_public_key: String,
    },
    /// This operator could not complete the ceremony (e.g. a share failed
    /// `verify_share` against its dealer's own published commitments) —
    /// see `session/nova_operator.rs`'s module doc for why Milestone A
    /// aborts here rather than running the crate's fault-exclusion path.
    CeremonyFailed {
        reason: String,
    },
    PartialDecryptResponse {
        point: String,
    },
    Round1Response(SigningCommitmentWire),
    Round2Response {
        z: String,
    },
}

/// Coordinator → operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CoordinatorMessage {
    /// This operator's assigned id and the full roster (including itself)
    /// — every operator needs every other operator's static public key to
    /// seal shares to them.
    Welcome {
        participant_id: ParticipantId,
        threshold: u32,
        roster: Vec<(ParticipantId, String)>,
    },
    /// Every configured operator's commitment hash is in — safe to reveal
    /// (closes the rushing-bias window `Dealer`'s own docs describe).
    /// Carries every dealer's hash so each operator can, once it receives
    /// that dealer's `RevealBroadcast`, check the revealed commitments
    /// actually hash to what was committed here — the check the commit-
    /// then-reveal ordering exists to make possible. Skipping it would
    /// have made the commit round decorative. Keyed by decimal string —
    /// see [`RevealPayload::sealed_shares`]'s doc for why.
    AllCommitmentsReceived {
        hashes: BTreeMap<String, String>,
    },
    /// One operator's `Reveal`, relayed verbatim to every operator
    /// (including the sender, who can treat it as a no-op self-check).
    RevealBroadcast {
        from: ParticipantId,
        payload: RevealPayload,
    },
    CeremonyComplete {
        group_public_key: String,
    },
    CeremonyFailed {
        reason: String,
    },
    /// A demo round needs this operator's partial decryption of
    /// `ephemeral_point`.
    PartialDecryptRequest {
        ephemeral_point: String,
    },
    /// Round 1 of a FROST signing session: produce a fresh nonce pair and
    /// commitment (`round1_commit`).
    Round1Request,
    /// Round 2: every live signer's round-1 commitment is in; produce this
    /// operator's signature share (`round2_sign`).
    Round2Request {
        message: String,
        signer_ids: Vec<ParticipantId>,
        commitments: Vec<SigningCommitmentWire>,
    },
}
