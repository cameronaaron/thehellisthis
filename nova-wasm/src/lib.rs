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
