//! The WebSocket wire format: every type that crosses the socket.
//!
//! No dependency on server state, so the browser's contract reads in one file.
//! Any change here is a protocol change, and the client changes in the same
//! commit.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::config::ESTIMATED_MESSAGE_SIZE;

/// Serialises a frame the server is about to send.
///
/// Every outgoing type is owned strings, numbers and UUIDs (no non-string map
/// keys, no NaN-able float), so this can't fail; saying so once beats an
/// unreachable error arm at each call site.
pub(crate) fn encode_event<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| {
        // Reached only if an outgoing type gains an unrepresentable field. The client
        // drops an empty frame; a panic would take the connection task (§5.1).
        error!(error = %e, "failed to serialise an outgoing frame");
        String::new()
    })
}

/// Serialises a broadcast frame once for every connection in the room.
///
/// `broadcast::Receiver::recv` clones its value per receiver, so a channel of
/// unencoded events made every forward task re-clone the payload and re-run
/// `serde_json::to_string` on identical output: up to a hundred copies of a
/// 128 KiB image message in a full room, on the server's one thread. `Arc<str>`
/// makes each receiver's share a refcount bump (§1.4c).
pub(crate) fn encode_broadcast<T: Serialize>(value: &T) -> Arc<str> {
    Arc::from(encode_event(value))
}

/// The message a reply points at, denormalised so the client can render the
/// quoted preview without holding full history.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReplyInfo {
    pub message_id: String,
    pub author_name: String,
    pub preview_text: String,
}

/// An inline image carried by a message.
///
/// The bytes travel in the message because there's nowhere else: no object
/// store, no database, and no origin the CSP would let the browser fetch
/// (§5.7). The client renders `data:{mime};base64,{data}`, exactly what
/// `img-src 'self' data:` permits.
///
/// `data` is the payload without the `data:` prefix, which the client rebuilds
/// from `mime`, so a client can't smuggle another scheme past the type check.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Attachment {
    /// One of [`crate::config::ALLOWED_ATTACHMENT_MIMES`], verified against the
    /// payload's magic bytes rather than believed.
    pub mime: String,
    /// Base64, no prefix. Empty once the image has faded from the room
    /// (`faded` is then true) — the message survives, the picture does not.
    pub data: String,
    /// Display hints so the client can reserve space before decoding. Bounded, not
    /// trusted.
    pub width: u32,
    pub height: u32,
    /// True when the payload has aged out of the room's attachment budget. The
    /// client shows a placeholder at the right size instead of a broken image.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub faded: bool,
}

impl Attachment {
    /// Bytes this attachment costs the room.
    pub fn estimate_size(&self) -> usize {
        self.mime.len() + self.data.len()
    }
}

/// One emoji bucket on a message: which emoji, and who is in it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Reaction {
    /// Always a member of [`crate::emoji::REACTION_EMOJI`].
    pub emoji: String,
    /// How many people are in this bucket.
    pub count: usize,
    /// Whether the recipient is one of them, filled in per client as history is
    /// sent, so no user's reactor list reaches another.
    pub reacted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OutgoingMessage {
    pub message_id: Uuid,
    /// Present so a client can identify its own messages without guessing from
    /// the animal name, which is not unique across time.
    pub user_id: String,
    pub animal_name: String,
    /// Already rendered from Markdown and sanitised. Never raw user input.
    pub text: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

/// A history message with the reactions this viewer should see.
///
/// Borrowed, and flattened so the frame matches a live `Message`. Reactions
/// aren't a field of [`OutgoingMessage`] because "did you react" differs per
/// recipient, and storing it would mean copying the message per viewer.
#[derive(Serialize)]
pub struct HistoryMessage<'a> {
    #[serde(flatten)]
    pub message: &'a OutgoingMessage,
    #[serde(skip_serializing_if = "<[Reaction]>::is_empty")]
    pub reactions: &'a [Reaction],
}

/// The history replay frame. Same `type` tag and shape as
/// [`OutgoingEvent::Message`], so the client needs no second code path.
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum HistoryEvent<'a> {
    Message { message: HistoryMessage<'a> },
}

impl OutgoingMessage {
    /// Approximate heap cost, for the memory ceiling. Deliberately an
    /// over-estimate: [`ESTIMATED_MESSAGE_SIZE`] covers allocator and `Vec`
    /// overhead, and under-counting risks an OOM kill that ends every session.
    pub fn estimate_size(&self) -> usize {
        let reply_size = self.reply_to.as_ref().map_or(0, |r| {
            r.message_id.len() + r.author_name.len() + r.preview_text.len()
        });

        self.user_id.len()
            + self.animal_name.len()
            + self.text.len()
            + self.timestamp.len()
            + size_of::<Uuid>()
            + reply_size
            + self
                .attachment
                .as_ref()
                .map_or(0, Attachment::estimate_size)
            + ESTIMATED_MESSAGE_SIZE
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SystemEvent {
    UserJoined {
        user_id: String,
        animal_name: String,
    },
    UserLeft {
        user_id: String,
        animal_name: String,
    },
    Typing {
        user_id: String,
        animal_name: String,
        is_typing: bool,
    },
    ReadReceipt {
        user_id: String,
        animal_name: String,
        message_id: Uuid,
    },
    ServerShutdown {
        reason: String,
    },
    /// One person's reaction to one message changed.
    ///
    /// A delta, not a re-send: O(1) to apply, on the wire too. `count` is the new
    /// total, so a client that missed a delta still converges.
    Reaction {
        message_id: Uuid,
        user_id: String,
        emoji: String,
        /// True when the user just added the reaction, false when they removed
        /// it — reacting is a toggle.
        active: bool,
        count: usize,
    },
}

/// Server → client.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum OutgoingEvent {
    Message {
        message: OutgoingMessage,
    },
    System {
        event: SystemEvent,
    },
    UserCount {
        count: usize,
    },
    Heartbeat,
    ReconnectToken {
        token: String,
    },
    /// Who is in the room, by name, sent on request.
    ///
    /// Pushing the roster to everyone on every join is O(users) per recipient for a
    /// panel almost nobody opens; the client asks on open and keeps it current from
    /// `UserJoined`/`UserLeft`. Names only: a client has no use for others' ids
    /// (§5.6).
    Roster {
        users: Vec<String>,
    },
    /// Sent first on every connection: tells the client who it is.
    ///
    /// The client used to assume the first `UserJoined` was itself, or parse
    /// `document.cookie`, which kept the cookies from being `HttpOnly`.
    Welcome {
        user_id: String,
        animal_name: String,
    },
    /// `nova` only (`session/nova.rs`): the server's X3DH prekey bundle
    /// (`PreKeyBundle::to_bytes`, base64), to the asker alone (§5.15).
    ///
    /// This sets up the connection's pairwise channel, used for sending and for
    /// replies owed to one connection. Joining the room's shared `Group` for
    /// broadcasts is a second handshake ([`OutgoingEvent::NovaWelcome`]); a nova
    /// connection completes both.
    NovaPreKeyBundleResponse {
        bundle: String,
    },
    /// `nova` only: the `Welcome` and `Commit` that admit a joiner to the room's
    /// `Group`, to the asker alone. `welcome` (`group::Welcome::to_bytes`, base64)
    /// is sealed to the joiner's `LeafKeyPackage`. `commit` is the same bytes
    /// broadcast to existing members as [`OutgoingEvent::NovaCommit`], which the
    /// joiner can't yet receive.
    NovaWelcome {
        welcome: String,
        commit: String,
    },
    /// `nova` only: an admitted membership change (add, remove, self-update),
    /// broadcast so members `apply_commit` and reach the new epoch. `commit` is
    /// `group::Commit::to_bytes`, base64, unsealed: its per-node path-secret
    /// ciphertexts already limit who can use it.
    NovaCommit {
        commit: String,
    },
    /// `nova` only: a frame sealed under this connection's pairwise ratchet,
    /// base64; `data` decrypts to another `OutgoingEvent`.
    ///
    /// Only for replies owed to one connection, never broadcasts
    /// ([`OutgoingEvent::GroupSealed`]): a group ratchet's per-sender chain has no
    /// "for one recipient" ciphertext, and sealing one under it would desync every
    /// other member's sequence number for the server's leaf (`session/nova.rs`).
    Sealed {
        data: String,
    },
    /// `nova` only: a broadcast frame sealed once under the room's `Group`
    /// (`session/nova.rs::run_seal_loop`), base64; `data` decrypts to another
    /// `OutgoingEvent`. Every connection receives identical bytes, relayed verbatim
    /// by `session/tasks.rs::forward_broadcasts`, because every member derives the
    /// same per-sender chain: one seal, not one per recipient.
    GroupSealed {
        data: String,
    },
    /// `nova` only (`session/nova_rln.rs`): this connection's permanent leaf index
    /// in the RLN membership tree.
    RlnRegistered {
        leaf_index: usize,
    },
    /// `nova` only: this leaf's current Merkle path, fetched before every
    /// anonymous post because it changes whenever anyone registers
    /// (`session/nova_rln.rs`).
    RlnPathResponse {
        path: Vec<RlnPathStep>,
        root: String,
    },
    /// `nova` only: an anonymous, rate-limited message. Having no
    /// `user_id`/`animal_name` is the point of an RLN proof.
    NovaAnonymousMessage {
        message_id: Uuid,
        text: String,
        timestamp: String,
    },
    /// `nova` only: a member posted two different anonymous messages in one
    /// epoch, which lets anyone holding both proofs (this server included) recover
    /// their identity secret. Broadcast so viewers see the mechanism work.
    NovaRlnSlashed {
        recovered_secret: String,
    },
    /// `nova` only (`session/nova_operator.rs`): one DKG, threshold-decryption and
    /// FROST-signing round against the connected `nova-operator` quorum; the
    /// coordinator never holds their shares. Broadcast, since nothing in it is
    /// per-connection or secret.
    NovaMpcDemoResult {
        num_operators: u32,
        threshold: u32,
        quorum: Vec<u32>,
        group_public_key: String,
        recovered_key_matches: bool,
        /// What the quorum's FROST signature attests to — the room's
        /// current RLN membership root at the moment this ran.
        signed_message: String,
        signature_valid: bool,
    },
    /// `nova` only: fewer than `needed` operators are connected. There is no
    /// simulation to fall back to.
    NovaMpcQuorumUnavailable {
        live: u32,
        needed: u32,
    },
}

/// One step of an RLN Merkle path, mirrored as hex/tag rather than re-exporting
/// [`novachannel_rln::merkle::PathStep`], so this file stays the one definition
/// of what crosses the socket.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RlnPathStep {
    pub sibling: String,
    pub side: RlnSide,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum RlnSide {
    Left,
    Right,
}

/// Client → server.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClientEvent {
    #[serde(rename = "Message")]
    Message {
        text: String,
        #[serde(default)]
        reply_to: Option<ReplyInfo>,
        /// An image to send. A message may be an image with no text, so `text` alone
        /// doesn't decide whether there is anything to send.
        #[serde(default)]
        attachment: Option<Attachment>,
    },
    #[serde(rename = "Typing")]
    Typing { is_typing: bool },
    #[serde(rename = "ReadReceipt")]
    ReadReceipt { message_id: String },
    /// Toggle this user's reaction on a message.
    #[serde(rename = "React")]
    React { message_id: String, emoji: String },
    /// Ask who is in the room. Answered to the asker alone.
    #[serde(rename = "RequestRoster")]
    RequestRoster,
    /// `nova` only: asks for the server's X3DH prekey bundle, right after
    /// `Welcome`. No payload; there is one bundle. Sets up the pairwise channel
    /// (see `OutgoingEvent::NovaPreKeyBundleResponse`).
    #[serde(rename = "NovaPreKeyBundleRequest")]
    NovaPreKeyBundleRequest,
    /// `nova` only: the X3DH init message (`x3dh::InitMessage::bytes`, base64)
    /// built from `NovaPreKeyBundleResponse`. Completes the pairwise session
    /// server-side; the client already has its half and waits for nothing.
    #[serde(rename = "NovaX3dhInit")]
    NovaX3dhInit { message: String },
    /// `nova` only: asks to join the room's `Group`, the separate handshake for
    /// receiving broadcasts (`OutgoingEvent::GroupSealed`). `key_package` is the
    /// public `LeafKeyPackage::to_bytes`, base64; the private half never leaves the
    /// browser.
    #[serde(rename = "NovaJoinRequest")]
    NovaJoinRequest { key_package: String },
    /// `nova` only: a `ClientEvent` sealed under this connection's pairwise
    /// ratchet, base64, mirroring [`OutgoingEvent::Sealed`]. It is dispatched as if
    /// it had arrived unsealed.
    #[serde(rename = "Sealed")]
    Sealed { data: String },
    /// `nova` only: registers this connection's RLN identity commitment, once;
    /// `session/nova_rln.rs` replies with a permanent leaf index.
    #[serde(rename = "RlnRegister")]
    RlnRegister { commitment: String },
    /// `nova` only: asks for this leaf's current Merkle path, fresh before every
    /// anonymous post (`session/nova_rln.rs`).
    #[serde(rename = "RlnPathRequest")]
    RlnPathRequest { leaf_index: usize },
    /// `nova` only: an anonymous, rate-limited message. `proof` is the base64 STARK
    /// proof; `y` and `nullifier` are its RLN share (hex of big-endian field
    /// elements), which the server can't recompute without the secret the proof
    /// hides. The membership root, epoch and message binding `x` are recomputed
    /// server-side, never taken from the client (`session/nova_rln.rs`).
    #[serde(rename = "RlnMessage")]
    RlnMessage {
        proof: String,
        y: String,
        nullifier: String,
        text: String,
    },
    /// `nova` only: cover traffic from `client.js`'s `novachannel-dp` scheduler.
    /// `padding` brings a sealed dummy closer to a real frame's size, since a size
    /// difference would distinguish them. At the positive epsilon the client uses
    /// this is cover traffic, not differential privacy: a silent slot still shows
    /// nothing was queued. The server discards it (`session/nova.rs::dispatch`).
    #[serde(rename = "Dummy")]
    Dummy { padding: String },
    /// `nova` only: run one DKG/FROST demonstration round
    /// (`session/nova_operator.rs`). No payload: every parameter is fixed, so a
    /// client has nothing to lie about.
    #[serde(rename = "NovaMpcDemoRequest")]
    NovaMpcDemoRequest,
}
