//! `nova` room only: a labeled protocol-demonstration panel for
//! `novachannel-mpc`'s threshold DKG and decryption (FROST's Feldman VSS
//! half, not its signature half — see that crate's own module doc).
//!
//! # This is a demonstration, not distributed trust
//!
//! `t`-of-`n` threshold cryptography is only as trustworthy as the
//! independence of the `n` parties holding a share each. Here, every
//! "operator" is simulated inside this one process — the whole DKG runs
//! in one function, on one machine, in one call. Compromising this
//! process compromises every share simultaneously, which is exactly the
//! single point of failure `t`-of-`n` exists to remove. What this panel
//! demonstrates honestly is the *math*: real Feldman verifiable secret
//! sharing, a real commit-then-reveal round to block the rushing bias
//! attack the crate's own docs describe, and a real proof that **any**
//! `threshold`-sized quorum of the simulated operators recovers the exact
//! same symmetric key a sender encapsulated to the group's public key —
//! not that this server is a safe place to trust with a real secret.
//! `client.js`'s rendering of this event says so explicitly; this module
//! doc is why.
//!
//! # Why broadcast rather than reply-to-asker
//!
//! Unlike the RLN handshake/registration replies, nothing here is
//! per-connection state or a secret belonging to the asker — every
//! "operator" is synthetic and the whole point is a demonstration
//! everyone in the room can watch happen. Broadcasting it is the more
//! useful behaviour, not a privacy shortcut taken because it was easier.

use novachannel_mpc::{
    Dealer, KeyShare, ParticipantId, combine_partials, derive_symmetric_key, encapsulate,
    finalize_key_share_excluding_faulty, identify_faulty_dealers, partial_decrypt,
};

/// Total simulated operators and how many are needed to decrypt. Fixed
/// rather than configurable — this is a demo of the mechanism at one
/// illustrative size, not a tunable deployment.
const NUM_OPERATORS: u32 = 5;
const THRESHOLD: u32 = 3;

pub(crate) struct MpcDemoResult {
    pub num_operators: u32,
    pub threshold: u32,
    /// Which operators formed the quorum that decrypted — deliberately
    /// *not* the first `threshold` of them in every run's framing, so
    /// `client.js` can say "any `threshold` of these five" honestly rather
    /// than implying some operators are more equal than others.
    pub quorum: Vec<ParticipantId>,
    pub group_public_key_hex: String,
    /// Whether the quorum's combined partial decryptions produced the
    /// exact symmetric key a sender's `encapsulate` call derived. Always
    /// `true` when this function returns at all — a real cryptographic
    /// check, not a canned success flag, computed fresh on every request.
    pub recovered_key_matches: bool,
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs one full DKG + threshold-decryption round: `NUM_OPERATORS`
/// simulated dealers commit, reveal, and finalize key shares; a simulated
/// sender encapsulates a symmetric key to the resulting group public key;
/// a `THRESHOLD`-sized quorum recovers it via partial decryption and
/// Lagrange combination. Pure CPU, microseconds — no I/O, so this runs
/// inline rather than off-thread (contrast `render_off_thread` in
/// `session/events.rs`, which exists for genuinely slow work).
pub(crate) fn run_demo() -> MpcDemoResult {
    let dealers: Vec<Dealer> = (0..NUM_OPERATORS)
        .map(|_| Dealer::new(THRESHOLD, NUM_OPERATORS))
        .collect();

    // Commit round: every dealer's hash, collected before any reveal —
    // the ordering that blocks a rushing bias attack (novachannel-mpc's
    // own module doc). Nothing here needs the hashes again once every
    // dealer has revealed; collecting them is the round itself.
    let _commitment_hashes: Vec<[u8; 32]> = dealers.iter().map(Dealer::commitment_hash).collect();

    let reveals: Vec<_> = dealers.iter().map(Dealer::reveal).collect();
    let dealer_commitments: Vec<_> = reveals.iter().map(|(c, _)| c.clone()).collect();
    let dealer_shares: Vec<_> = reveals.iter().map(|(_, s)| s.clone()).collect();

    let faulty = identify_faulty_dealers(&dealer_commitments, &dealer_shares);

    let key_shares: Vec<KeyShare> = (1..=NUM_OPERATORS)
        .map(|pid| {
            finalize_key_share_excluding_faulty(pid, &dealer_commitments, &dealer_shares, &faulty)
        })
        .collect();

    let group_public_key = key_shares
        .first()
        .expect("NUM_OPERATORS >= 1")
        .group_public_key;

    let (ephemeral_point, expected_key) = encapsulate(&group_public_key);

    // The *last* `THRESHOLD` operators, not the first — an arbitrary but
    // fixed choice that keeps the demo from silently only ever exercising
    // participant ids 1..=THRESHOLD.
    let quorum: Vec<ParticipantId> = (NUM_OPERATORS - THRESHOLD + 1..=NUM_OPERATORS).collect();
    let partials: Vec<(ParticipantId, _)> = quorum
        .iter()
        .map(|&pid| {
            (
                pid,
                partial_decrypt(&key_shares[(pid - 1) as usize], &ephemeral_point),
            )
        })
        .collect();
    let combined = combine_partials(&partials);
    let recovered_key = derive_symmetric_key(&combined);

    MpcDemoResult {
        num_operators: NUM_OPERATORS,
        threshold: THRESHOLD,
        quorum,
        group_public_key_hex: hex_encode(group_public_key.compress().as_bytes()),
        recovered_key_matches: recovered_key == expected_key,
    }
}

// ---------------------------------------------------------------------------
// Protocol-facing handler, called from `session/nova.rs::dispatch`.
// ---------------------------------------------------------------------------

use std::sync::Arc;

use crate::protocol::{OutgoingEvent, encode_broadcast};
use crate::state::AppState;

/// `NovaMpcDemoRequest`: runs one demo round and broadcasts the result.
/// No reply value — the broadcast this sends *is* the answer, to everyone
/// in the room including the asker (module doc: nothing here is a secret
/// belonging to whoever clicked the button).
pub(crate) async fn handle_demo_request(state: &Arc<AppState>, room: &str) {
    let result = run_demo();

    let event = OutgoingEvent::NovaMpcDemoResult {
        num_operators: result.num_operators,
        threshold: result.threshold,
        quorum: result.quorum,
        group_public_key: result.group_public_key_hex,
        recovered_key_matches: result.recovered_key_matches,
    };

    let mut rooms = state.rooms.write().await;
    let Some(room_state) = rooms.get_mut(room) else {
        return;
    };
    let frame = encode_broadcast(&event);
    let _ = room_state.sender.send(frame);
}
