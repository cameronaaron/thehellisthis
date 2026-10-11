//! A standalone `nova` MPC/FROST operator: one participant in a real
//! `t`-of-`n` Feldman DKG and FROST threshold signature, in its own process so
//! the chat server never holds its share. The design (a hub-and-spoke relay,
//! shares sealed so the relay can forward but not read them) is in the main
//! crate's `session/nova_operator.rs`.
//!
//! Run one per participant, ideally on separate machines:
//!
//! ```text
//! nova-operator --server wss://thehellisthis.com/ws/nova-operator \
//!   --token "$NOVA_OPERATOR_TOKEN" \
//!   --identity ~/.nova-operator/identity.key
//! ```
//!
//! `--identity` defaults to `~/.nova-operator/identity.key`: this operator's
//! long-term X25519 identity, generated on first run and reused so the roster
//! needn't be redistributed. It is not the DKG share, which lives only in this
//! process's memory and is never written to disk. The identity file is the
//! project's one deliberate exception to keeping nothing on disk.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use nova_operator::protocol::{
    CoordinatorMessage, OperatorMessage, RevealPayload, SigningCommitmentWire,
};
use nova_operator::{crypto, wire};
use novachannel_mpc::frost::{SecretNonces, round1_commit, round2_sign};
use novachannel_mpc::{Dealer, KeyShare, ParticipantId, finalize_key_share, verify_share};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use x25519_dalek::{PublicKey, StaticSecret};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, WsMessage>;
type WsRead = SplitStream<WsStream>;

/// Starting backoff, and its doubling ceiling, for reconnects after the
/// ceremony (module doc). Only reached after a first successful connection;
/// a coordinator that stays unreachable after that is worth backing off from.
const RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Argument parsing without `parse_args`'s `std::process::exit`, which never
/// returns and would end the test binary (as `main_inner` in the main crate
/// returns an `ExitCode`).
fn parse_args_from<I: Iterator<Item = String>>(
    mut args: I,
) -> Result<(String, String, PathBuf), String> {
    let mut server = None;
    let mut token = None;
    let mut identity = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = args.next(),
            "--token" => token = args.next(),
            "--identity" => identity = args.next().map(PathBuf::from),
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let server = server.ok_or("--server <ws(s)://host/ws/nova-operator> is required")?;
    let token = token.ok_or(
        "--token <bearer token> is required (must match the coordinator's NOVA_OPERATOR_TOKEN)",
    )?;
    let identity = identity.unwrap_or_else(default_identity_path);

    Ok((server, token, identity))
}

fn parse_args() -> (String, String, PathBuf) {
    parse_args_from(std::env::args().skip(1)).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    })
}

fn identity_path_for(home: &str) -> PathBuf {
    Path::new(home).join(".nova-operator").join("identity.key")
}

fn default_identity_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    identity_path_for(&home)
}

/// Loads the identity keypair from `path`, or generates and saves one. Mode
/// `0600`: it is a long-term secret for this user only.
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

/// Opens one WebSocket to the coordinator with the bearer token. Returns an
/// error rather than panicking: `main` treats the first failure as fatal, but
/// post-ceremony reconnects must retry.
async fn connect_ws(server: &str, token: &str) -> Result<WsStream, String> {
    let mut request = server
        .into_client_request()
        .map_err(|e| format!("invalid --server URL: {e}"))?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| e.to_string())?,
    );
    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ws_stream)
}

/// Reconnects after the ceremony, with capped exponential backoff, until the
/// coordinator confirms this operator as `my_id`
/// (`CoordinatorMessage::Reconnected`). It doesn't return until it succeeds;
/// a coordinator down for a while is worth waiting out.
async fn reconnect_until_confirmed(
    server: &str,
    token: &str,
    my_static_public: &PublicKey,
    my_id: ParticipantId,
) -> (WsWrite, WsRead) {
    let mut backoff = RECONNECT_INITIAL_BACKOFF;
    loop {
        let stream = match connect_ws(server, token).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("reconnect failed: {e}; retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX_BACKOFF);
                continue;
            }
        };
        let (mut write, mut read) = stream.split();
        send(
            &mut write,
            &OperatorMessage::Hello {
                static_public_key: wire::hex_encode(my_static_public.as_bytes()),
            },
        )
        .await;

        let confirmed = loop {
            match next_coordinator_message(&mut read).await {
                Some(CoordinatorMessage::Reconnected { participant_id }) => {
                    break Some(participant_id);
                }
                Some(_) => continue,
                None => break None,
            }
        };

        match confirmed {
            Some(id) if id == my_id => {
                println!("reconnected as participant {my_id}");
                return (write, read);
            }
            Some(other) => eprintln!(
                "coordinator confirmed the wrong participant id ({other}, expected {my_id}) \
                 — retrying"
            ),
            None => {
                eprintln!("coordinator closed the connection during reconnect — retrying");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX_BACKOFF);
    }
}

#[tokio::main]
async fn main() {
    let (server, token, identity_path) = parse_args();
    let identity = load_or_create_identity(&identity_path);
    let my_static_public = PublicKey::from(&identity);

    let ws_stream = connect_ws(&server, &token)
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
        match msg {
            CoordinatorMessage::Welcome {
                participant_id,
                threshold,
                roster,
            } => break (participant_id, threshold, roster),
            // The coordinator recognised a participant from a completed
            // ceremony, but this is a cold start with no `KeyShare` in memory:
            // the identity file survived a restart and the share didn't. Only
            // `reconnect_until_confirmed` may legitimately see `Reconnected`.
            // Fail clearly instead of waiting for a `Welcome` that won't come.
            CoordinatorMessage::Reconnected { participant_id } => {
                eprintln!(
                    "the coordinator recognized this identity as participant {participant_id} \
                     from an already-completed ceremony, but this process has no KeyShare in \
                     memory to serve as that participant — the whole server needs restarting \
                     for a fresh ceremony"
                );
                std::process::exit(1);
            }
            _ => continue,
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

    // ---- Collect every dealer's reveal, checking hash and share as each
    // arrives. A hash mismatch is excluded silently: both values are public,
    // so every honest party reaches the same exclusion. A bad share only its
    // recipient can see becomes a `Complaint` revealing that one share so
    // others can verify it (module doc). ------------------------------------
    let mut dealer_c0s: BTreeMap<ParticipantId, curve25519_dalek::ristretto::RistrettoPoint> =
        BTreeMap::new();
    let mut my_shares_by_dealer: BTreeMap<ParticipantId, curve25519_dalek::scalar::Scalar> =
        BTreeMap::new();
    let mut excluded_dealers: std::collections::BTreeSet<ParticipantId> =
        std::collections::BTreeSet::new();
    let mut received: std::collections::BTreeSet<ParticipantId> = std::collections::BTreeSet::new();

    while (received.len() as u32) < n {
        let Some(msg) = next_coordinator_message(&mut read).await else {
            eprintln!("coordinator closed the connection during the reveal round");
            std::process::exit(1);
        };
        match msg {
            CoordinatorMessage::RevealBroadcast { from, payload } => {
                if !received.insert(from) {
                    continue;
                }

                let Some(commitments) = payload
                    .commitments
                    .iter()
                    .map(|h| wire::point_from_hex(h))
                    .collect::<Option<Vec<_>>>()
                else {
                    report_failure(
                        &mut write,
                        format!("dealer {from}'s reveal had a malformed commitment"),
                    )
                    .await;
                    std::process::exit(1);
                };

                // This is what commit-then-reveal is for: a dealer choosing
                // coefficients after seeing others' would also need a matching
                // hash committed before anyone else. Both sides are public, so
                // exclusion is silent.
                let actual_hash = wire::hex_encode(&wire::commitment_hash(&commitments));
                let expected_hash = expected_hashes.get(&from.to_string());
                if expected_hash != Some(&actual_hash) {
                    eprintln!(
                        "dealer {from}'s revealed commitments do not match its earlier \
                         committed hash — excluding (possible rushing-bias attempt)"
                    );
                    excluded_dealers.insert(from);
                    continue;
                }

                let Some(sealed_b64) = payload.sealed_shares.get(&my_id.to_string()) else {
                    report_failure(
                        &mut write,
                        format!("dealer {from} sent no share for participant {my_id}"),
                    )
                    .await;
                    std::process::exit(1);
                };
                let Some(sealed_bytes) = base64_decode(sealed_b64) else {
                    report_failure(
                        &mut write,
                        format!("dealer {from}'s sealed share for me was not valid base64"),
                    )
                    .await;
                    std::process::exit(1);
                };
                let Some(opened) = crypto::open(&identity, &sealed_bytes) else {
                    report_failure(
                        &mut write,
                        format!(
                            "could not open dealer {from}'s sealed share — wrong key or tampered"
                        ),
                    )
                    .await;
                    std::process::exit(1);
                };
                let Ok(share_bytes) = <[u8; 32]>::try_from(opened.as_slice()) else {
                    report_failure(
                        &mut write,
                        format!("dealer {from}'s opened share was not 32 bytes"),
                    )
                    .await;
                    std::process::exit(1);
                };
                let Some(share) =
                    curve25519_dalek::scalar::Scalar::from_canonical_bytes(share_bytes)
                        .into_option()
                else {
                    report_failure(
                        &mut write,
                        format!("dealer {from}'s share did not decode to a canonical scalar"),
                    )
                    .await;
                    std::process::exit(1);
                };

                if !verify_share(&commitments, my_id, &share) {
                    eprintln!(
                        "dealer {from} sent me a share that fails Feldman verification — \
                         complaining and excluding"
                    );
                    excluded_dealers.insert(from);
                    send(
                        &mut write,
                        &OperatorMessage::Complaint {
                            against_dealer: from,
                            disputed_share: wire::scalar_to_hex(&share),
                        },
                    )
                    .await;
                    continue;
                }

                dealer_c0s.insert(from, commitments[0]);
                my_shares_by_dealer.insert(from, share);
            }
            CoordinatorMessage::ComplaintBroadcast { against_dealer, .. } => {
                excluded_dealers.insert(against_dealer);
            }
            CoordinatorMessage::CeremonyFailed { reason } => {
                eprintln!("ceremony failed during reveal: {reason}");
                std::process::exit(1);
            }
            _ => continue,
        }
    }

    send(&mut write, &OperatorMessage::NoMoreComplaints).await;

    // Everyone sends `NoMoreComplaints` after its reveals; waiting for
    // the coordinator to close the window, not just for this operator
    // to finish, makes sure late complaints count before anyone
    // finalises.
    loop {
        match next_coordinator_message(&mut read).await {
            Some(CoordinatorMessage::ComplaintBroadcast { against_dealer, .. }) => {
                excluded_dealers.insert(against_dealer);
            }
            Some(CoordinatorMessage::ComplaintWindowClosed) => break,
            Some(CoordinatorMessage::CeremonyFailed { reason }) => {
                eprintln!("ceremony failed: {reason}");
                std::process::exit(1);
            }
            Some(_) => continue,
            None => {
                eprintln!(
                    "coordinator closed the connection while waiting for the complaint \
                     window to close"
                );
                std::process::exit(1);
            }
        }
    }

    if !excluded_dealers.is_empty() {
        println!(
            "excluding {} dealer(s) from the group key: {excluded_dealers:?}",
            excluded_dealers.len()
        );
    }

    let secret_share = my_shares_by_dealer
        .iter()
        .filter(|(id, _)| !excluded_dealers.contains(id))
        .map(|(_, s)| *s)
        .fold(curve25519_dalek::scalar::Scalar::ZERO, |acc, s| acc + s);
    let group_public_key = dealer_c0s
        .iter()
        .filter(|(id, _)| !excluded_dealers.contains(id))
        .map(|(_, c)| *c)
        .fold(
            curve25519_dalek::ristretto::RistrettoPoint::default(),
            |acc, c| acc + c,
        );
    let key_share = KeyShare {
        participant_id: my_id,
        secret_share,
        group_public_key,
    };
    // Cross-check against the library's own sum, meaningful only when
    // nothing was excluded: `finalize_key_share` takes no exclusions,
    // and `finalize_key_share_excluding_faulty` needs the full share
    // matrix this operator never sees.
    if excluded_dealers.is_empty() {
        debug_assert_eq!(
            finalize_key_share(
                my_id,
                &my_shares_by_dealer.values().copied().collect::<Vec<_>>(),
                &dealer_c0s.values().copied().collect::<Vec<_>>(),
            )
            .group_public_key,
            key_share.group_public_key
        );
    }

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

    // ---- Steady state, reconnecting on a drop ----------------------------
    // The ceremony runs once; this loop lets a network blip after it pass
    // without a restart, since the `KeyShare` stays in this stack frame.
    loop {
        run_steady_state(&mut write, &mut read, &key_share, my_id).await;
        println!("connection to coordinator lost; attempting to reconnect...");
        let (w, r) = reconnect_until_confirmed(&server, &token, &my_static_public, my_id).await;
        write = w;
        read = r;
    }
}

/// Answers decrypt and sign requests until the connection drops, then
/// returns so `main` can reconnect with the same `key_share`.
///
/// Generic over the `SinkExt`/`StreamExt` bounds rather than the concrete
/// socket halves, as the main crate's connection tasks are, so a test can
/// drive it with an in-memory stream.
async fn run_steady_state<W, R>(
    write: &mut W,
    read: &mut R,
    key_share: &KeyShare,
    my_id: ParticipantId,
) where
    W: SinkExt<WsMessage> + Unpin,
    W::Error: std::fmt::Debug,
    R: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut pending_nonces: Option<SecretNonces> = None;
    while let Some(msg) = next_coordinator_message(read).await {
        match msg {
            CoordinatorMessage::PartialDecryptRequest { ephemeral_point } => {
                let point =
                    wire::point_from_hex(&ephemeral_point).expect("ephemeral point must be valid");
                let partial = novachannel_mpc::partial_decrypt(key_share, &point);
                send(
                    write,
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
                    write,
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
                    key_share,
                    nonces,
                    message.as_bytes(),
                    &signer_ids,
                    &commitments,
                );
                send(
                    write,
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

    /// Pins `commitment_hash()` to the real `Dealer::commitment_hash()`: a byte
    /// layout mismatch would fail every reveal's rushing-bias check, or, if both
    /// sides agreed on a wrong hash, check nothing.
    #[test]
    fn dealer_reveal_hash_matches_recomputed_hash() {
        let dealer = Dealer::new(3, 5);
        let expected = dealer.commitment_hash();
        let (commitments, _shares) = dealer.reveal();
        assert_eq!(wire::commitment_hash(&commitments), expected);
    }

    #[test]
    fn a_tampered_commitment_produces_a_different_hash() {
        let dealer = Dealer::new(3, 5);
        let expected = dealer.commitment_hash();
        let (mut commitments, _shares) = dealer.reveal();
        commitments[0] += curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
        assert_ne!(wire::commitment_hash(&commitments), expected);
    }

    #[test]
    fn parse_args_from_reads_server_token_and_identity() {
        let args = [
            "--server",
            "wss://x",
            "--token",
            "tok",
            "--identity",
            "/tmp/id",
        ]
        .into_iter()
        .map(String::from);
        let (server, token, identity) = parse_args_from(args).expect("all required flags given");
        assert_eq!(server, "wss://x");
        assert_eq!(token, "tok");
        assert_eq!(identity, PathBuf::from("/tmp/id"));
    }

    #[test]
    fn parse_args_from_defaults_identity_when_omitted() {
        let args = ["--server", "wss://x", "--token", "tok"]
            .into_iter()
            .map(String::from);
        let (_, _, identity) = parse_args_from(args).expect("required flags given");
        assert_eq!(identity, default_identity_path());
    }

    #[test]
    fn parse_args_from_rejects_missing_server() {
        let args = ["--token", "tok"].into_iter().map(String::from);
        assert!(parse_args_from(args).unwrap_err().contains("--server"));
    }

    #[test]
    fn parse_args_from_rejects_missing_token() {
        let args = ["--server", "wss://x"].into_iter().map(String::from);
        assert!(parse_args_from(args).unwrap_err().contains("--token"));
    }

    #[test]
    fn parse_args_from_rejects_an_unknown_flag() {
        let args = ["--bogus"].into_iter().map(String::from);
        assert!(
            parse_args_from(args)
                .unwrap_err()
                .contains("unknown argument")
        );
    }

    #[test]
    fn identity_path_for_joins_home_and_the_default_filename() {
        assert_eq!(
            identity_path_for("/home/x"),
            PathBuf::from("/home/x/.nova-operator/identity.key")
        );
    }

    #[test]
    fn load_or_create_identity_generates_and_persists_a_new_key() {
        let dir =
            std::env::temp_dir().join(format!("nova-operator-test-gen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("identity.key");

        let secret = load_or_create_identity(&path);
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "identity file must not be group/world readable"
            );
        }

        let reloaded = load_or_create_identity(&path);
        assert_eq!(
            secret.to_bytes(),
            reloaded.to_bytes(),
            "a second load must return the same persisted key, not generate a new one"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[should_panic(expected = "is not a 32-byte key")]
    fn load_or_create_identity_panics_on_a_malformed_file() {
        let dir = std::env::temp_dir().join(format!(
            "nova-operator-test-malformed-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("identity.key");
        fs::write(&path, b"too short").unwrap();

        load_or_create_identity(&path);
    }

    #[test]
    fn base64_round_trips() {
        let data = b"nova operator round trip";
        let encoded = base64_encode(data);
        assert_eq!(base64_decode(&encoded).expect("valid base64"), data);
    }

    #[test]
    fn base64_decode_rejects_invalid_input() {
        assert_eq!(base64_decode("not valid base64!!"), None);
    }

    #[tokio::test]
    async fn next_coordinator_message_parses_a_valid_frame() {
        let json = serde_json::to_string(&CoordinatorMessage::ComplaintWindowClosed).unwrap();
        let mut stream = futures_util::stream::iter(vec![Ok(WsMessage::Text(json.into()))]);
        let msg = next_coordinator_message(&mut stream).await;
        assert!(matches!(
            msg,
            Some(CoordinatorMessage::ComplaintWindowClosed)
        ));
    }

    #[tokio::test]
    async fn next_coordinator_message_returns_none_on_malformed_json() {
        let mut stream = futures_util::stream::iter(vec![Ok(WsMessage::Text("not json".into()))]);
        assert!(next_coordinator_message(&mut stream).await.is_none());
    }

    #[tokio::test]
    async fn next_coordinator_message_returns_none_on_close() {
        let mut stream = futures_util::stream::iter(vec![Ok(WsMessage::Close(None))]);
        assert!(next_coordinator_message(&mut stream).await.is_none());
    }

    #[tokio::test]
    async fn next_coordinator_message_skips_non_text_frames() {
        let json = serde_json::to_string(&CoordinatorMessage::ComplaintWindowClosed).unwrap();
        let mut stream = futures_util::stream::iter(vec![
            Ok(WsMessage::Ping(Vec::new().into())),
            Ok(WsMessage::Text(json.into())),
        ]);
        let msg = next_coordinator_message(&mut stream).await;
        assert!(matches!(
            msg,
            Some(CoordinatorMessage::ComplaintWindowClosed)
        ));
    }

    #[tokio::test]
    async fn next_coordinator_message_returns_none_on_a_stream_error() {
        let mut stream = futures_util::stream::iter(vec![Err(
            tokio_tungstenite::tungstenite::Error::AlreadyClosed,
        )]);
        assert!(next_coordinator_message(&mut stream).await.is_none());
    }

    #[tokio::test]
    async fn next_coordinator_message_returns_none_on_an_empty_stream() {
        let mut stream = futures_util::stream::iter(Vec::<
            Result<WsMessage, tokio_tungstenite::tungstenite::Error>,
        >::new());
        assert!(next_coordinator_message(&mut stream).await.is_none());
    }

    type MockSink = std::pin::Pin<
        Box<dyn futures_util::Sink<WsMessage, Error = std::convert::Infallible> + Send>,
    >;

    fn collecting_sink() -> (MockSink, std::sync::Arc<std::sync::Mutex<Vec<WsMessage>>>) {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sent_for_sink = sent.clone();
        let sink = futures_util::sink::unfold((), move |_, item: WsMessage| {
            let sent = sent_for_sink.clone();
            async move {
                sent.lock().unwrap().push(item);
                Ok::<(), std::convert::Infallible>(())
            }
        });
        (Box::pin(sink), sent)
    }

    #[tokio::test]
    async fn send_serializes_and_writes_one_frame() {
        let (mut sink, sent) = collecting_sink();
        send(&mut sink, &OperatorMessage::NoMoreComplaints).await;
        let frames = sent.lock().unwrap();
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            WsMessage::Text(t) => assert!(t.contains("NoMoreComplaints")),
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn report_failure_sends_a_ceremony_failed_frame_and_logs() {
        let (mut sink, sent) = collecting_sink();
        report_failure(&mut sink, "something went wrong".to_string()).await;
        let frames = sent.lock().unwrap();
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            WsMessage::Text(t) => {
                assert!(t.contains("CeremonyFailed"));
                assert!(t.contains("something went wrong"));
            }
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    fn test_key_share(my_id: ParticipantId) -> KeyShare {
        KeyShare {
            participant_id: my_id,
            secret_share: curve25519_dalek::scalar::Scalar::from(7u64),
            group_public_key: curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT,
        }
    }

    #[tokio::test]
    async fn run_steady_state_answers_a_partial_decrypt_request() {
        let key_share = test_key_share(1);
        let ephemeral = curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
        let request = CoordinatorMessage::PartialDecryptRequest {
            ephemeral_point: wire::point_to_hex(&ephemeral),
        };
        let json = serde_json::to_string(&request).unwrap();
        let mut read = futures_util::stream::iter(vec![Ok(WsMessage::Text(json.into()))]);
        let (mut write, sent) = collecting_sink();

        run_steady_state(&mut write, &mut read, &key_share, 1).await;

        let frames = sent.lock().unwrap();
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            WsMessage::Text(t) => assert!(t.contains("PartialDecryptResponse")),
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_steady_state_answers_round1_then_round2() {
        let key_share = test_key_share(1);
        let round2_request = CoordinatorMessage::Round2Request {
            message: "sign me".to_string(),
            signer_ids: vec![1],
            commitments: vec![],
        };
        let mut read = futures_util::stream::iter(vec![
            Ok(WsMessage::Text(
                serde_json::to_string(&CoordinatorMessage::Round1Request)
                    .unwrap()
                    .into(),
            )),
            Ok(WsMessage::Text(
                serde_json::to_string(&round2_request).unwrap().into(),
            )),
        ]);
        let (mut write, sent) = collecting_sink();

        run_steady_state(&mut write, &mut read, &key_share, 1).await;

        let frames = sent.lock().unwrap();
        assert_eq!(
            frames.len(),
            2,
            "expects a Round1Response then a Round2Response"
        );
        match &frames[0] {
            WsMessage::Text(t) => assert!(t.contains("Round1Response")),
            other => panic!("expected a text frame, got {other:?}"),
        }
        match &frames[1] {
            WsMessage::Text(t) => assert!(t.contains("Round2Response")),
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_steady_state_ignores_round2_with_no_outstanding_nonces() {
        let key_share = test_key_share(1);
        let round2_request = CoordinatorMessage::Round2Request {
            message: "sign me".to_string(),
            signer_ids: vec![1],
            commitments: vec![],
        };
        let mut read = futures_util::stream::iter(vec![Ok(WsMessage::Text(
            serde_json::to_string(&round2_request).unwrap().into(),
        ))]);
        let (mut write, sent) = collecting_sink();

        run_steady_state(&mut write, &mut read, &key_share, 1).await;

        assert!(
            sent.lock().unwrap().is_empty(),
            "a Round2Request with no prior Round1 must produce no reply"
        );
    }

    #[tokio::test]
    async fn run_steady_state_logs_and_continues_on_ceremony_failed() {
        let key_share = test_key_share(1);
        let mut read = futures_util::stream::iter(vec![
            Ok(WsMessage::Text(
                serde_json::to_string(&CoordinatorMessage::CeremonyFailed {
                    reason: "demo aborted".to_string(),
                })
                .unwrap()
                .into(),
            )),
            Ok(WsMessage::Text(
                serde_json::to_string(&CoordinatorMessage::PartialDecryptRequest {
                    ephemeral_point: wire::point_to_hex(
                        &curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT,
                    ),
                })
                .unwrap()
                .into(),
            )),
        ]);
        let (mut write, sent) = collecting_sink();

        run_steady_state(&mut write, &mut read, &key_share, 1).await;

        assert_eq!(
            sent.lock().unwrap().len(),
            1,
            "CeremonyFailed produces no reply of its own but must not stop the loop"
        );
    }

    #[tokio::test]
    async fn run_steady_state_ignores_unrelated_message_types_and_returns_when_the_stream_ends() {
        let key_share = test_key_share(1);
        let mut read = futures_util::stream::iter(vec![Ok(WsMessage::Text(
            serde_json::to_string(&CoordinatorMessage::ComplaintWindowClosed)
                .unwrap()
                .into(),
        ))]);
        let (mut write, sent) = collecting_sink();

        run_steady_state(&mut write, &mut read, &key_share, 1).await;

        assert!(sent.lock().unwrap().is_empty());
    }
}
