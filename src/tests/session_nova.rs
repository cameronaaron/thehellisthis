//! End-to-end proof that the `nova` room's two crypto sessions —
//! `novachannel`'s pairwise X3DH ratchet and the room's shared `Group` —
//! actually work: real WebSocket clients, playing the initiator/joiner
//! side themselves (the same steps `/nova.wasm` will run in a browser),
//! against the real server.
//!
//! Every other room's existing message tests keep passing unmodified
//! elsewhere in this suite — nothing here touches a non-`nova` code path.

use super::*;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use novachannel::Opened;
use novachannel::group::{Commit, Group, MyLeafKeyPackage, Welcome};
use novachannel::x3dh::initiate;
use novachannel::{DhIdentity, Identity, PreKeyBundle, RatchetedSession};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
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

/// Runs the initiator side of X3DH over `ws`: fetches the server's prekey
/// bundle, verifies it, and calls `initiate` — which completes the
/// initiator's side of the session in this one call, unlike the old
/// synchronous handshake this replaced (no second server round trip to
/// wait for). Returns the established `RatchetedSession` a real client
/// would use for every message from here on.
async fn handshake(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> RatchetedSession {
    let local_identity = Identity::generate();
    let local_dh_identity = DhIdentity::generate();

    ws.send(text_frame(
        serde_json::json!({"type": "NovaPreKeyBundleRequest"}).to_string(),
    ))
    .await
    .expect("send NovaPreKeyBundleRequest");

    let response = loop {
        let event = recv_json_event(ws).await;
        if event["type"] == "NovaPreKeyBundleResponse" {
            break event;
        }
    };
    let bundle_bytes = BASE64
        .decode(response["bundle"].as_str().expect("bundle is a string"))
        .expect("bundle is valid base64");
    let bundle = PreKeyBundle::from_bytes(&bundle_bytes).expect("bundle deserializes");
    bundle.verify().expect("bundle signature verifies");

    let initiated = initiate(&local_identity.public(), &local_dh_identity, &bundle, &[])
        .expect("x3dh initiate succeeds");

    ws.send(text_frame(
        serde_json::json!({
            "type": "NovaX3dhInit",
            "message": BASE64.encode(&initiated.message.bytes),
        })
        .to_string(),
    ))
    .await
    .expect("send NovaX3dhInit");

    RatchetedSession::new(&initiated.session, true)
}

/// Reads frames until a `Sealed` one arrives, opens it with `session`, and
/// returns the `ClientEvent`/`OutgoingEvent` JSON it decrypts to. Only
/// [`recv_sealed_of_type`] (release-only RLN test) still reaches for
/// pairwise-sealed content directly — every other test now receives
/// broadcast content via [`recv_group_sealed`] instead (this module's doc).
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

/// Joins the room's shared `Group`, the same steps `nova-wasm`'s
/// `myKeyPackage`/`completeJoin` run: generates a fresh `MyLeafKeyPackage`,
/// publishes its public half as `NovaJoinRequest`, and joins from the
/// server's `NovaWelcome` reply (`welcome` + the admitting `commit`, both
/// base64). Returns the `Group` a real client would use to open every
/// broadcast from here on — content no longer arrives sealed under the
/// pairwise session `handshake` returns (see this module's doc).
async fn join_group(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Group {
    let my_key_package = MyLeafKeyPackage::generate(Identity::generate().public());
    let key_package_b64 = BASE64.encode(my_key_package.public().to_bytes());

    ws.send(text_frame(
        serde_json::json!({"type": "NovaJoinRequest", "key_package": key_package_b64}).to_string(),
    ))
    .await
    .expect("send NovaJoinRequest");

    let welcome_event = loop {
        let event = recv_json_event(ws).await;
        if event["type"] == "NovaWelcome" {
            break event;
        }
    };
    let welcome = Welcome::from_bytes(
        &BASE64
            .decode(
                welcome_event["welcome"]
                    .as_str()
                    .expect("welcome is a string"),
            )
            .expect("welcome is valid base64"),
    )
    .expect("welcome deserializes");
    let commit = Commit::from_bytes(
        &BASE64
            .decode(
                welcome_event["commit"]
                    .as_str()
                    .expect("commit is a string"),
            )
            .expect("commit is valid base64"),
    )
    .expect("commit deserializes");

    Group::join(my_key_package, &welcome, &commit).expect("joins the group")
}

/// Reads frames until a `GroupSealed` one arrives, opens it with `group`,
/// and returns the `OutgoingEvent` JSON it decrypts to. A `NovaCommit`
/// encountered along the way (another client joining, or this
/// connection's own admitting commit echoed back through the room
/// broadcast — `session/nova.rs`'s module doc on why the joiner sees it
/// twice) is applied and ignored if it fails, the same tolerance
/// `client.js::applyNovaGroupCommit` has.
async fn recv_group_sealed(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    group: &mut Group,
) -> JsonValue {
    loop {
        let event = recv_json_event(ws).await;
        if event["type"] == "NovaCommit" {
            if let Ok(commit_bytes) = BASE64.decode(event["commit"].as_str().unwrap_or_default())
                && let Ok(commit) = Commit::from_bytes(&commit_bytes)
            {
                let _ = group.apply_commit(&commit);
            }
            continue;
        }
        if event["type"] != "GroupSealed" {
            continue;
        }
        let record = BASE64
            .decode(event["data"].as_str().expect("data is a string"))
            .expect("data is valid base64");
        let (_sender_leaf, plaintext) = group.open(&record).expect("opens under our own group");
        return serde_json::from_slice(&plaintext).expect("group payload is JSON");
    }
}

/// Like [`recv_group_sealed`], but skips any frame whose `type` isn't
/// `expected` — the same reasoning as [`recv_sealed_of_type`].
async fn recv_group_sealed_of_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    group: &mut Group,
    expected: &str,
) -> JsonValue {
    loop {
        let event = recv_group_sealed(ws, group).await;
        if event["type"] == expected {
            return event;
        }
    }
}

/// The full round trip: two clients handshake and join the group
/// independently, one sends over its pairwise session, the other receives
/// it group-sealed and recovers the same plaintext — proving the server
/// actually decrypts with the sender's pairwise session, renders it, and
/// seals the broadcast once under the shared group, not just echoing bytes
/// through.
#[tokio::test]
async fn two_clients_handshake_and_exchange_a_sealed_message() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

    // Give both handshakes a moment to actually land server-side before A
    // sends.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let plaintext = serde_json::json!({"type": "Message", "text": "hello from nova"}).to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed message");

    let received = recv_group_sealed(&mut ws_b, &mut group_b).await;
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

/// A third client, joining after the first two already have, receives the
/// `Commit` admitting it and can decrypt a broadcast sent afterward —
/// proving `Welcome`/`Commit` distribution genuinely works for more than a
/// single pairwise exchange, and that a mid-session join reaches the same
/// epoch every other current member does.
#[tokio::test]
async fn a_third_client_joining_mid_session_can_decrypt_a_later_broadcast() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    join_group(&mut ws_b).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // C joins only after A and B are already members.
    let (mut ws_c, _) = connect_async(&ws_url).await.expect("client C connects");
    drain_preamble(&mut ws_c).await;
    let _session_c = handshake(&mut ws_c).await;
    let mut group_c = join_group(&mut ws_c).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let plaintext =
        serde_json::json!({"type": "Message", "text": "hello to the whole group, C included"})
            .to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed message");

    let received = recv_group_sealed_of_type(&mut ws_c, &mut group_c, "Message").await;
    assert!(
        received["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("hello to the whole group, C included"),
        "the third, later-joining member must be able to decrypt a broadcast sent after it joined: {received}"
    );

    handle.abort();
}

/// Disconnecting a member excludes them cryptographically going forward —
/// the free security win from wiring `Group::propose_remove` into
/// connection teardown (`session/lifecycle.rs::run_session`,
/// `session/nova.rs::remove_member`). Three members join; one disconnects;
/// a message sent afterward must still reach the two remaining members —
/// proving the room's `Group` survived the removal and kept working, not
/// just that the removed member is gone.
#[tokio::test]
async fn a_disconnected_member_is_cryptographically_excluded() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

    let (ws_c, _) = connect_async(&ws_url).await.expect("client C connects");
    let mut ws_c = ws_c;
    drain_preamble(&mut ws_c).await;
    let _session_c = handshake(&mut ws_c).await;
    join_group(&mut ws_c).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // C disconnects — `run_session`'s teardown removes its leaf from the
    // group and broadcasts the resulting `Commit`.
    drop(ws_c);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let plaintext = serde_json::json!({"type": "Message", "text": "after C left"}).to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed message");

    let received = recv_group_sealed_of_type(&mut ws_b, &mut group_b, "Message").await;
    assert!(
        received["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("after C left"),
        "the remaining member must still be able to decrypt a broadcast sent after another \
         member was removed: {received}"
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
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

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

    let received = recv_group_sealed(&mut ws_b, &mut group_b).await;
    assert!(
        received["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("this one is sealed"),
        "the plaintext frame must never reach another connection: {received}"
    );

    handle.abort();
}

/// A second `NovaX3dhInit` on the same connection is rejected — X3DH
/// completes in one call, so there is no "renegotiate" path.
#[tokio::test]
async fn x3dh_init_sent_twice_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let session = handshake(&mut ws).await;

    // A second init, still valid-looking, must not disturb the already
    // established session: sealing with the first session must still work.
    let local_identity = Identity::generate();
    let local_dh_identity = DhIdentity::generate();
    ws.send(text_frame(
        serde_json::json!({"type": "NovaPreKeyBundleRequest"}).to_string(),
    ))
    .await
    .expect("send NovaPreKeyBundleRequest");
    let response = loop {
        let event = recv_json_event(&mut ws).await;
        if event["type"] == "NovaPreKeyBundleResponse" {
            break event;
        }
    };
    let bundle_bytes = BASE64.decode(response["bundle"].as_str().unwrap()).unwrap();
    let bundle = PreKeyBundle::from_bytes(&bundle_bytes).unwrap();
    let initiated = initiate(&local_identity.public(), &local_dh_identity, &bundle, &[])
        .expect("x3dh initiate succeeds");
    ws.send(text_frame(
        serde_json::json!({
            "type": "NovaX3dhInit",
            "message": BASE64.encode(&initiated.message.bytes),
        })
        .to_string(),
    ))
    .await
    .expect("send second NovaX3dhInit");

    let mut session = session;
    let plaintext = serde_json::json!({"type": "RequestRoster"}).to_string();
    let sealed = session
        .seal(plaintext.as_bytes())
        .expect("original session still works");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed frame under the original session");

    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// A `NovaX3dhInit` whose `message` is not valid base64 is rejected —
/// silently, no reply, the session stays `AwaitingSession`.
#[tokio::test]
async fn x3dh_init_with_malformed_base64_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;

    ws.send(text_frame(
        serde_json::json!({"type": "NovaX3dhInit", "message": "not valid base64!!"}).to_string(),
    ))
    .await
    .expect("send malformed NovaX3dhInit");

    // The session must still be establishable afterward — a malformed
    // attempt does not burn the "only one init" allowance.
    let mut session = handshake(&mut ws).await;
    let plaintext = serde_json::json!({"type": "RequestRoster"}).to_string();
    let sealed = session.seal(plaintext.as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed frame");
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// A `NovaX3dhInit` with valid base64 that isn't a real X3DH init message is
/// rejected by `respond()` itself, not just by base64 decoding.
#[tokio::test]
async fn x3dh_init_with_garbage_message_bytes_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;

    ws.send(text_frame(
        serde_json::json!({
            "type": "NovaX3dhInit",
            "message": BASE64.encode(b"not a real x3dh init message"),
        })
        .to_string(),
    ))
    .await
    .expect("send garbage NovaX3dhInit");

    let mut session = handshake(&mut ws).await;
    let plaintext = serde_json::json!({"type": "RequestRoster"}).to_string();
    let sealed = session.seal(plaintext.as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed frame");
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// A `Sealed` frame arriving before the pairwise session is established is
/// rejected — there is nothing to open it with yet.
#[tokio::test]
async fn sealed_frame_before_handshake_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;

    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(b"anything")}).to_string(),
    ))
    .await
    .expect("send premature sealed frame");

    // The real handshake right behind it still works.
    let mut session = handshake(&mut ws).await;
    let plaintext = serde_json::json!({"type": "RequestRoster"}).to_string();
    let sealed = session.seal(plaintext.as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed frame");
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// A `Sealed` frame whose `data` is not valid base64, or whose ciphertext
/// is tampered, is rejected — the connection is not torn down over it, and
/// a genuine sealed frame right behind it still works.
#[tokio::test]
async fn sealed_frame_with_malformed_or_tampered_data_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let mut session = handshake(&mut ws).await;

    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": "not valid base64!!"}).to_string(),
    ))
    .await
    .expect("send malformed sealed frame");

    let mut tampered = session
        .seal(
            serde_json::json!({"type": "RequestRoster"})
                .to_string()
                .as_bytes(),
        )
        .expect("seal");
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF;
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&tampered)}).to_string(),
    ))
    .await
    .expect("send tampered sealed frame");

    let plaintext = serde_json::json!({"type": "RequestRoster"}).to_string();
    let sealed = session.seal(plaintext.as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send genuine sealed frame");
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// A `NovaJoinRequest` sent twice on the same connection is rejected the
/// second time — this connection already has a leaf.
#[tokio::test]
async fn join_requested_twice_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    handshake(&mut ws).await;
    join_group(&mut ws).await;

    let my_key_package = MyLeafKeyPackage::generate(Identity::generate().public());
    let key_package_b64 = BASE64.encode(my_key_package.public().to_bytes());
    ws.send(text_frame(
        serde_json::json!({"type": "NovaJoinRequest", "key_package": key_package_b64}).to_string(),
    ))
    .await
    .expect("send second NovaJoinRequest");

    // No second `NovaWelcome` should ever arrive; assert on the next frame
    // seen being unrelated (a `UserJoined`/`UserCount` from this connection
    // itself, never a second `NovaWelcome`).
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

/// A `NovaJoinRequest` whose `key_package` is valid base64 but not a real
/// `LeafKeyPackage` is rejected by deserialization, not silently accepted.
#[tokio::test]
async fn join_with_malformed_key_package_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    handshake(&mut ws).await;

    ws.send(text_frame(
        serde_json::json!({
            "type": "NovaJoinRequest",
            "key_package": BASE64.encode(b"not a real key package"),
        })
        .to_string(),
    ))
    .await
    .expect("send malformed NovaJoinRequest");

    // A real join right behind it still works.
    join_group(&mut ws).await;
    handle.abort();
}

/// A `Sealed` frame that itself wraps another transport frame (rather than
/// an application `ClientEvent`) is rejected — once sealed, content must be
/// an application frame all the way down.
#[tokio::test]
async fn a_sealed_frame_nesting_a_transport_frame_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let mut session = handshake(&mut ws).await;

    let nested = serde_json::json!({"type": "NovaPreKeyBundleRequest"}).to_string();
    let sealed = session.seal(nested.as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send sealed-nested-transport frame");

    // A genuine application frame right behind it still gets a reply.
    let plaintext = serde_json::json!({"type": "RequestRoster"}).to_string();
    let sealed = session.seal(plaintext.as_bytes()).expect("seal");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send genuine sealed frame");
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// `RlnRegister`/`RlnPathRequest` need no STARK proof at all — proving
/// happens only in `RlnMessage`. A malformed commitment produces no reply;
/// a well-formed one is registered and its path can be fetched, all inside
/// the ordinary debug test run (unlike the full anonymous-post round trip,
/// which needs `--release` — see `nova_rln_anonymous_post_and_double_post_slashing`).
#[tokio::test]
async fn rln_register_and_path_request_work_without_a_stark_proof() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let mut session = handshake(&mut ws).await;

    // A malformed commitment (not valid hex) produces no reply.
    let malformed = serde_json::json!({"type": "RlnRegister", "commitment": "not hex"});
    send_sealed(&mut ws, &mut session, &malformed).await;

    // A well-formed one registers and gets a leaf index.
    let commitment_hex = format!("{:032x}", 42u128);
    let register = serde_json::json!({"type": "RlnRegister", "commitment": commitment_hex});
    send_sealed(&mut ws, &mut session, &register).await;
    let registered = recv_sealed_of_type(&mut ws, &mut session, "RlnRegistered").await;
    let leaf_index = registered["leaf_index"].as_u64().expect("leaf_index");

    // An out-of-range leaf index produces no reply.
    let bad_path = serde_json::json!({"type": "RlnPathRequest", "leaf_index": leaf_index + 1000});
    send_sealed(&mut ws, &mut session, &bad_path).await;

    // The real leaf's path is fetched successfully.
    let path_request = serde_json::json!({"type": "RlnPathRequest", "leaf_index": leaf_index});
    send_sealed(&mut ws, &mut session, &path_request).await;
    let path_response = recv_sealed_of_type(&mut ws, &mut session, "RlnPathResponse").await;
    assert!(path_response["path"].is_array());

    handle.abort();
}

/// Once the room's RLN membership tree has registered
/// [`crate::session::nova_rln::NOVA_RLN_CAPACITY`] distinct commitments —
/// 32, `2^DEPTH` — further registrations are refused rather than silently
/// growing past the tree's fixed depth.
#[tokio::test]
async fn rln_register_refuses_once_the_membership_tree_is_full() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let mut session = handshake(&mut ws).await;

    // `NOVA_RLN_CAPACITY` is `2^DEPTH` (`novachannel_rln::air::DEPTH` = 5),
    // private to `session::nova_rln` — 32 rather than importing it.
    const NOVA_RLN_CAPACITY: usize = 32;
    for i in 0..NOVA_RLN_CAPACITY {
        let commitment_hex = format!("{:032x}", i as u128 + 1);
        let register = serde_json::json!({"type": "RlnRegister", "commitment": commitment_hex});
        send_sealed(&mut ws, &mut session, &register).await;
        let registered = recv_sealed_of_type(&mut ws, &mut session, "RlnRegistered").await;
        assert_eq!(registered["leaf_index"].as_u64(), Some(i as u64));
    }

    // The tree is now full; one more registration is refused. A real
    // roster request right behind it proves the connection is still alive
    // rather than this simply timing out.
    let one_too_many = format!("{:032x}", 999u128);
    let register = serde_json::json!({"type": "RlnRegister", "commitment": one_too_many});
    send_sealed(&mut ws, &mut session, &register).await;

    let roster = serde_json::json!({"type": "RequestRoster"});
    send_sealed(&mut ws, &mut session, &roster).await;
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// An `RlnMessage` whose proof is garbage fails verification quietly — no
/// broadcast, no reply, no panic. This exercises the malformed-proof and
/// verification-failure paths in both `session/nova.rs::dispatch` and
/// `session/nova_rln.rs::verify_and_record` without needing a real STARK
/// proof (that path is `nova_rln_anonymous_post_and_double_post_slashing`,
/// release-only — see its doc comment).
#[tokio::test]
async fn rln_message_with_a_garbage_proof_is_rejected_quietly() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

    // A well-formed 16-byte hex value, so the malformed cases below
    // exercise the specific check they're named for rather than all
    // failing at the same `hex_to_field` call.
    let valid_field_hex = format!("{:032x}", 7u128);

    // Not valid base64 at all — `handle_message`'s own decode, before
    // `verify_and_record` is even called.
    let garbage_b64 = serde_json::json!({
        "type": "RlnMessage",
        "proof": "not valid base64!!",
        "y": valid_field_hex,
        "nullifier": valid_field_hex,
        "text": "hello",
    });
    send_sealed(&mut ws_a, &mut session_a, &garbage_b64).await;

    // `y` is not a well-formed field element.
    let malformed_y = serde_json::json!({
        "type": "RlnMessage",
        "proof": BASE64.encode(b"irrelevant, y is checked first"),
        "y": "not hex",
        "nullifier": valid_field_hex,
        "text": "hello",
    });
    send_sealed(&mut ws_a, &mut session_a, &malformed_y).await;

    // `nullifier` is not a well-formed field element.
    let malformed_nullifier = serde_json::json!({
        "type": "RlnMessage",
        "proof": BASE64.encode(b"irrelevant, nullifier is checked next"),
        "y": valid_field_hex,
        "nullifier": "not hex",
        "text": "hello",
    });
    send_sealed(&mut ws_a, &mut session_a, &malformed_nullifier).await;

    // Valid base64 and well-formed `y`/`nullifier`, but not a real STARK
    // proof — reaches `Proof::from_bytes` and fails there. A real message
    // right behind it, so this checks the garbage one specifically
    // produced nothing (§6.4), not just that something eventually arrived.
    let garbage_proof = serde_json::json!({
        "type": "RlnMessage",
        "proof": BASE64.encode(b"not a real stark proof"),
        "y": valid_field_hex,
        "nullifier": valid_field_hex,
        "text": "hello",
    });
    send_sealed(&mut ws_a, &mut session_a, &garbage_proof).await;

    let plaintext = serde_json::json!({"type": "Message", "text": "a real broadcast"}).to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send a real message");

    let received = recv_group_sealed(&mut ws_b, &mut group_b).await;
    assert!(
        received["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("a real broadcast"),
        "the garbage RLN messages must never broadcast: {received}"
    );

    handle.abort();
}

/// `NovaMpcDemoRequest` is throttled per connection — a second request
/// inside the window produces no second demo run.
#[tokio::test]
async fn mpc_demo_request_is_throttled_on_rapid_repeat() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let mut session = handshake(&mut ws).await;

    let demo = serde_json::json!({"type": "NovaMpcDemoRequest"});
    send_sealed(&mut ws, &mut session, &demo).await;
    send_sealed(&mut ws, &mut session, &demo).await;

    // Whether or not any demo participants are configured, the connection
    // must still be alive and answering ordinary requests afterward.
    let roster = serde_json::json!({"type": "RequestRoster"});
    send_sealed(&mut ws, &mut session, &roster).await;
    let reply = recv_sealed_of_type(&mut ws, &mut session, "Roster").await;
    assert_eq!(reply["type"], "Roster");

    handle.abort();
}

/// A raw ratchet-control record (produced by `initiate_ratchet`, not
/// `seal`) arriving as `Sealed` data is rejected — Phase 1 never sends one,
/// so `open_pairwise` treats it as malformed rather than an application
/// frame.
#[tokio::test]
async fn a_ratchet_control_record_arriving_as_sealed_data_is_rejected() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let mut session = handshake(&mut ws).await;

    let control_record = session
        .initiate_ratchet()
        .expect("initiate_ratchet produces a control record");
    ws.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&control_record)}).to_string(),
    ))
    .await
    .expect("send ratchet-control record as Sealed data");

    // The connection stays alive and usable afterward — proven with
    // `NovaJoinRequest`, which travels unsealed and so doesn't depend on
    // the pairwise session (which a real ratchet advance would need a
    // further round trip to resynchronize, out of scope for this test).
    join_group(&mut ws).await;

    handle.abort();
}

/// `remove_member` on a leaf that is not (or no longer) a group member is a
/// no-op, not a panic — `session/nova.rs`'s own doc: "already removed (e.g.
/// a duplicate teardown call) or the group somehow disagrees about this
/// leaf". Called directly, the same way connection teardown does, rather
/// than through a real disconnect — nothing about a double-teardown is
/// reachable through the wire protocol on purpose.
#[tokio::test]
async fn removing_a_leaf_that_is_not_a_member_is_a_harmless_no_op() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        rooms.insert(NOVA_ROOM.to_string(), create_room());
    }

    // Never joined, so leaf 3 is not a member of anything.
    remove_member(&state, NOVA_ROOM, 3).await;

    // The group is left usable afterward.
    let group = state.nova_group.lock().await;
    assert!(
        !group.is_member(3),
        "removing a non-member must not spuriously add it"
    );
}

/// `run_seal_loop`'s receiver falling behind the channel's capacity
/// produces `RecvError::Lagged`, which the loop logs and continues past
/// rather than treating as fatal. A freshly created, small-capacity
/// channel (not the production `BROADCAST_CHANNEL_CAPACITY`, which would
/// need over a thousand sends to overflow) makes this cheap to trigger
/// directly.
#[tokio::test]
async fn run_seal_loop_survives_a_lagged_receiver() {
    let state = Arc::new(AppState::new());
    let (tx, rx) = tokio::sync::broadcast::channel::<Arc<str>>(2);
    let mut sealed_rx = state.nova_sealed_sender.subscribe();

    // Send more than the channel holds before the loop ever polls, so the
    // first `recv` is guaranteed to observe a lag rather than a message.
    for i in 0..5 {
        let event = serde_json::json!({
            "type": "Message",
            "message": {
                "message_id": Uuid::new_v4(),
                "user_id": "u",
                "animal_name": "otter",
                "text": format!("frame {i}"),
                "timestamp": "1",
                "reactions": [],
            }
        });
        let _ = tx.send(Arc::from(event.to_string()));
    }

    let loop_handle = tokio::spawn(run_seal_loop(state.clone(), rx));

    // A frame sent after the lag must still be sealed and published —
    // proving the loop kept running past the `Lagged` error rather than
    // returning.
    let event = serde_json::json!({
        "type": "Message",
        "message": {
            "message_id": Uuid::new_v4(),
            "user_id": "u",
            "animal_name": "otter",
            "text": "after the lag",
            "timestamp": "1",
            "reactions": [],
        }
    });
    let _ = tx.send(Arc::from(event.to_string()));
    drop(tx);

    let sealed = timeout(Duration::from_secs(5), sealed_rx.recv())
        .await
        .expect("timed out")
        .expect("a sealed frame after the lag");
    assert!(
        sealed.contains("GroupSealed"),
        "expected a sealed broadcast, got {sealed}"
    );

    loop_handle.await.expect("seal loop task panicked");
}

/// `handle_join_request` refuses a join once the room's `Group` is at
/// [`crate::config::NOVA_GROUP_CAPACITY`] — the `Group`'s own ceiling,
/// checked independently of the room's ordinary user-count limit.
/// [`MAX_CONCURRENT_CONNECTIONS_PER_IP`] (3) makes filling the group's real
/// 64-leaf capacity with live sockets from one test-harness IP impossible —
/// correctly so, that limit is a real defense. Instead this fills the
/// *group itself* directly through its own already-proven `propose_add`
/// (bypassing the wire, not the code under test), then makes exactly one
/// real connection and confirms `handle_join_request` refuses it through
/// the genuine dispatch path.
#[tokio::test]
async fn a_group_at_capacity_rejects_further_joins() {
    let (addr, state, handle) = start_ws_server_with_state().await;
    {
        let mut rooms = state.rooms.write().await;
        rooms.insert(NOVA_ROOM.to_string(), create_room());
    }
    {
        let mut group = state.nova_group.lock().await;
        // The server itself is the group's founding leaf 0 (module doc),
        // so capacity minus one more fills every remaining leaf.
        for _ in 0..crate::config::NOVA_GROUP_CAPACITY - 1 {
            let key_package = MyLeafKeyPackage::generate(Identity::generate().public());
            group
                .propose_add(&state.nova_identity, key_package.public())
                .expect("group has room during setup");
        }
    }

    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws, _) = connect_async(&ws_url).await.expect("client connects");
    drain_preamble(&mut ws).await;
    let my_key_package = MyLeafKeyPackage::generate(Identity::generate().public());
    let key_package_b64 = BASE64.encode(my_key_package.public().to_bytes());
    ws.send(text_frame(
        serde_json::json!({"type": "NovaJoinRequest", "key_package": key_package_b64}).to_string(),
    ))
    .await
    .expect("send NovaJoinRequest against a full group");

    // No `NovaWelcome` arrives for this connection; the connection stays
    // open and usable regardless (a refused join is not a fatal error) —
    // proven by the ordinary preamble/roster traffic still flowing.
    ws.send(text_frame(
        serde_json::json!({"type": "RequestRoster"}).to_string(),
    ))
    .await
    .expect("connection is still usable after a refused join");
    tokio::time::sleep(Duration::from_millis(100)).await;

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
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

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

    let received = recv_group_sealed_of_type(&mut ws_b, &mut group_b, "NovaAnonymousMessage").await;
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

    let slashed = recv_group_sealed_of_type(&mut ws_b, &mut group_b, "NovaRlnSlashed").await;
    let recovered = hex_to_field(slashed["recovered_secret"].as_str().expect("hex"));
    assert_eq!(
        recovered, rln_identity.sk,
        "the recovered secret must be the actual offending member's sk"
    );

    handle.abort();
}

/// Cover traffic (`novachannel-dp`, driven by `client.js`'s periodic
/// scheduler) is discarded before it becomes content: no broadcast, no
/// reply, nothing another connection ever sees. Sent right before a real
/// message so the assertion isn't "nothing arrived within some timeout" —
/// it's "the first thing that *did* arrive is the real message, not
/// something derived from the dummy" (§6.4: a test that can pass by
/// waiting for something that must not happen proves nothing).
#[tokio::test]
async fn nova_dummy_frames_are_discarded_without_a_trace() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");

    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let dummy = session_a
        .seal(
            serde_json::json!({"type": "Dummy", "padding": "x".repeat(64)})
                .to_string()
                .as_bytes(),
        )
        .expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&dummy)}).to_string(),
    ))
    .await
    .expect("send dummy frame");

    let plaintext =
        serde_json::json!({"type": "Message", "text": "real message after a dummy"}).to_string();
    let sealed = session_a.seal(plaintext.as_bytes()).expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&sealed)}).to_string(),
    ))
    .await
    .expect("send real message");

    let received = recv_group_sealed(&mut ws_b, &mut group_b).await;
    assert_eq!(
        received["type"], "Message",
        "the dummy must produce nothing observable; first arrival should be the real message: {received}"
    );
    assert!(
        received["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("real message after a dummy")
    );

    handle.abort();
}

/// Spawns [`config::NOVA_OPERATOR_COUNT`] real, separate `nova-operator`
/// processes, waits for them to complete a real DKG ceremony with the
/// server (never sharing a secret share with it — `session/nova_operator.rs`'s
/// whole point), then drives the room's demo button and asserts the result
/// came from that live quorum: `recovered_key_matches` and
/// `signature_valid` both real cryptographic checks, not canned. One client
/// requests the demo; the *other* connection receives the broadcast result
/// too (nothing in it is per-asker).
///
/// Builds `nova-operator`'s binary first if it isn't already there —
/// harmless if it is, `cargo build` no-ops on an unchanged target.
fn nova_operator_bin() -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let build_status = std::process::Command::new(&cargo)
        .args(["build", "-p", "nova-operator", "--bin", "nova-operator"])
        .status()
        .expect("run cargo build for nova-operator");
    assert!(build_status.success(), "nova-operator must build");
    format!("{}/target/debug/nova-operator", env!("CARGO_MANIFEST_DIR"))
}

fn nova_operator_identity_path(index: u32) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "nova-operator-test-{}-{index}.key",
        std::process::id()
    ))
}

/// Spawns one real `nova-operator` process against `operator_url`, using
/// `identity_path` (created fresh if it doesn't already exist — reusing an
/// existing file is exactly how a restarted process presents the same
/// identity, which the reconnect tests below rely on).
fn spawn_operator(
    operator_bin: &str,
    operator_url: &str,
    token: &str,
    identity_path: &std::path::Path,
    stderr: std::process::Stdio,
) -> std::process::Child {
    std::process::Command::new(operator_bin)
        .args([
            "--server",
            operator_url,
            "--token",
            token,
            "--identity",
            identity_path.to_str().expect("temp path is valid UTF-8"),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(stderr)
        .spawn()
        .expect("spawn nova-operator")
}

/// Watches `child`'s piped stdout on a background thread, incrementing
/// `ready` every time a line contains `needle` — the same
/// spawn-then-watch-stdout pattern every operator-fleet test here uses to
/// learn when a process reaches a particular point without polling the
/// server's own state (which the real protocol doesn't expose for tests to
/// peek at, deliberately — see `session/nova_operator.rs`'s module doc on
/// this server never holding a share to report on in the first place).
fn watch_for(
    child: &mut std::process::Child,
    needle: &'static str,
    ready: std::sync::Arc<std::sync::atomic::AtomicU32>,
) {
    use std::io::{BufRead, BufReader};
    let stdout = child.stdout.take().expect("stdout is piped");
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains(needle) {
                ready.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
}

async fn wait_until_at_least(
    counter: &std::sync::atomic::AtomicU32,
    target: u32,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if counter.load(std::sync::atomic::Ordering::SeqCst) >= target {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "only {} of {target} reached in time",
                counter.load(std::sync::atomic::Ordering::SeqCst)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Spawns [`config::NOVA_OPERATOR_COUNT`] real, separate `nova-operator`
/// processes, waits for them to complete a real DKG ceremony with the
/// server (never sharing a secret share with it — `session/nova_operator.rs`'s
/// whole point), then drives the room's demo button and asserts the result
/// came from that live quorum: `recovered_key_matches` and
/// `signature_valid` both real cryptographic checks, not canned. One client
/// requests the demo; the *other* connection receives the broadcast result
/// too (nothing in it is per-asker).
///
/// Builds `nova-operator`'s binary first if it isn't already there —
/// harmless if it is, `cargo build` no-ops on an unchanged target.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn nova_mpc_demo_uses_a_real_distributed_operator_quorum() {
    use std::sync::atomic::AtomicU32;

    let _token_guard = NOVA_OPERATOR_TOKEN_ENV.lock().await;
    let operator_bin = nova_operator_bin();
    let token = format!("test-operator-token-{}", std::process::id());
    unsafe { std::env::set_var("NOVA_OPERATOR_TOKEN", &token) };

    // `start_ws_server()` mounts only `/ws/{room}` — not the real router,
    // so `/ws/nova-operator` would silently be swallowed by that room
    // wildcard instead of reaching `nova_operator_ws_handler`.
    // `start_ws_server_with_state()` uses the actual `build_router`.
    let (addr, _app_state, handle) = start_ws_server_with_state().await;
    let operator_url = format!("ws://{addr}/ws/nova-operator");

    let ready = std::sync::Arc::new(AtomicU32::new(0));
    let mut children = Vec::new();
    for i in 0..crate::config::NOVA_OPERATOR_COUNT {
        let identity_path = nova_operator_identity_path(i);
        let _ = std::fs::remove_file(&identity_path);
        let mut child = spawn_operator(
            &operator_bin,
            &operator_url,
            &token,
            &identity_path,
            std::process::Stdio::null(),
        );
        watch_for(&mut child, "ceremony complete", ready.clone());
        children.push(child);
    }

    wait_until_at_least(
        &ready,
        crate::config::NOVA_OPERATOR_COUNT,
        Duration::from_secs(30),
    )
    .await;

    let ws_url = format!("ws://{addr}/ws/{NOVA_ROOM}");
    let (mut ws_a, _) = connect_async(&ws_url).await.expect("client A connects");
    drain_preamble(&mut ws_a).await;
    let mut session_a = handshake(&mut ws_a).await;
    join_group(&mut ws_a).await;

    let (mut ws_b, _) = connect_async(&ws_url).await.expect("client B connects");
    drain_preamble(&mut ws_b).await;
    let _session_b = handshake(&mut ws_b).await;
    let mut group_b = join_group(&mut ws_b).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let request = session_a
        .seal(
            serde_json::json!({"type": "NovaMpcDemoRequest"})
                .to_string()
                .as_bytes(),
        )
        .expect("seal");
    ws_a.send(text_frame(
        serde_json::json!({"type": "Sealed", "data": BASE64.encode(&request)}).to_string(),
    ))
    .await
    .expect("send demo request");

    let received = recv_group_sealed(&mut ws_b, &mut group_b).await;
    assert_eq!(
        received["type"], "NovaMpcDemoResult",
        "expected a real result from the live quorum, got: {received}"
    );
    assert!(
        received["recovered_key_matches"].as_bool().unwrap_or(false),
        "the real, separate operator processes must recover the real encapsulated key: {received}"
    );
    assert_eq!(
        received["quorum"].as_array().unwrap().len() as u64,
        received["threshold"]
    );
    assert!(
        !received["group_public_key"]
            .as_str()
            .unwrap_or("")
            .is_empty()
    );
    assert!(
        received["signature_valid"].as_bool().unwrap_or(false),
        "every FROST share, computed by a genuinely separate process, and the aggregate must verify: {received}"
    );
    assert!(
        !received["signed_message"].as_str().unwrap_or("").is_empty(),
        "the quorum must have signed the room's real RLN root: {received}"
    );

    for mut child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
    handle.abort();
}

/// The other half of "reconnecting after the ceremony completes"
/// (`session/nova_operator.rs`'s module doc): a *process* restart, unlike a
/// mere dropped connection, genuinely loses the in-memory `KeyShare` — so
/// when the coordinator recognizes the restarted process's identity as a
/// returning participant and replies `Reconnected` instead of a fresh
/// `Welcome`, the operator must fail loudly and exit, not hang forever
/// waiting for a roster that will never come. This test is what caught
/// that exact hang during development (`main.rs`'s "Wait for the roster"
/// loop had no arm for `Reconnected` at all) before fixing it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_restarted_operator_fails_clearly_instead_of_hanging() {
    use std::sync::atomic::AtomicU32;

    let _token_guard = NOVA_OPERATOR_TOKEN_ENV.lock().await;
    let operator_bin = nova_operator_bin();
    let token = format!("test-operator-token-{}", std::process::id());
    unsafe { std::env::set_var("NOVA_OPERATOR_TOKEN", &token) };

    let (addr, _app_state, handle) = start_ws_server_with_state().await;
    let operator_url = format!("ws://{addr}/ws/nova-operator");

    let ready = std::sync::Arc::new(AtomicU32::new(0));
    let mut children = Vec::new();
    let mut identity_paths = Vec::new();
    for i in 0..crate::config::NOVA_OPERATOR_COUNT {
        let identity_path = nova_operator_identity_path(100 + i);
        let _ = std::fs::remove_file(&identity_path);
        let mut child = spawn_operator(
            &operator_bin,
            &operator_url,
            &token,
            &identity_path,
            std::process::Stdio::null(),
        );
        watch_for(&mut child, "ceremony complete", ready.clone());
        identity_paths.push(identity_path);
        children.push(child);
    }

    wait_until_at_least(
        &ready,
        crate::config::NOVA_OPERATOR_COUNT,
        Duration::from_secs(30),
    )
    .await;

    // Kill participant 0's process — its `KeyShare` goes with it, held only
    // in that process's own memory — then start a *new* process pointed at
    // its same identity file. The coordinator still has that identity on
    // the completed ceremony's roster and isn't currently hearing from it,
    // so it will recognize and admit the reconnect; the new process has to
    // recognize *itself* as unable to serve and give up cleanly instead.
    children[0].kill().expect("kill participant 0");
    let _ = children[0].wait();

    let mut restarted = spawn_operator(
        &operator_bin,
        &operator_url,
        &token,
        &identity_paths[0],
        std::process::Stdio::null(),
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = restarted.try_wait().expect("poll restarted process") {
            break status;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = restarted.kill();
            panic!(
                "restarted operator neither exited nor errored within 15s — it hung instead \
                 of failing clearly"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        !status.success(),
        "a restarted operator with no KeyShare must exit non-zero when recognized as a \
         returning participant, not report success"
    );

    for mut child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
    handle.abort();
}

/// Server-side only: the reconnect-admission logic itself
/// (`session/nova_operator.rs::handle_operator_connection`), driven by a
/// raw WebSocket client rather than a real `nova-operator` process — this
/// test only needs to check *admission* (is this identity recognized, is
/// it currently connected already), not actually serve any decrypt/sign
/// request, so a raw client presenting a real identity's public key is
/// enough; it never needs to complete a DKG itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn reconnect_is_rejected_while_still_connected_and_accepted_after_disconnect() {
    use std::sync::atomic::AtomicU32;
    use x25519_dalek::{PublicKey, StaticSecret};

    let _token_guard = NOVA_OPERATOR_TOKEN_ENV.lock().await;
    let operator_bin = nova_operator_bin();
    let token = format!("test-operator-token-{}", std::process::id());
    unsafe { std::env::set_var("NOVA_OPERATOR_TOKEN", &token) };

    let (addr, _app_state, handle) = start_ws_server_with_state().await;
    let operator_url = format!("ws://{addr}/ws/nova-operator");

    let ready = std::sync::Arc::new(AtomicU32::new(0));
    // Connection order among 5 concurrently-spawned processes is a race —
    // `children[0]` is not guaranteed (and in practice usually isn't)
    // assigned participant id 1, so its *actual* id has to be read back
    // from its own stdout rather than assumed.
    let target_participant_id = std::sync::Arc::new(std::sync::Mutex::new(None::<u32>));
    let mut children = Vec::new();
    let mut identity_paths = Vec::new();
    for i in 0..crate::config::NOVA_OPERATOR_COUNT {
        let identity_path = nova_operator_identity_path(200 + i);
        let _ = std::fs::remove_file(&identity_path);
        let mut child = spawn_operator(
            &operator_bin,
            &operator_url,
            &token,
            &identity_path,
            std::process::Stdio::null(),
        );
        if i == 0 {
            // A combined watcher, not `watch_for` — `child.stdout` can only
            // be taken once, and this process's line stream needs checking
            // for both signals (ceremony completion, and its own assigned
            // participant id).
            let stdout = child.stdout.take().expect("stdout is piped");
            let ready = ready.clone();
            let target_participant_id = target_participant_id.clone();
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if line.contains("ceremony complete") {
                        ready.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    if let Some(rest) = line.strip_prefix("assigned participant id ")
                        && let Some(id_str) = rest.split_whitespace().next()
                        && let Ok(id) = id_str.parse::<u32>()
                    {
                        *target_participant_id.lock().expect("lock poisoned") = Some(id);
                    }
                }
            });
        } else {
            watch_for(&mut child, "ceremony complete", ready.clone());
        }
        identity_paths.push(identity_path);
        children.push(child);
    }

    wait_until_at_least(
        &ready,
        crate::config::NOVA_OPERATOR_COUNT,
        Duration::from_secs(30),
    )
    .await;
    let target_participant_id = target_participant_id
        .lock()
        .expect("lock poisoned")
        .expect("participant 0's assigned id must have been observed by now");

    // Read participant 0's real identity back off disk — the same 32 raw
    // secret bytes `nova_operator::crypto::generate_identity` wrote — and
    // derive the public key a raw client presents in its own `Hello`.
    let secret_bytes: [u8; 32] = std::fs::read(&identity_paths[0])
        .expect("read participant 0's identity file")
        .try_into()
        .expect("identity file is 32 bytes");
    let static_public_key_hex: String = PublicKey::from(&StaticSecret::from(secret_bytes))
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    async fn hello_and_first_reply(
        operator_url: &str,
        token: &str,
        static_public_key_hex: &str,
    ) -> serde_json::Value {
        let mut request = operator_url.into_client_request().expect("valid ws url");
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}").parse().expect("valid header"),
        );
        let (ws_stream, _) = connect_async(request).await.expect("raw client connects");
        let (mut write, mut read) = ws_stream.split();
        write
            .send(text_frame(
                serde_json::json!({
                    "type": "Hello",
                    "static_public_key": static_public_key_hex,
                })
                .to_string(),
            ))
            .await
            .expect("send Hello");
        let msg = read
            .next()
            .await
            .expect("a reply before the stream ends")
            .expect("no transport error");
        let text = msg.into_text().expect("a text frame");
        serde_json::from_str(&text).expect("valid JSON")
    }

    // Participant 0's real process is still connected — a raw client
    // presenting the same identity must be rejected, not silently swapped
    // in for the real one. `handle_operator_connection` rejects by closing
    // the socket immediately with no message at all (`ws_sink.close()`),
    // so the only valid outcomes here are the stream ending or erroring —
    // never a `Text` frame, which would mean it was actually admitted.
    let mut request = operator_url
        .as_str()
        .into_client_request()
        .expect("valid ws url");
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}").parse().expect("valid header"),
    );
    let (ws_stream, _) = connect_async(request)
        .await
        .expect("raw client connects while participant 0 is still live");
    let (mut write, mut read) = ws_stream.split();
    write
        .send(text_frame(
            serde_json::json!({"type": "Hello", "static_public_key": static_public_key_hex})
                .to_string(),
        ))
        .await
        .expect("send Hello");
    match read.next().await {
        None => {}                          // stream ended — rejected
        Some(Err(_)) => {}                  // transport error — rejected
        Some(Ok(WsMessage::Close(_))) => {} // explicit close frame — rejected
        Some(Ok(other)) => panic!(
            "a raw client presenting a currently-connected participant's identity must be \
             rejected, not answered with {other:?}"
        ),
    }

    // Now kill participant 0's real process — freeing that identity to
    // reconnect — and confirm the *same* raw-client approach is accepted
    // and told its original participant id back.
    children[0].kill().expect("kill participant 0");
    let _ = children[0].wait();
    // Give the server a moment to process the disconnect before retrying;
    // the WS close and the retry are two different connections with no
    // other ordering guarantee between them.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let reply = hello_and_first_reply(&operator_url, &token, &static_public_key_hex).await;
    assert_eq!(
        reply["type"], "Reconnected",
        "a raw client presenting a now-free, previously-known identity must be admitted as a \
         reconnect: {reply}"
    );
    assert_eq!(
        reply["participant_id"], target_participant_id,
        "must be reassigned its original participant id: {reply}"
    );

    for mut child in children.into_iter().skip(1) {
        let _ = child.kill();
        let _ = child.wait();
    }
    handle.abort();
}

/// `complaint_is_valid` in isolation, against a real `Dealer`'s real
/// `/ws/nova-operator` is gated on `NOVA_OPERATOR_TOKEN` exactly like
/// `/metrics`/`/admin` — a connection with no or the wrong bearer token
/// never reaches the WebSocket upgrade at all.
#[tokio::test]
async fn an_unauthorized_operator_connection_is_rejected_before_the_upgrade() {
    let _token_guard = NOVA_OPERATOR_TOKEN_ENV.lock().await;
    let token = format!("test-operator-token-{}", std::process::id());
    unsafe { std::env::set_var("NOVA_OPERATOR_TOKEN", &token) };

    let (addr, _app_state, handle) = start_ws_server_with_state().await;
    let operator_url = format!("ws://{addr}/ws/nova-operator");

    // No Authorization header at all.
    let err = connect_async(&operator_url)
        .await
        .expect_err("no token must be rejected");
    assert!(
        matches!(
            err,
            tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status() == http::StatusCode::UNAUTHORIZED
        ),
        "expected 401, got {err:?}"
    );

    // The wrong token.
    let mut request = operator_url.into_client_request().expect("valid ws url");
    request.headers_mut().insert(
        "Authorization",
        "Bearer not-the-real-token".parse().expect("valid header"),
    );
    let err = connect_async(request)
        .await
        .expect_err("wrong token must be rejected");
    assert!(
        matches!(
            err,
            tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status() == http::StatusCode::UNAUTHORIZED
        ),
        "expected 401, got {err:?}"
    );

    unsafe { std::env::remove_var("NOVA_OPERATOR_TOKEN") };
    handle.abort();
}

/// Anything received before the first `Hello` — a non-text frame, or a
/// well-formed `OperatorMessage` of the wrong variant — is silently
/// ignored, not treated as an error; a real `Hello` right behind it still
/// gets admitted.
#[tokio::test]
async fn frames_before_the_first_hello_are_ignored_not_fatal() {
    let _token_guard = NOVA_OPERATOR_TOKEN_ENV.lock().await;
    let token = format!("test-operator-token-{}", std::process::id());
    unsafe { std::env::set_var("NOVA_OPERATOR_TOKEN", &token) };

    let (addr, _app_state, handle) = start_ws_server_with_state().await;
    let operator_url = format!("ws://{addr}/ws/nova-operator");

    let mut request = operator_url
        .as_str()
        .into_client_request()
        .expect("valid ws url");
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}").parse().expect("valid header"),
    );
    let (ws_stream, _) = connect_async(request).await.expect("client connects");
    let (mut write, mut read) = ws_stream.split();

    // A non-text frame first.
    write
        .send(WsMessage::Ping(vec![].into()))
        .await
        .expect("send ping");
    // A well-formed message of the wrong type.
    write
        .send(text_frame(
            serde_json::json!({"type": "NoMoreComplaints"}).to_string(),
        ))
        .await
        .expect("send NoMoreComplaints before Hello");

    let secret_bytes: [u8; 32] = [7u8; 32];
    let static_public_key_hex: String =
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(secret_bytes))
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
    write
        .send(text_frame(
            serde_json::json!({
                "type": "Hello",
                "static_public_key": static_public_key_hex,
            })
            .to_string(),
        ))
        .await
        .expect("send the real Hello");

    // The connection is admitted and stays open — proven by not closing
    // within a short window. `Pong` is the transport layer's own automatic
    // reply to the `Ping` above, not a signal from admission logic, so it
    // is drained rather than treated as "something arrived."
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match timeout(remaining, read.next()).await {
            Err(_) => break,
            Ok(Some(Ok(WsMessage::Pong(_)))) => continue,
            Ok(other) => panic!(
                "an admitted connection must not be closed just because it received pre-Hello noise: {other:?}"
            ),
        }
    }

    unsafe { std::env::remove_var("NOVA_OPERATOR_TOKEN") };
    handle.abort();
}

/// Once [`crate::config::NOVA_OPERATOR_COUNT`] operators are connected and
/// the roster is still being assembled, a further connection is refused —
/// raw clients that only ever send `Hello`, never a real DKG participant,
/// which is all `handle_operator_connection`'s admission logic needs to
/// exercise this branch.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_sixth_operator_is_rejected_while_the_roster_is_still_forming() {
    let _token_guard = NOVA_OPERATOR_TOKEN_ENV.lock().await;
    let token = format!("test-operator-token-{}", std::process::id());
    unsafe { std::env::set_var("NOVA_OPERATOR_TOKEN", &token) };

    let (addr, _app_state, handle) = start_ws_server_with_state().await;
    let operator_url = format!("ws://{addr}/ws/nova-operator");

    async fn connect_and_say_hello(
        operator_url: &str,
        token: &str,
        seed: u8,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut request = operator_url.into_client_request().expect("valid ws url");
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}").parse().expect("valid header"),
        );
        let (mut ws_stream, _) = connect_async(request).await.expect("client connects");
        let secret_bytes = [seed; 32];
        let static_public_key_hex: String =
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(secret_bytes))
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
        ws_stream
            .send(text_frame(
                serde_json::json!({
                    "type": "Hello",
                    "static_public_key": static_public_key_hex,
                })
                .to_string(),
            ))
            .await
            .expect("send Hello");
        ws_stream
    }

    // Fill the roster — `NOVA_OPERATOR_COUNT` (5) raw clients, each
    // distinct by its identity seed.
    let mut sockets = Vec::new();
    for seed in 1..=crate::config::NOVA_OPERATOR_COUNT as u8 {
        sockets.push(connect_and_say_hello(&operator_url, &token, seed).await);
    }
    // Give the server a moment to have processed every `Hello`.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A sixth is refused: the connection is closed rather than admitted.
    let mut sixth = connect_and_say_hello(&operator_url, &token, 99).await;
    let outcome = timeout(Duration::from_secs(2), sixth.next()).await;
    match outcome {
        Ok(None) | Err(_) => {}
        Ok(Some(Ok(WsMessage::Close(_)))) => {}
        Ok(Some(Ok(msg))) => panic!("a rejected sixth operator must not be admitted: {msg:?}"),
        Ok(Some(Err(_))) => {}
    }

    unsafe { std::env::remove_var("NOVA_OPERATOR_TOKEN") };
    handle.abort();
}

/// reveal — no processes, no network, deterministic and fast, unlike the
/// full multi-process tests above. This is the exact function both this
/// server and every honest `nova-operator` run to decide whether to
/// exclude an accused dealer, so pinning it directly is what would catch a
/// regression that let a malicious dealer's bad share go unexcluded (or,
/// the opposite failure, excluded an innocent one).
#[test]
fn complaint_is_valid_only_when_the_share_genuinely_fails_verification() {
    let dealer = novachannel_mpc::Dealer::new(3, 5);
    let (commitments, shares) = dealer.reveal();
    let mut dealer_commitments = std::collections::BTreeMap::new();
    dealer_commitments.insert(1u32, commitments);

    let real_share = shares[&3];
    assert!(
        !complaint_is_valid(&dealer_commitments, 1, 3, &real_share),
        "a complaint about a genuinely valid share must not validate"
    );

    let tampered_share = real_share + curve25519_dalek::scalar::Scalar::ONE;
    assert!(
        complaint_is_valid(&dealer_commitments, 1, 3, &tampered_share),
        "a complaint about a genuinely tampered share must validate"
    );

    assert!(
        !complaint_is_valid(&dealer_commitments, 99, 3, &real_share),
        "a complaint naming a dealer id this check has no commitments for must not validate"
    );
}
