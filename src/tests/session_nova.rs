//! End-to-end proof that the `nova` room's novachannel handshake and sealed
//! transport actually work: two real WebSocket clients, playing the
//! initiator side of the handshake themselves (the same steps `/nova.wasm`
//! will run in a browser), against the real server.
//!
//! Every other room's existing message tests keep passing unmodified
//! elsewhere in this suite — nothing here touches a non-`nova` code path.

use super::*;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use novachannel::{Identity, Opened, RatchetedSession, initiator_start};
// RLN's own proving path is exercised only by the release-only test below
// (see its doc comment for why) — these imports go with it.
#[cfg(not(debug_assertions))]
use novachannel_rln::merkle::{PathStep, Side};
#[cfg(not(debug_assertions))]
use novachannel_rln::permutation::{Params, compress2};
#[cfg(not(debug_assertions))]
use novachannel_rln::{air, bytes_to_field, epoch_field};
#[cfg(not(debug_assertions))]
use winterfell::math::StarkField;
#[cfg(not(debug_assertions))]
use winterfell::math::fields::f128::BaseElement;

/// Drains frames up to and including `ReconnectToken` — the end of the
/// history-replay preamble (§4.2's frame-order contract) — the same point
/// `client.js` waits for before sending its own first frame.
async fn drain_preamble(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    loop {
        let event = recv_json_event(ws).await;
        if event["type"] == "ReconnectToken" {
            return;
        }
    }
}

/// Runs the initiator side of the handshake over `ws`, returning the
/// established `RatchetedSession` a real client would use for every
/// message from here on.
async fn handshake(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> RatchetedSession {
    let local_identity = Identity::generate();
    let (init_state, msg1) = initiator_start(None);

    ws.send(text_frame(
        serde_json::json!({"type": "NovaHandshakeInit", "msg1": BASE64.encode(&msg1)}).to_string(),
    ))
    .await
    .expect("send NovaHandshakeInit");

    let response = loop {
        let event = recv_json_event(ws).await;
        if event["type"] == "NovaHandshakeResponse" {
            break event;
        }
    };
    let msg2 = BASE64
        .decode(response["msg2"].as_str().expect("msg2 is a string"))
        .expect("msg2 is valid base64");

    let (msg3, established) = init_state
        .complete(&local_identity, &msg2)
        .expect("handshake completes");

    ws.send(text_frame(
        serde_json::json!({"type": "NovaHandshakeComplete", "msg3": BASE64.encode(&msg3)})
            .to_string(),
    ))
    .await
    .expect("send NovaHandshakeComplete");

    RatchetedSession::new(&established, true)
}

/// Reads frames until a `Sealed` one arrives, opens it with `session`, and
/// returns the `ClientEvent`/`OutgoingEvent` JSON it decrypts to.
async fn recv_sealed(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    session: &mut RatchetedSession,
) -> JsonValue {
    loop {
        let event = recv_json_event(ws).await;
        if event["type"] != "Sealed" {
            continue;
        }
        let record = BASE64
            .decode(event["data"].as_str().expect("data is a string"))
            .expect("data is valid base64");
        match session.open(&record).expect("opens under our own ratchet") {
            Opened::Application(bytes) => {
                return serde_json::from_slice(&bytes).expect("sealed payload is JSON");
            }
            Opened::RatchetAdvanced { .. } => continue,
        }
    }
}

/// Seals `value` under `session` and sends it as a `Sealed` frame.
#[cfg(not(debug_assertions))]
async fn send_sealed(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    session: &mut RatchetedSession,
    value: &JsonValue,
) {
    let sealed = session.seal(value.to_string().as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed frame");
}

/// Like [`recv_sealed`], but skips any sealed frame whose `type` isn't
/// `expected` — a `nova` connection's other broadcasts (`UserCount`,
/// `UserJoined` from the second client connecting) are sealed exactly the
/// same way and can arrive interleaved with the reply a test is waiting
/// for.
#[cfg(not(debug_assertions))]
async fn recv_sealed_of_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    session: &mut RatchetedSession,
    expected: &str,
) -> JsonValue {
    loop {
        let event = recv_sealed(ws, session).await;
        if event["type"] == expected {
            return event;
        }
    }
}

/// The full round trip: two clients handshake independently, one sends a
/// sealed message, the other receives it sealed under its *own* ratchet and
/// recovers the same plaintext — proving the server actually decrypts with
/// the sender's session and re-encrypts per recipient, not just echoing
/// bytes through.
#[tokio::test]
async fn two_clients_handshake_and_exchange_a_sealed_message() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let mut session_b = handshake(&mut ws_b).await;

    // Give B's handshake a moment to actually land server-side before A
    // sends — `forward_broadcasts` drops a broadcast for any connection
    // whose handshake has not completed yet (session/nova.rs), which is
    // exactly the gap this sleep is standing in for.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let plaintext = serde_json::json!({"type": "Message", "text": "hello from nova"}).to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed message");

    let received = recv_sealed(&mut ws_b, &mut session_b).await;
    assert_eq!(received["type"], "Message");
    assert!(
        received["message"]["text"]
            .as_str()
            .expect("message.text is a string")
            .contains("hello from nova"),
        "unexpected payload: {received}"
    );

    handle.abort();
}

/// A message sent to `nova` in the clear — no `Sealed` wrapper — is
/// rejected outright, not silently accepted as an unsealed message. Once a
/// `nova` connection exists, `session/nova.rs::dispatch`'s catch-all arm is
/// the only thing standing between "the crypto layer is real" and "the
/// crypto layer is decorative", so this is the one negative case worth its
/// own test.
#[tokio::test]
async fn nova_rejects_a_plaintext_message_sent_outside_the_sealed_wrapper() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let mut session_b = handshake(&mut ws_b).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // The rejected plaintext frame, first.
    ws_a.send(text_frame(
        serde_json::json!({"type": "Message", "text": "not sealed"}).to_string(),
    ))
    .await
    .expect("send plaintext message");

    // A genuine sealed message, right behind it. If the plaintext one had
    // been accepted, it would broadcast first and this assertion would see
    // *it* rather than the sealed payload — so this doesn't just check that
    // something arrives, it checks that the plaintext frame specifically
    // produced nothing (§6.4: a test that can pass by waiting on a timeout
    // for something that must not happen proves nothing; racing a real
    // positive case behind it does).
    let plaintext =
        serde_json::json!({"type": "Message", "text": "this one is sealed"}).to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed message");

    let received = recv_sealed(&mut ws_b, &mut session_b).await;
    assert!(
        received["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("this one is sealed"),
        "the plaintext frame must never reach another connection: {received}"
    );

    handle.abort();
}

#[cfg(not(debug_assertions))]
fn field_to_hex(v: BaseElement) -> String {
    v.as_int()
        .to_be_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(not(debug_assertions))]
fn hex_to_field(hex: &str) -> BaseElement {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    BaseElement::new(u128::from_be_bytes(bytes.try_into().unwrap()))
}

#[cfg(not(debug_assertions))]
fn wire_path(path: &JsonValue) -> Vec<PathStep> {
    path.as_array()
        .expect("path is an array")
        .iter()
        .map(|step| PathStep {
            sibling: hex_to_field(step["sibling"].as_str().unwrap()),
            side: match step["side"].as_str().unwrap() {
                "Left" => Side::Left,
                "Right" => Side::Right,
                other => panic!("unexpected side {other}"),
            },
        })
        .collect()
}

/// Proves an RLN message for `text` at `sk`'s `leaf_index`/`path`, returning
/// the wire-shaped `RlnMessage` payload. Mirrors exactly what `nova-wasm`
/// does client-side (`nova-wasm/src/lib.rs` will grow the same steps).
#[cfg(not(debug_assertions))]
fn prove_rln_message(sk: BaseElement, path: Vec<PathStep>, epoch: u64, text: &str) -> JsonValue {
    let params = novachannel_rln::permutation::Params::new();
    let epoch_x = epoch_field(epoch);
    let message_x = bytes_to_field(text.as_bytes());
    let a1 = compress2(&params, sk, epoch_x);
    let y = sk + a1 * message_x;

    let witness = air::Witness { sk, path };
    let (proof, public) =
        air::prove(&witness, epoch_x, message_x, y, a1).expect("proving succeeds");

    serde_json::json!({
        "type": "RlnMessage",
        "proof": BASE64.encode(proof.to_bytes()),
        "y": field_to_hex(public.y),
        "nullifier": field_to_hex(public.nullifier),
        "text": text,
    })
}

#[cfg(not(debug_assertions))]
fn current_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / NOVA_RLN_EPOCH_SECONDS
}

/// The whole RLN path: two clients register, one proves membership and
/// posts anonymously — the other receives it with *no sender identity at
/// all* — and then a second, different anonymous message from the same
/// member in the same epoch recovers that member's identity secret, which
/// is RLN's namesake property, not a side effect.
///
/// `cfg(not(debug_assertions))` — release only, not `#[ignore]`: winterfell
/// runs an internal debug-only sanity check that the AIR's *declared*
/// transition constraint degrees exactly equal the *measured* degree for
/// the specific witness proved. `novachannel-rln`'s own crate docs name
/// this exact false positive (a few boundary-injection columns are sparse,
/// so their measured degree varies with which Merkle path a given leaf
/// produces, tripping the exact-match debug assertion with no actual
/// soundness issue) and its own test suite is documented to require
/// `--release` for the same reason. `ci.yml` runs this specific test under
/// `cargo test --release` in a dedicated step for exactly this reason —
/// see the step's own comment for why it isn't just folded into the
/// existing `cargo test --all-features` (debug) run.
#[cfg(not(debug_assertions))]
#[tokio::test]
async fn nova_rln_anonymous_post_and_double_post_slashing() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let mut session_b = handshake(&mut ws_b).await;

    // A registers an RLN identity.
    let params = Params::new();
    let rln_identity = novachannel_rln::Identity::generate();
    let commitment = rln_identity.commitment(&params);
    send_sealed(
        &mut ws_a,
        &mut session_a,
        &serde_json::json!({"type": "RlnRegister", "commitment": field_to_hex(commitment)}),
    )
    .await;
    let registered = recv_sealed_of_type(&mut ws_a, &mut session_a, "RlnRegistered").await;
    let leaf_index = registered["leaf_index"].as_u64().expect("leaf_index") as usize;

    // Fetches this leaf's current path (module doc in session/nova_rln.rs
    // on why this is a separate, fresh-every-time request).
    send_sealed(
        &mut ws_a,
        &mut session_a,
        &serde_json::json!({"type": "RlnPathRequest", "leaf_index": leaf_index}),
    )
    .await;
    let path_response = recv_sealed_of_type(&mut ws_a, &mut session_a, "RlnPathResponse").await;
    let path = wire_path(&path_response["path"]);

    let epoch = current_epoch();

    // First anonymous message: accepted, and carries no sender identity.
    let msg1 = prove_rln_message(rln_identity.sk, path.clone(), epoch, "hello anonymously");
    send_sealed(&mut ws_a, &mut session_a, &msg1).await;

    let received = recv_sealed_of_type(&mut ws_b, &mut session_b, "NovaAnonymousMessage").await;
    assert!(
        received["text"]
            .as_str()
            .unwrap_or_default()
            .contains("hello anonymously"),
        "unexpected payload: {received}"
    );
    assert!(
        received.get("user_id").is_none() && received.get("animal_name").is_none(),
        "an RLN-proven message must carry no sender identity at all: {received}"
    );

    // Second, *different* anonymous message from the same member in the
    // same epoch: the rate-limit violation RLN is named for. The path is
    // requested fresh again — the module doc's whole point.
    send_sealed(
        &mut ws_a,
        &mut session_a,
        &serde_json::json!({"type": "RlnPathRequest", "leaf_index": leaf_index}),
    )
    .await;
    let path_response = recv_sealed_of_type(&mut ws_a, &mut session_a, "RlnPathResponse").await;
    let path = wire_path(&path_response["path"]);

    let msg2 = prove_rln_message(rln_identity.sk, path, epoch, "a second, different message");
    send_sealed(&mut ws_a, &mut session_a, &msg2).await;

    let slashed = recv_sealed_of_type(&mut ws_b, &mut session_b, "NovaRlnSlashed").await;
    let recovered = hex_to_field(slashed["recovered_secret"].as_str().expect("hex"));
    assert_eq!(
        recovered, rln_identity.sk,
        "the recovered secret must be the actual offending member's sk"
    );

    handle.abort();
}
