//! A standalone `nova` MPC/FROST operator: one participant in a real
//! `t`-of-`n` Feldman DKG and FROST threshold signature, run as its own
//! process so the chat server never holds this operator's secret share.
//! See the plan this implements and `session/nova_operator.rs`'s module
//! doc in the main crate for the full design (hub-and-spoke relay through
//! the coordinator, point-to-point shares sealed so the relay can forward
//! but not read them).
//!
//! Run one of these per participant, ideally on genuinely separate
//! machines. Usage:
//!
//! ```text
//! nova-operator --server wss://thehellisthis.com/ws/nova-operator \
//!                --token "$NOVA_OPERATOR_TOKEN" \
//!                --identity ~/.nova-operator/identity.key
//! ```
//!
//! `--identity` defaults to `~/.nova-operator/identity.key` if omitted.
//! That file holds this operator's long-term X25519 identity keypair —
//! generated on first run, reused on every restart so the roster doesn't
//! have to be redistributed. It is **not** the DKG secret share: that
//! lives only in this process's memory for the ceremony's lifetime and is
//! never written to disk, the same ephemeral-by-design posture as
//! everything else in this project (`ENGINEERING-STANDARDS.md` §3) — this
//! identity file is the one deliberate exception, and it is a long-term
//! *identity*, not the secret the whole design exists to protect.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use futures_util::{SinkExt, StreamExt};
use nova_operator::protocol::{
    CoordinatorMessage, OperatorMessage, RevealPayload, SigningCommitmentWire,
};
use nova_operator::{crypto, wire};
use novachannel_mpc::frost::{SecretNonces, round1_commit, round2_sign};
use novachannel_mpc::{Dealer, KeyShare, ParticipantId, finalize_key_share, verify_share};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use x25519_dalek::{PublicKey, StaticSecret};

/// Recomputes `Dealer::commitment_hash()` from a *received* commitment
/// vector, so a dealer's `Reveal` can be checked against the hash it
/// committed to earlier. Must match that method's own byte layout exactly
/// (SHA-256 over each commitment's compressed bytes, concatenated, in
/// order) — verified against it directly by
/// `dealer_reveal_hash_matches_recomputed_hash` below.
fn commitment_hash(commitments: &[curve25519_dalek::ristretto::RistrettoPoint]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for c in commitments {
        hasher.update(c.compress().as_bytes());
    }
    hasher.finalize().into()
}

fn parse_args() -> (String, String, PathBuf) {
    let mut server = None;
    let mut token = None;
    let mut identity = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = args.next(),
            "--token" => token = args.next(),
            "--identity" => identity = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let server = server.unwrap_or_else(|| {
        eprintln!("--server <ws(s)://host/ws/nova-operator> is required");
        std::process::exit(2);
    });
    let token = token.unwrap_or_else(|| {
        eprintln!(
            "--token <bearer token> is required (must match the coordinator's NOVA_OPERATOR_TOKEN)"
        );
        std::process::exit(2);
    });
    let identity = identity.unwrap_or_else(default_identity_path);

    (server, token, identity)
}

fn default_identity_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".nova-operator").join("identity.key")
}

/// Loads this operator's persistent identity keypair from `path`, or
/// generates and saves a fresh one if the file doesn't exist yet.
/// `0600` permissions: this file is a long-term identity secret, readable
/// by no one but this operator's own user account.
fn load_or_create_identity(path: &Path) -> StaticSecret {
    match fs::read(path) {
        Ok(bytes) => {
            let arr: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .unwrap_or_else(|_| panic!("{} is not a 32-byte key", path.display()));
            StaticSecret::from(arr)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let secret = crypto::generate_identity();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create identity directory");
            }
            fs::write(path, secret.to_bytes()).expect("write identity file");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                    .expect("restrict identity file permissions");
            }
            println!("generated a new identity, saved to {}", path.display());
            secret
        }
        Err(e) => panic!("reading identity file {}: {e}", path.display()),
    }
}

#[tokio::main]
async fn main() {
    let (server, token, identity_path) = parse_args();
    let identity = load_or_create_identity(&identity_path);
    let my_static_public = PublicKey::from(&identity);

    let mut request = server
        .as_str()
        .into_client_request()
        .expect("--server must be a valid ws(s):// URL");
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("token must be a valid header value"),
    );

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect to coordinator");
    let (mut write, mut read) = ws_stream.split();

    send(
        &mut write,
        &OperatorMessage::Hello {
            static_public_key: wire::hex_encode(my_static_public.as_bytes()),
        },
    )
    .await;

    // ---- Wait for the roster ------------------------------------------
    let (my_id, threshold, roster) = loop {
        let Some(msg) = next_coordinator_message(&mut read).await else {
            eprintln!("coordinator closed the connection before sending a roster");
            std::process::exit(1);
        };
        if let CoordinatorMessage::Welcome {
            participant_id,
            threshold,
            roster,
        } = msg
        {
            break (participant_id, threshold, roster);
        }
    };
    let n = roster.len() as u32;
    println!("assigned participant id {my_id} of {n} (threshold {threshold})");

    let peer_public_keys: HashMap<ParticipantId, PublicKey> = roster
        .iter()
        .map(|(id, hex_key)| {
            let bytes = wire::hex_decode(hex_key).expect("roster pubkey must be valid hex");
            let arr: [u8; 32] = bytes.try_into().expect("pubkey must be 32 bytes");
            (*id, PublicKey::from(arr))
        })
        .collect();

    // ---- Commit round ---------------------------------------------------
    let dealer = Dealer::new(threshold, n);
    send(
        &mut write,
        &OperatorMessage::CommitmentHash {
            hash: wire::hex_encode(&dealer.commitment_hash()),
        },
    )
    .await;

    let expected_hashes: BTreeMap<String, String> = loop {
        match next_coordinator_message(&mut read).await {
            Some(CoordinatorMessage::AllCommitmentsReceived { hashes }) => break hashes,
            Some(CoordinatorMessage::CeremonyFailed { reason }) => {
                eprintln!("ceremony failed before reveal: {reason}");
                std::process::exit(1);
            }
            Some(_) => continue,
            None => {
                eprintln!("coordinator closed the connection during the commit round");
                std::process::exit(1);
            }
        }
    };

    // ---- Reveal round -----------------------------------------------------
    let (my_commitments, my_shares) = dealer.reveal();
    let sealed_shares: BTreeMap<String, String> = my_shares
        .iter()
        .map(|(recipient, share)| {
            let recipient_key = peer_public_keys.get(recipient).unwrap_or_else(|| {
                panic!("no public key on the roster for participant {recipient}")
            });
            let sealed = crypto::seal(recipient_key, &share.to_bytes());
            (recipient.to_string(), base64_encode(&sealed))
        })
        .collect();

    send(
        &mut write,
        &OperatorMessage::Reveal(RevealPayload {
            commitments: my_commitments.iter().map(wire::point_to_hex).collect(),
            sealed_shares,
        }),
    )
    .await;

    // ---- Collect every dealer's reveal, verify, finalize -----------------
    let mut dealer_c0s: BTreeMap<ParticipantId, curve25519_dalek::ristretto::RistrettoPoint> =
        BTreeMap::new();
    let mut my_verified_shares: BTreeMap<ParticipantId, curve25519_dalek::scalar::Scalar> =
        BTreeMap::new();

    while my_verified_shares.len() < n as usize {
        let Some(msg) = next_coordinator_message(&mut read).await else {
            eprintln!("coordinator closed the connection during the reveal round");
            std::process::exit(1);
        };
        let CoordinatorMessage::RevealBroadcast { from, payload } = msg else {
            if let CoordinatorMessage::CeremonyFailed { reason } = msg {
                eprintln!("ceremony failed during reveal: {reason}");
                std::process::exit(1);
            }
            continue;
        };
        if my_verified_shares.contains_key(&from) {
            continue;
        }

        let commitments: Vec<curve25519_dalek::ristretto::RistrettoPoint> = payload
            .commitments
            .iter()
            .map(|h| wire::point_from_hex(h).expect("dealer commitment must be a valid point"))
            .collect();

        // The whole point of the commit-then-reveal ordering (module doc,
        // `Dealer`'s own docs): a dealer who picked coefficients *after*
        // seeing everyone else's would have to also fabricate a matching
        // hash before anyone else committed — this is where that gets
        // checked, not merely trusted because the coordinator relayed it.
        let actual_hash = wire::hex_encode(&commitment_hash(&commitments));
        let expected_hash = expected_hashes.get(&from.to_string());
        if expected_hash != Some(&actual_hash) {
            report_failure(
                &mut write,
                format!(
                    "dealer {from}'s revealed commitments do not match its earlier committed hash \
                     — possible rushing-bias attempt"
                ),
            )
            .await;
            std::process::exit(1);
        }

        let Some(sealed_b64) = payload.sealed_shares.get(&my_id.to_string()) else {
            report_failure(
                &mut write,
                format!("dealer {from} sent no share for participant {my_id}"),
            )
            .await;
            std::process::exit(1);
        };
        let sealed_bytes = base64_decode(sealed_b64).unwrap_or_else(|| {
            eprintln!("dealer {from}'s sealed share for me was not valid base64");
            std::process::exit(1);
        });
        let Some(opened) = crypto::open(&identity, &sealed_bytes) else {
            report_failure(
                &mut write,
                format!("could not open dealer {from}'s sealed share — wrong key or tampered"),
            )
            .await;
            std::process::exit(1);
        };
        let share_bytes: [u8; 32] = opened.as_slice().try_into().unwrap_or_else(|_| {
            eprintln!("dealer {from}'s opened share was not 32 bytes");
            std::process::exit(1);
        });
        let Some(share) =
            curve25519_dalek::scalar::Scalar::from_canonical_bytes(share_bytes).into_option()
        else {
            report_failure(
                &mut write,
                format!("dealer {from}'s share did not decode to a canonical scalar"),
            )
            .await;
            std::process::exit(1);
        };

        if !verify_share(&commitments, my_id, &share) {
            report_failure(
                &mut write,
                format!("dealer {from} sent me a share that fails Feldman verification"),
            )
            .await;
            std::process::exit(1);
        }

        dealer_c0s.insert(from, commitments[0]);
        my_verified_shares.insert(from, share);
    }

    let secret_share = my_verified_shares
        .values()
        .fold(curve25519_dalek::scalar::Scalar::ZERO, |acc, s| acc + s);
    let group_public_key = dealer_c0s.values().fold(
        curve25519_dalek::ristretto::RistrettoPoint::default(),
        |acc, c| acc + c,
    );
    let key_share = KeyShare {
        participant_id: my_id,
        secret_share,
        group_public_key,
    };
    // Cross-check against the library's own summation to catch a mistake in
    // the manual folds above rather than trust them silently.
    debug_assert_eq!(
        finalize_key_share(
            my_id,
            &my_verified_shares.values().copied().collect::<Vec<_>>(),
            &dealer_c0s.values().copied().collect::<Vec<_>>(),
        )
        .group_public_key,
        key_share.group_public_key
    );

    send(
        &mut write,
        &OperatorMessage::CeremonyAck {
            group_public_key: wire::point_to_hex(&key_share.group_public_key),
        },
    )
    .await;

    loop {
        match next_coordinator_message(&mut read).await {
            Some(CoordinatorMessage::CeremonyComplete {
                group_public_key: confirmed,
            }) => {
                let expected = wire::point_to_hex(&key_share.group_public_key);
                if confirmed != expected {
                    eprintln!(
                        "coordinator confirmed a different group public key than I computed — aborting"
                    );
                    std::process::exit(1);
                }
                break;
            }
            Some(CoordinatorMessage::CeremonyFailed { reason }) => {
                eprintln!("ceremony failed: {reason}");
                std::process::exit(1);
            }
            Some(_) => continue,
            None => {
                eprintln!("coordinator closed the connection before confirming the ceremony");
                std::process::exit(1);
            }
        }
    }

    println!(
        "ceremony complete — group public key {}. Waiting for demo requests.",
        wire::point_to_hex(&key_share.group_public_key)
    );

    // ---- Steady state: answer decrypt/sign requests ------------------------
    let mut pending_nonces: Option<SecretNonces> = None;
    while let Some(msg) = next_coordinator_message(&mut read).await {
        match msg {
            CoordinatorMessage::PartialDecryptRequest { ephemeral_point } => {
                let point =
                    wire::point_from_hex(&ephemeral_point).expect("ephemeral point must be valid");
                let partial = novachannel_mpc::partial_decrypt(&key_share, &point);
                send(
                    &mut write,
                    &OperatorMessage::PartialDecryptResponse {
                        point: wire::point_to_hex(&partial),
                    },
                )
                .await;
            }
            CoordinatorMessage::Round1Request => {
                let (nonces, commitment) = round1_commit(my_id);
                pending_nonces = Some(nonces);
                send(
                    &mut write,
                    &OperatorMessage::Round1Response(SigningCommitmentWire {
                        participant_id: commitment.participant_id,
                        hiding: wire::point_to_hex(&commitment.hiding),
                        binding: wire::point_to_hex(&commitment.binding),
                    }),
                )
                .await;
            }
            CoordinatorMessage::Round2Request {
                message,
                signer_ids,
                commitments,
            } => {
                let Some(nonces) = pending_nonces.take() else {
                    eprintln!(
                        "received Round2Request with no outstanding Round1 nonces — ignoring"
                    );
                    continue;
                };
                let commitments: Vec<novachannel_mpc::frost::SigningCommitment> = commitments
                    .iter()
                    .map(|c| novachannel_mpc::frost::SigningCommitment {
                        participant_id: c.participant_id,
                        hiding: wire::point_from_hex(&c.hiding)
                            .expect("commitment point must be valid"),
                        binding: wire::point_from_hex(&c.binding)
                            .expect("commitment point must be valid"),
                    })
                    .collect();
                let z = round2_sign(
                    &key_share,
                    nonces,
                    message.as_bytes(),
                    &signer_ids,
                    &commitments,
                );
                send(
                    &mut write,
                    &OperatorMessage::Round2Response {
                        z: wire::scalar_to_hex(&z),
                    },
                )
                .await;
            }
            CoordinatorMessage::CeremonyFailed { reason } => {
                eprintln!("coordinator reported a ceremony failure: {reason}");
            }
            _ => {}
        }
    }

    println!("coordinator connection closed; exiting");
}

async fn report_failure<S>(write: &mut S, reason: String)
where
    S: SinkExt<WsMessage> + Unpin,
    S::Error: std::fmt::Debug,
{
    eprintln!("{reason}");
    let _ = send(write, &OperatorMessage::CeremonyFailed { reason }).await;
}

async fn send<S>(write: &mut S, msg: &OperatorMessage)
where
    S: SinkExt<WsMessage> + Unpin,
    S::Error: std::fmt::Debug,
{
    let json = serde_json::to_string(msg).expect("OperatorMessage always serializes");
    write
        .send(WsMessage::Text(json.into()))
        .await
        .expect("send to coordinator");
}

async fn next_coordinator_message<S>(read: &mut S) -> Option<CoordinatorMessage>
where
    S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = match read.next().await {
            None => return None,
            Some(Err(e)) => {
                eprintln!("coordinator connection error: {e}");
                return None;
            }
            Some(Ok(m)) => m,
        };
        match msg {
            WsMessage::Text(text) => {
                let parsed = serde_json::from_str(&text);
                if parsed.is_err() {
                    eprintln!("coordinator sent a frame this operator could not parse: {text}");
                }
                return parsed.ok();
            }
            WsMessage::Close(_) => return None,
            _ => continue,
        }
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins `commitment_hash()` against the real `Dealer::commitment_hash()`
    /// it exists to recompute — a byte-layout mismatch here would silently
    /// make every reveal fail the anti-rushing-bias check (or, worse,
    /// silently never check anything if both sides happened to agree on a
    /// wrong hash).
    #[test]
    fn dealer_reveal_hash_matches_recomputed_hash() {
        let dealer = Dealer::new(3, 5);
        let expected = dealer.commitment_hash();
        let (commitments, _shares) = dealer.reveal();
        assert_eq!(commitment_hash(&commitments), expected);
    }

    #[test]
    fn a_tampered_commitment_produces_a_different_hash() {
        let dealer = Dealer::new(3, 5);
        let expected = dealer.commitment_hash();
        let (mut commitments, _shares) = dealer.reveal();
        commitments[0] += curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
        assert_ne!(commitment_hash(&commitments), expected);
    }
}
