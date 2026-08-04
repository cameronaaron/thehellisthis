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
