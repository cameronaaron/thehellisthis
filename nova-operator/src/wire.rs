//! Hex encoding for the curve types the DKG/FROST protocol passes over the
//! wire (`RistrettoPoint`, `Scalar`) — the same hex-string convention
//! `session/nova_rln.rs` and `nova-wasm` already use for the RLN field
//! elements, kept here rather than re-derived so operator and coordinator
//! agree on one encoding without sharing a dependency on the main crate.

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

pub fn point_to_hex(p: &RistrettoPoint) -> String {
    hex_encode(p.compress().as_bytes())
}

pub fn point_from_hex(s: &str) -> Option<RistrettoPoint> {
    let bytes = hex_decode(s)?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    CompressedRistretto(arr).decompress()
}

pub fn scalar_to_hex(s: &Scalar) -> String {
    hex_encode(&s.to_bytes())
}

pub fn scalar_from_hex(s: &str) -> Option<Scalar> {
    let bytes = hex_decode(s)?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Scalar::from_canonical_bytes(arr).into_option()
}

/// Recomputes `novachannel_mpc::Dealer::commitment_hash()` from a
/// *received* commitment vector, so a dealer's `Reveal` can be checked
/// against the hash it committed to earlier — by the recipient operator,
/// and by the coordinator, both needing the identical byte layout that
/// method uses internally (SHA-256 over each commitment's compressed
/// bytes, concatenated, in order). Verified against the real
/// `Dealer::commitment_hash()` by
/// `dealer_reveal_hash_matches_recomputed_hash` in `main.rs`'s own tests
/// (that crate depends on `novachannel-mpc` directly; this one doesn't, to
/// keep the coordinator's dependency on this crate to just the wire
/// format).
pub fn commitment_hash(commitments: &[RistrettoPoint]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for c in commitments {
        hasher.update(c.compress().as_bytes());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;

    #[test]
    fn point_round_trips_through_hex() {
        let p = RISTRETTO_BASEPOINT_POINT;
        let hex = point_to_hex(&p);
        assert_eq!(point_from_hex(&hex), Some(p));
    }

    #[test]
    fn scalar_round_trips_through_hex() {
        let s = Scalar::from(42u64);
        let hex = scalar_to_hex(&s);
        assert_eq!(scalar_from_hex(&hex), Some(s));
    }

    #[test]
    fn malformed_hex_is_rejected_not_panicked_on() {
        assert_eq!(point_from_hex("not hex"), None);
        assert_eq!(scalar_from_hex("00"), None);
        assert_eq!(point_from_hex(""), None);
    }
}
