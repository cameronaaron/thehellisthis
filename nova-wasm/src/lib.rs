//! Browser bindings for `novachannel`'s PQ channel and group ratchet, mirroring
//! `src/session/nova.rs`:
//!
//! - The pairwise X3DH session (`establishSession`/`seal`/`open`), the initiator
//!   side of what `session/nova.rs` answers. Used for sending and for replies
//!   owed to this connection alone.
//! - The room's `Group` (`myKeyPackage`/`completeJoin`/`applyCommit`/
//!   `openGroup`), joined once and then only opened with: broadcasts are sealed
//!   once by the server, and no browser calls `Group::seal` (a shared send
//!   chain can't have several sealers without agreeing on order).
//!
//! One `NovaClient` per connection; `client.js` runs both handshakes side by
//! side.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use novachannel::group::{Commit, Group, MyLeafKeyPackage, Welcome};
use novachannel::prekey::{DhIdentity, PreKeyBundle};
use novachannel::ratchet::{Opened, RatchetedSession};
use novachannel::x3dh::initiate;
use novachannel::{Identity, PublicIdentity};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Whether this connection has joined the `Group`. The `LeafKeyPackage` is
/// generated up front, ready to publish; the `Group` exists only once
/// `completeJoin` processes the server's reply.
enum GroupState {
    AwaitingJoin(MyLeafKeyPackage),
    Joined(Group),
}

#[wasm_bindgen]
pub struct NovaClient {
    identity: Identity,
    dh_identity: DhIdentity,
    session: Option<RatchetedSession>,
    group: GroupState,
}

impl Default for NovaClient {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl NovaClient {
    /// Fresh keys for both sessions, generated in the browser for this
    /// connection's lifetime only. TOFU, like the server's own keys
    /// (`AppState::nova_dh_identity`/`nova_identity`).
    #[wasm_bindgen(constructor)]
    pub fn new() -> NovaClient {
        let identity = Identity::generate();
        let my_key_package = MyLeafKeyPackage::generate(&identity);
        NovaClient {
            identity,
            dh_identity: DhIdentity::generate(),
            session: None,
            group: GroupState::AwaitingJoin(my_key_package),
        }
    }

    // ---- Pairwise session (sending, single-recipient replies) ------------

    /// Establishes the pairwise session from the server's prekey bundle
    /// (`NovaPreKeyBundleResponse.bundle`, base64) and returns the base64 X3DH init
    /// message for `{"type":"NovaX3dhInit","message":...}`. The session exists as
    /// soon as this returns; there is no reply to wait for.
    #[wasm_bindgen(js_name = establishSession)]
    pub fn establish_session(&mut self, bundle_b64: &str) -> Result<String, JsValue> {
        let bundle_bytes = BASE64.decode(bundle_b64).map_err(js_err)?;
        let bundle = PreKeyBundle::from_bytes(&bundle_bytes).map_err(js_err)?;
        // The server's identity is TOFU, but the bundle's internal
        // signature over its signed prekey is still checked: trust a
        // coherently signed bundle, don't skip verification.
        bundle.verify().map_err(js_err)?;

        let my_public: PublicIdentity = self.identity.public();
        let initiated = initiate(&my_public, &self.dh_identity, &bundle, &[]).map_err(js_err)?;
        self.session = Some(RatchetedSession::new(&initiated.session, true));
        Ok(BASE64.encode(initiated.message.bytes))
    }

    /// True once `establishSession` succeeded; `client.js` uses it to decide
    /// whether a frame goes out sealed.
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

    /// Opens a pairwise record from `{"type":"Sealed","data":...}` (broadcasts use
    /// [`Self::open_group`]). Returns the JSON, or `undefined` for a
    /// ratchet-control record; the server never sends one today, but the protocol
    /// allows it.
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

    // ---- Group (broadcast content only) -----------------------------------

    /// This connection's public `LeafKeyPackage` (base64) for
    /// `{"type":"NovaJoinRequest","key_package":...}`. The same package on every
    /// call before joining; after `completeJoin` it errors rather than generate an
    /// unrelated second leaf.
    #[wasm_bindgen(js_name = myKeyPackage)]
    pub fn my_key_package(&self) -> Result<String, JsValue> {
        match &self.group {
            GroupState::AwaitingJoin(my_key_package) => {
                Ok(BASE64.encode(my_key_package.public().to_bytes()))
            }
            GroupState::Joined(_) => Err(js_err("already joined the group")),
        }
    }

    /// Completes the `Group` join from `NovaWelcome` (`welcome`/`commit`, base64);
    /// [`Self::open_group`] works from then on. `false` if already joined or if
    /// they don't apply to this connection's pending key package.
    #[wasm_bindgen(js_name = completeJoin)]
    pub fn complete_join(&mut self, welcome_b64: &str, commit_b64: &str) -> Result<(), JsValue> {
        let GroupState::AwaitingJoin(_) = &self.group else {
            return Err(js_err("already joined the group"));
        };
        // Swapped out with a throwaway because `Group::join` consumes the
        // `MyLeafKeyPackage` by value and private leaf keys aren't
        // `Clone` (they're meant to exist in one place).
        let GroupState::AwaitingJoin(my_key_package) = std::mem::replace(
            &mut self.group,
            GroupState::AwaitingJoin(dummy_key_package()),
        ) else {
            unreachable!("matched AwaitingJoin above");
        };

        let welcome =
            Welcome::from_bytes(&BASE64.decode(welcome_b64).map_err(js_err)?).map_err(js_err)?;
        let commit =
            Commit::from_bytes(&BASE64.decode(commit_b64).map_err(js_err)?).map_err(js_err)?;
        let group = Group::join(my_key_package, &welcome, &commit).map_err(js_err)?;
        self.group = GroupState::Joined(group);
        Ok(())
    }

    /// True once [`Self::complete_join`] has succeeded.
    #[wasm_bindgen(js_name = isGroupJoined)]
    pub fn is_group_joined(&self) -> bool {
        matches!(self.group, GroupState::Joined(_))
    }

    /// This connection's view of the group epoch once joined, else
    /// `undefined`. Logged next to the server's `group.epoch()` so a viewer can
    /// confirm both sides agree.
    pub fn epoch(&self) -> Option<u64> {
        match &self.group {
            GroupState::Joined(group) => Some(group.epoch()),
            GroupState::AwaitingJoin(_) => None,
        }
    }

    /// Applies a `NovaCommit` broadcast (base64): each membership change after
    /// this connection's own join, in the server's order. Errors rather than
    /// panics if not yet joined or the commit doesn't fit the current epoch;
    /// `client.js` logs and continues, since the legitimate case (receiving its own
    /// admitting commit twice, via `NovaWelcome` and the broadcast) is harmless.
    #[wasm_bindgen(js_name = applyCommit)]
    pub fn apply_commit(&mut self, commit_b64: &str) -> Result<(), JsValue> {
        let GroupState::Joined(group) = &mut self.group else {
            return Err(js_err("group not joined yet"));
        };
        let commit =
            Commit::from_bytes(&BASE64.decode(commit_b64).map_err(js_err)?).map_err(js_err)?;
        group.apply_commit(&commit).map_err(js_err)
    }

    /// Opens a `{"type":"GroupSealed","data":...}` record (base64), sealed once by
    /// the server (`session/nova.rs::run_seal_loop`), returning the JSON.
    #[wasm_bindgen(js_name = openGroup)]
    pub fn open_group(&mut self, record_b64: &str) -> Result<String, JsValue> {
        let GroupState::Joined(group) = &mut self.group else {
            return Err(js_err("group not joined yet"));
        };
        let record = BASE64.decode(record_b64).map_err(js_err)?;
        let (_sender_leaf, plaintext) = group.open(&record).map_err(js_err)?;
        String::from_utf8(plaintext).map_err(js_err)
    }
}

/// Filler for `std::mem::replace` while the real package is moved out of
/// `GroupState`; [`NovaClient::complete_join`] reaches that line once, so no
/// caller sees it.
fn dummy_key_package() -> MyLeafKeyPackage {
    MyLeafKeyPackage::generate(&Identity::generate())
}

/// One RLN Merkle path step as `RlnPathResponse.path` serialises it
/// (`protocol.rs::RlnPathStep`), parsed here because `nova-wasm` doesn't
/// depend on the server crate.
#[derive(serde::Deserialize)]
struct WirePathStep {
    sibling: String,
    side: String,
}

/// Decodes a canonical field element, rejecting values at or above the
/// modulus exactly as the server's `hex_to_field` does.
fn hex_to_field(hex: &str) -> Result<novachannel_rln_field::BaseElement, JsValue> {
    use novachannel_rln_field::StarkField;
    let bytes = hex_decode(hex).ok_or_else(|| js_err("malformed hex"))?;
    let arr: [u8; 8] = bytes.try_into().map_err(|_| js_err("hex is not 8 bytes"))?;
    let value = u64::from_be_bytes(arr);
    if value >= novachannel_rln_field::BaseElement::MODULUS {
        return Err(js_err("field element is not canonical"));
    }
    Ok(novachannel_rln_field::BaseElement::new(value))
}

fn field_to_hex(v: novachannel_rln_field::BaseElement) -> String {
    v.as_int()
        .to_be_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
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

/// The field types `novachannel_rln` is built on, named once.
mod novachannel_rln_field {
    pub use winterfell::math::fields::f64::BaseElement;
    pub use winterfell::math::StarkField;
}

/// An RLN membership identity for anonymous, rate-limited posting
/// (`session/nova_rln.rs` verifies). Independent of [`NovaClient`]: an
/// anonymous post never carries the PQ-channel identity.
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
    /// A fresh secret key that never leaves the browser; only its
    /// [`commitment`](Self::commitment) and proof outputs do.
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

    /// Proves membership and a rate-limit share for `text` at `epoch`, using
    /// `path_json` (`RlnPathResponse.path`, fetched just before, never cached; see
    /// `session/nova_rln.rs`). Returns `{"proof":...,"y":...,"nullifier":...}`, the
    /// fields `RlnMessage` needs besides `text`.
    pub fn prove(&self, path_json: &str, epoch: u64, text: &str) -> Result<String, JsValue> {
        use novachannel_rln::air;
        use novachannel_rln::merkle::{PathStep, Side};
        use novachannel_rln::permutation::{compress2, Params};
        use novachannel_rln::{bytes_to_field, epoch_field};

        let wire_path: Vec<WirePathStep> = serde_json::from_str(path_json).map_err(js_err)?;
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

/// Cover-traffic decisions (`novachannel-dp`), made in the browser because they
/// exist to hide from a network observer whether this connection is sending;
/// the server sees every real send anyway.
///
/// At positive epsilon this is cover traffic, not differential privacy:
/// upstream documents that a silent slot still reveals nothing was queued, and
/// that its `Budget` doesn't certify this scheduler. The `Budget` here is a cap
/// on how long the connection sends dummies: `decide` spends a slot per call,
/// real or dummy, and once it runs out dummies stop while real sends continue.
#[wasm_bindgen]
pub struct NovaDummyScheduler {
    scheduler: novachannel_dp::DummyScheduler,
    budget: novachannel_dp::Budget,
}

impl Default for NovaDummyScheduler {
    fn default() -> Self {
        Self::new(1.0, 1000.0)
    }
}

#[wasm_bindgen]
impl NovaDummyScheduler {
    /// `epsilon` sets the dummy probability (`e^-epsilon` per idle slot; lower
    /// means more dummies and bandwidth). `total_budget` caps the slots spent
    /// before dummies stop. `client.js` picks both (`NOVA_DP_EPSILON`,
    /// `NOVA_DP_TOTAL_BUDGET`); this binding is mechanism, not policy.
    #[wasm_bindgen(constructor)]
    pub fn new(epsilon: f64, total_budget: f64) -> NovaDummyScheduler {
        NovaDummyScheduler {
            scheduler: novachannel_dp::DummyScheduler::new(epsilon),
            budget: novachannel_dp::Budget::new(epsilon, total_budget),
        }
    }

    /// Whether to transmit this slot: always with a real message; otherwise with
    /// the scheduler's dummy probability until the cap is spent, then never. Real
    /// sends are never withheld. When this says to send a dummy, `client.js` must
    /// send one indistinguishable from a real frame by size and timing.
    pub fn decide(&mut self, has_real_message: bool) -> bool {
        if !self.budget.spend_slot() {
            return has_real_message;
        }
        self.scheduler.decide(has_real_message, &mut rand::rng())
    }

    /// Total epsilon spent so far — `client.js`'s visible budget readout.
    pub fn spent(&self) -> f64 {
        self.budget.spent()
    }

    /// Epsilon left before cover traffic stops.
    pub fn remaining(&self) -> f64 {
        self.budget.remaining()
    }

    /// True once `remaining()` is zero and `decide` has stopped sending dummies.
    #[wasm_bindgen(js_name = isExhausted)]
    pub fn is_exhausted(&self) -> bool {
        self.budget.remaining() <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::hex_decode;

    #[test]
    fn hex_decode_rejects_non_ascii_and_signed_pairs() {
        assert_eq!(hex_decode("a\u{e9}b"), None);
        assert_eq!(hex_decode("+1"), None);
        assert_eq!(hex_decode("0aff"), Some(vec![0x0a, 0xff]));
    }
}
