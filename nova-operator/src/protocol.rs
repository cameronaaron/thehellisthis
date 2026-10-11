//! The wire protocol between a `nova-operator` and the server's
//! `/ws/nova-operator` endpoint (`session/nova_operator.rs`). Both sides use
//! this crate (the server depends on it by path), so the shapes can't drift.
//!
//! Curve values travel as hex via `wire::{point,scalar}_{to,from}_hex`:
//! `novachannel-mpc`'s types don't derive `Serialize`, and a serde shim would
//! be more machinery than the few values this moves.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ParticipantId = u32;

/// One dealer's public Feldman commitments plus its sealed per-recipient
/// shares. `sealed_shares[id]` is `crypto::seal`'s output, base64, opaque to
/// the coordinator and openable only by participant `id`.
///
/// Keyed by the decimal string of the `ParticipantId`: `serde_json` can't
/// deserialize an integer-keyed map inside an internally tagged enum
/// (`#[serde(tag = "type")]`), which was checked directly. Callers convert at
/// the boundary.
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
    /// First frame after the upgrade: this operator's static X25519 public key
    /// (hex), for the roster every other operator receives.
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
    /// This operator assembled its `KeyShare` and computed a group public key
    /// from the public commitments, sent so the coordinator can confirm everyone
    /// converged before declaring the ceremony complete.
    CeremonyAck {
        group_public_key: String,
    },
    /// An unrecoverable failure (a malformed frame, an I/O error). A single
    /// dealer's bad share has its own recoverable path,
    /// [`OperatorMessage::Complaint`].
    CeremonyFailed {
        reason: String,
    },
    /// One of this operator's shares failed `verify_share` against
    /// `against_dealer`'s commitments. `disputed_share` is that share, hex: safe
    /// to reveal, since only this operator was meant to hold it, and it lets
    /// everyone recompute the check instead of trusting the claim
    /// (`session/nova_operator.rs`).
    Complaint {
        against_dealer: ParticipantId,
        disputed_share: String,
    },
    /// This operator has checked all its shares and sent any complaints; the
    /// coordinator waits for this from everyone before closing the window.
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
    /// This operator's id and the full roster, including itself: every operator
    /// needs every other's static key to seal shares to it.
    Welcome {
        participant_id: ParticipantId,
        threshold: u32,
        roster: Vec<(ParticipantId, String)>,
    },
    /// Every operator's commitment hash is in, so reveals are safe (closing the
    /// rushing-bias window). Carries each dealer's hash so its later
    /// `RevealBroadcast` can be checked against it; without that check the commit
    /// round would be decorative. Keyed by decimal string (see
    /// [`RevealPayload::sealed_shares`]).
    AllCommitmentsReceived {
        hashes: BTreeMap<String, String>,
    },
    /// One operator's `Reveal`, relayed to every operator, the sender included.
    /// Each recipient checks that it matches the earlier hash (silently: both are
    /// public) and that its own share verifies; a bad share becomes an
    /// [`OperatorMessage::Complaint`]. After every reveal it sends
    /// [`OperatorMessage::NoMoreComplaints`], complaint or not.
    RevealBroadcast {
        from: ParticipantId,
        payload: RevealPayload,
    },
    /// A complaint the coordinator verified before relaying, as a garbage filter,
    /// not the authority. Every operator re-verifies it, so a false complaint let
    /// through would be caught by any honest operator.
    ComplaintBroadcast {
        from: ParticipantId,
        against_dealer: ParticipantId,
        disputed_share: String,
    },
    /// Everyone has sent [`OperatorMessage::NoMoreComplaints`]: finalise now,
    /// excluding dealers named in verified complaints.
    ComplaintWindowClosed,
    CeremonyComplete {
        group_public_key: String,
    },
    CeremonyFailed {
        reason: String,
    },
    /// This operator reconnected after the ceremony, recognised by its static
    /// key from the completed roster, so no new DKG is needed. Confirms the
    /// `participant_id` its `KeyShare` belongs to.
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
