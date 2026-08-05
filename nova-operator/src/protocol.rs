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
    /// This operator's real, unrecoverable failure to complete the
    /// ceremony — a malformed frame, an I/O error, anything that isn't a
    /// single dealer's bad share (that has its own, recoverable path:
    /// [`OperatorMessage::Complaint`]).
    CeremonyFailed {
        reason: String,
    },
    /// One of this operator's own received shares failed `verify_share`
    /// against `against_dealer`'s published commitments. `disputed_share`
    /// is that one share, hex-encoded — safe to reveal, since it's a
    /// share this operator alone was ever meant to hold, and revealing it
    /// lets every other operator (and the coordinator) independently
    /// recompute the same `verify_share` check against the already-public
    /// commitments rather than taking this operator's word for it. See
    /// `session/nova_operator.rs`'s module doc for the full complaint
    /// round.
    Complaint {
        against_dealer: ParticipantId,
        disputed_share: String,
    },
    /// This operator has checked every one of its received shares and
    /// sent every complaint it has (zero or more) — the signal the
    /// coordinator waits for from all participants before closing the
    /// complaint window and letting everyone finalize.
    NoMoreComplaints,
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
    /// (including the sender, who can treat it as a no-op self-check). Each
    /// recipient checks it immediately: first that it hashes to what the
    /// sender committed to earlier (silent, local, no message needed —
    /// [`OperatorMessage::CommitmentHash`]'s hash and this payload's
    /// commitments are both already public, so every party reaches the
    /// same answer independently), then that its own share verifies
    /// (`verify_share`) — a failure there becomes a
    /// [`OperatorMessage::Complaint`], since only the recipient can detect
    /// a bad share addressed to itself. Once every dealer's `Reveal` has
    /// been checked this way, the operator sends
    /// [`OperatorMessage::NoMoreComplaints`] whether or not it found one.
    RevealBroadcast {
        from: ParticipantId,
        payload: RevealPayload,
    },
    /// A complaint the coordinator independently verified (its own
    /// `verify_share` check against the accused dealer's public
    /// commitments actually failed) before relaying — a spam/garbage
    /// filter, not the authoritative check. Every operator still verifies
    /// it again itself; a coordinator that let a false complaint through
    /// would be caught the moment an honest operator's own recomputation
    /// disagreed and excluded nothing.
    ComplaintBroadcast {
        from: ParticipantId,
        against_dealer: ParticipantId,
        disputed_share: String,
    },
    /// Every participant has sent [`OperatorMessage::NoMoreComplaints`] —
    /// safe to finalize the key share now, excluding any dealer named in a
    /// verified complaint.
    ComplaintWindowClosed,
    CeremonyComplete {
        group_public_key: String,
    },
    CeremonyFailed {
        reason: String,
    },
    /// This operator's WebSocket connection dropped after the ceremony
    /// completed and it has now reconnected — recognized by its static
    /// public key matching a participant from the completed ceremony's
    /// roster, so no new DKG is needed. Confirms the `participant_id` it
    /// already holds a `KeyShare` under.
    Reconnected {
        participant_id: ParticipantId,
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
