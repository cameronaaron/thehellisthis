//! Hex encoding for the curve types the DKG and FROST messages carry
//! (`RistrettoPoint`, `Scalar`), the convention `session/nova_rln.rs` and
//! `nova-wasm` use for RLN field elements. Kept here so operator and
//! coordinator share one encoding without depending on the main crate.

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    // Byte-wise rather than `u8::from_str_radix` on `&str` slices, which
    // panics when a pair splits a multi-byte character and accepts `"+1"`
    // as a second spelling of `"01"`.
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let (pairs, _) = s.as_bytes().as_chunks::<2>();
    pairs
        .iter()
        .map(|&[hi, lo]| Some(digit(hi)? << 4 | digit(lo)?))
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

/// Recomputes `novachannel_mpc::Dealer::commitment_hash()` (SHA-256 over each
/// commitment's compressed bytes, in order) from a received vector, so the
/// recipient and the coordinator can check a `Reveal` against its earlier
/// hash. Pinned to the real method by `dealer_reveal_hash_matches_recomputed_hash`
/// in `main.rs`; this library doesn't depend on `novachannel-mpc`, keeping the
/// coordinator's dependency on it to the wire format.
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

    #[test]
    fn non_ascii_input_is_rejected_not_sliced_mid_character() {
        // Even byte length, but the second char straddles the first pair.
        assert_eq!(hex_decode("a\u{e9}b"), None);
        assert_eq!(hex_decode("\u{e9}\u{e9}"), None);
        assert_eq!(hex_decode("+1"), None);
    }
}
