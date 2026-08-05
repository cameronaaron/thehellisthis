//! A one-shot, anonymous-sender sealed box: seals a short byte string to a
//! recipient's known static X25519 public key, so the chat server — which
//! relays every DKG ceremony message between operators — can forward this
//! one but never read it.
//!
//! Deliberately simpler than `novachannel`'s own X3DH session: this is one
//! secret value, sent once, to a recipient whose static key is already
//! known out of band (the operator roster). There is no conversation to
//! ratchet and nothing to gain from session state that outlives one DKG
//! reveal round — reaching for a full session here would be machinery this
//! use case doesn't need. The construction is the standard "libsodium
//! `crypto_box_seal`" shape: a fresh ephemeral keypair per call, ECDH with
//! the recipient's static key, HKDF-SHA256 to derive a one-time symmetric
//! key, ChaCha20-Poly1305 with an all-zero nonce (safe here specifically
//! *because* the key is fresh and single-use every call — nonce reuse is
//! only a hazard when a key is reused across messages).

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

/// Same construction `novachannel_mpc::csprng()` uses: `getrandom`'s
/// `SysRng` only implements the fallible `TryCryptoRng`; `UnwrapErr` wraps
/// it to the infallible `CryptoRng` x25519-dalek's key generation wants.
fn csprng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

/// Generates a fresh static X25519 identity keypair — this operator's
/// long-term key, persisted by the caller (`main.rs`) across restarts so
/// the roster doesn't have to be redistributed every time the process
/// restarts. Not the DKG secret share itself, which is never persisted —
/// see this module's doc and `main.rs`'s identity-file handling.
pub fn generate_identity() -> StaticSecret {
    StaticSecret::random_from_rng(&mut csprng())
}

fn derive_key(shared: &[u8], ephemeral_public: &[u8; 32], recipient_public: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, shared);
    let mut okm = [0u8; 32];
    let mut info = Vec::with_capacity(21 + 32 + 32);
    info.extend_from_slice(b"nova-operator seal v1");
    info.extend_from_slice(ephemeral_public);
    info.extend_from_slice(recipient_public);
    hk.expand(&info, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}

/// Seals `plaintext` to `recipient_public`. Output is
/// `ephemeral_public (32 bytes) || ciphertext+tag`.
pub fn seal(recipient_public: &PublicKey, plaintext: &[u8]) -> Vec<u8> {
    let ephemeral_secret = StaticSecret::random_from_rng(&mut csprng());
    let ephemeral_public = PublicKey::from(&ephemeral_secret);
    let shared = ephemeral_secret.diffie_hellman(recipient_public);
    let key = derive_key(
        shared.as_bytes(),
        ephemeral_public.as_bytes(),
        recipient_public.as_bytes(),
    );
    let cipher = ChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(&Nonce::default(), plaintext)
        .expect("encryption under a fresh, single-use key cannot fail");

    let mut out = Vec::with_capacity(32 + ciphertext.len());
    out.extend_from_slice(ephemeral_public.as_bytes());
    out.extend_from_slice(&ciphertext);
    out
}

/// Opens a sealed box addressed to `my_secret`. `None` on any malformed or
/// tampered input — the caller (an operator receiving a `Reveal` it cannot
/// yet trust) must treat that as a ceremony failure, not retry with a
/// default.
pub fn open(my_secret: &StaticSecret, sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 32 {
        return None;
    }
    let (ephemeral_public_bytes, ciphertext) = sealed.split_at(32);
    let ephemeral_public_arr: [u8; 32] = ephemeral_public_bytes.try_into().ok()?;
    let ephemeral_public = PublicKey::from(ephemeral_public_arr);
    let shared = my_secret.diffie_hellman(&ephemeral_public);
    let my_public = PublicKey::from(my_secret);
    let key = derive_key(
        shared.as_bytes(),
        &ephemeral_public_arr,
        my_public.as_bytes(),
    );
    let cipher = ChaCha20Poly1305::new((&key).into());
    cipher.decrypt(&Nonce::default(), ciphertext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_then_open_recovers_the_plaintext() {
        let recipient_secret = generate_identity();
        let recipient_public = PublicKey::from(&recipient_secret);

        let sealed = seal(&recipient_public, b"a Feldman share, thirty-two bytes");
        let opened = open(&recipient_secret, &sealed).expect("must open for the real recipient");
        assert_eq!(opened, b"a Feldman share, thirty-two bytes");
    }

    #[test]
    fn a_different_recipient_cannot_open_it() {
        let recipient_secret = generate_identity();
        let recipient_public = PublicKey::from(&recipient_secret);
        let eavesdropper_secret = generate_identity();

        let sealed = seal(&recipient_public, b"secret share bytes");
        assert_eq!(open(&eavesdropper_secret, &sealed), None);
    }

    #[test]
    fn tampered_ciphertext_fails_to_open() {
        let recipient_secret = generate_identity();
        let recipient_public = PublicKey::from(&recipient_secret);

        let mut sealed = seal(&recipient_public, b"secret share bytes");
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert_eq!(open(&recipient_secret, &sealed), None);
    }

    #[test]
    fn truncated_input_is_rejected_not_panicked_on() {
        let recipient_secret = generate_identity();
        assert_eq!(open(&recipient_secret, &[0u8; 10]), None);
        assert_eq!(open(&recipient_secret, &[]), None);
    }
}
