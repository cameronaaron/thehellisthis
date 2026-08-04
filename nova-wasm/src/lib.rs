//! Browser bindings for `novachannel`'s core PQ channel: the initiator side
//! of the same handshake `src/session/nova.rs` runs as the responder.
//!
//! One object, one session — `client.js`'s `nova`-only branch constructs one
//! `NovaClient` per connection and drives it through exactly the four calls
//! below, in order. There is no group ratchet here (`novachannel` is
//! pairwise): this client talks to the server, and only the server, which is
//! what "the server reseals each broadcast once per recipient"
//! (`session/nova.rs`, `session/tasks.rs::forward_broadcasts`) is the other
//! half of.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use novachannel::{Identity, InitiatorHandshakeState, Opened, RatchetedSession, initiator_start};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub struct NovaClient {
    identity: Identity,
    handshake: Option<InitiatorHandshakeState>,
    session: Option<RatchetedSession>,
}

impl Default for NovaClient {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl NovaClient {
    /// A fresh, ephemeral identity — generated in the browser, held only for
    /// this connection's lifetime. There is nothing to persist: TOFU, same
    /// as the server's own identity (`state.rs::AppState::nova_identity`).
    #[wasm_bindgen(constructor)]
    pub fn new() -> NovaClient {
        NovaClient {
            identity: Identity::generate(),
            handshake: None,
            session: None,
        }
    }

    /// Step 1: produces msg1, base64-encoded, to send as
    /// `{"type":"NovaHandshakeInit","msg1":...}`.
    #[wasm_bindgen(js_name = startHandshake)]
    pub fn start_handshake(&mut self) -> String {
        let (state, msg1) = initiator_start(None);
        self.handshake = Some(state);
        BASE64.encode(msg1)
    }

    /// Step 2: consumes the server's msg2 (from `NovaHandshakeResponse`),
    /// establishes the ratcheted session, and returns msg3, base64-encoded,
    /// to send as `{"type":"NovaHandshakeComplete","msg3":...}`.
    #[wasm_bindgen(js_name = completeHandshake)]
    pub fn complete_handshake(&mut self, msg2_b64: &str) -> Result<String, JsValue> {
        let handshake = self
            .handshake
            .take()
            .ok_or_else(|| js_err("handshake was not started"))?;
        let msg2 = BASE64.decode(msg2_b64).map_err(js_err)?;
        let (msg3, established) = handshake
            .complete(&self.identity, &msg2)
            .map_err(js_err)?;
        self.session = Some(RatchetedSession::new(&established, true));
        Ok(BASE64.encode(msg3))
    }

    /// True once `completeHandshake` has succeeded — `client.js` uses this
    /// to decide whether a frame should go out sealed or is still part of
    /// the handshake itself.
    #[wasm_bindgen(js_name = isEstablished)]
    pub fn is_established(&self) -> bool {
        self.session.is_some()
    }

    /// Seals `plaintext` (already-JSON-encoded `ClientEvent`) for sending,
    /// returning base64 for `{"type":"Sealed","data":...}`.
    pub fn seal(&mut self, plaintext: &str) -> Result<String, JsValue> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| js_err("session is not established yet"))?;
        let record = session.seal(plaintext.as_bytes()).map_err(js_err)?;
        Ok(BASE64.encode(record))
    }

    /// Opens a base64 sealed record from `{"type":"Sealed","data":...}`.
    /// Returns the decrypted JSON string, or `undefined` for a
    /// ratchet-control record with nothing to deliver — Phase 1 never sends
    /// one, so `client.js` never actually sees `undefined` here today, but
    /// the type is honest about the case existing in the protocol.
    pub fn open(&mut self, record_b64: &str) -> Result<Option<String>, JsValue> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| js_err("session is not established yet"))?;
        let record = BASE64.decode(record_b64).map_err(js_err)?;
        match session.open(&record).map_err(js_err)? {
            Opened::Application(bytes) => Ok(Some(String::from_utf8(bytes).map_err(js_err)?)),
            Opened::RatchetAdvanced { .. } => Ok(None),
        }
    }
}

/// One step of an RLN Merkle path, exactly as `RlnPathResponse.path`
/// serialises it (`protocol.rs::RlnPathStep`) — this is the wire format,
/// parsed here rather than imported, since `nova-wasm` has no dependency on
/// the server crate and shouldn't grow one just for two field names.
#[derive(serde::Deserialize)]
struct WirePathStep {
    sibling: String,
    side: String,
}

fn hex_to_field(hex: &str) -> Result<novachannel_rln_field::BaseElement, JsValue> {
    let bytes = hex_decode(hex).ok_or_else(|| js_err("malformed hex"))?;
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| js_err("hex is not 16 bytes"))?;
    Ok(novachannel_rln_field::BaseElement::new(
        u128::from_be_bytes(arr),
    ))
}

fn field_to_hex(v: novachannel_rln_field::BaseElement) -> String {
    use novachannel_rln_field::StarkField;
    v.as_int()
        .to_be_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// The `winterfell`/`novachannel_rln` field types, named once so the rest
/// of this file reads as domain code rather than re-typing the same
/// `winterfell::math::fields::f128::BaseElement` path everywhere.
mod novachannel_rln_field {
    pub use winterfell::math::StarkField;
    pub use winterfell::math::fields::f128::BaseElement;
}

/// An RLN membership identity — the anonymous, rate-limited side of `nova`
/// (`session/nova_rln.rs` is the server-side verifier and nullifier set).
/// Independent of [`NovaClient`]: an anonymous post doesn't need this
/// connection's PQ-channel identity, and never carries it.
#[wasm_bindgen]
pub struct NovaRlnIdentity {
    identity: novachannel_rln::Identity,
}

impl Default for NovaRlnIdentity {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl NovaRlnIdentity {
    /// A fresh secret key, generated in the browser and never sent anywhere
    /// — only its public [`commitment`](Self::commitment) and, later, proof
    /// outputs ever leave this object.
    #[wasm_bindgen(constructor)]
    pub fn new() -> NovaRlnIdentity {
        NovaRlnIdentity {
            identity: novachannel_rln::Identity::generate(),
        }
    }

    /// Hex-encoded commitment for `{"type":"RlnRegister","commitment":...}`.
    pub fn commitment(&self) -> String {
        let params = novachannel_rln::permutation::Params::new();
        field_to_hex(self.identity.commitment(&params))
    }

    /// Proves membership + a rate-limit share for `text` at the given
    /// epoch, using `path_json` (`RlnPathResponse.path`, passed through
    /// verbatim as JSON text — fetched fresh immediately before this call,
    /// never cached; see `session/nova_rln.rs`'s module doc for why).
    /// Returns a JSON string `{"proof":...,"y":...,"nullifier":...}`, the
    /// three fields `{"type":"RlnMessage",...}` needs beyond `text` itself.
    pub fn prove(&self, path_json: &str, epoch: u64, text: &str) -> Result<String, JsValue> {
        use novachannel_rln::air;
        use novachannel_rln::merkle::{PathStep, Side};
        use novachannel_rln::permutation::{Params, compress2};
        use novachannel_rln::{bytes_to_field, epoch_field};

        let wire_path: Vec<WirePathStep> =
            serde_json::from_str(path_json).map_err(js_err)?;
        let path = wire_path
            .into_iter()
            .map(|step| {
                Ok(PathStep {
                    sibling: hex_to_field(&step.sibling)?,
                    side: match step.side.as_str() {
                        "Left" => Side::Left,
                        "Right" => Side::Right,
                        other => return Err(js_err(format!("unknown side {other}"))),
                    },
                })
            })
            .collect::<Result<Vec<_>, JsValue>>()?;

        let params = Params::new();
        let epoch_x = epoch_field(epoch);
        let message_x = bytes_to_field(text.as_bytes());
        let a1 = compress2(&params, self.identity.sk, epoch_x);
        let y = self.identity.sk + a1 * message_x;

        let witness = air::Witness {
            sk: self.identity.sk,
            path,
        };
        let (proof, public) = air::prove(&witness, epoch_x, message_x, y, a1).map_err(js_err)?;

        let out = serde_json::json!({
            "proof": BASE64.encode(proof.to_bytes()),
            "y": field_to_hex(public.y),
            "nullifier": field_to_hex(public.nullifier),
        });
        Ok(out.to_string())
    }
}

/// Cover-traffic decisions (`novachannel-dp`). The decision has to be made
/// here, in the browser, not server-side: it exists to hide from a
/// *network*-position observer whether this connection is sending real
/// traffic at all, and the server already sees every real send regardless
/// of what any scheduler decides.
#[wasm_bindgen]
pub struct NovaDummyScheduler {
    scheduler: novachannel_dp::DummyScheduler,
}

impl Default for NovaDummyScheduler {
    fn default() -> Self {
        Self::new(1.0)
    }
}

#[wasm_bindgen]
impl NovaDummyScheduler {
    /// `epsilon`: the per-slot differential-privacy budget. Lower hides
    /// more (higher dummy-send probability, more bandwidth); `client.js`
    /// picks the actual value (`NOVA_DP_EPSILON`) — this binding is
    /// mechanism, not policy.
    #[wasm_bindgen(constructor)]
    pub fn new(epsilon: f64) -> NovaDummyScheduler {
        NovaDummyScheduler {
            scheduler: novachannel_dp::DummyScheduler::new(epsilon),
        }
    }

    /// True if this slot should transmit — always true when
    /// `has_real_message`, otherwise true with the scheduler's calibrated
    /// dummy probability. The caller (`client.js`) is responsible for
    /// actually sending an indistinguishable dummy frame when this returns
    /// true and there was no real message; the guarantee is about the
    /// *decision bit*, and is void if a dummy is distinguishable from a
    /// real send by size or timing (`novachannel-dp`'s own doc comment).
    pub fn decide(&self, has_real_message: bool) -> bool {
        self.scheduler.decide(has_real_message, &mut rand::thread_rng())
    }
}
