//! The WebSocket wire format — every type that crosses the socket.
//!
//! Kept in one module with no dependency on server state, so the contract the
//! browser codes against is readable in a single file. Changing anything here
//! is a protocol change: `index.html` must change with it, in the same commit.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::config::ESTIMATED_MESSAGE_SIZE;

/// Serialises a frame the server is about to send.
///
/// Every outgoing type is a plain struct or enum of owned strings, numbers and
/// UUIDs — no map with non-string keys, no float that could be NaN — so this
/// cannot fail. Saying that in one place, once, is better than an unreachable
/// error arm at each call site, each of which a reader has to work out is
/// unreachable for themselves.
pub(crate) fn encode_event<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| {
        // Reached only if an outgoing type gains a field that cannot be
        // represented. An empty frame is dropped by the client; a panic here
        // would take the whole connection task with it (§5.1).
        error!(error = %e, "failed to serialise an outgoing frame");
        String::new()
    })
}

/// Serialises a broadcast frame once, for every connection in the room to
/// share.
///
/// A room's broadcast channel carries the encoded frame itself, not the
/// event — `tokio::sync::broadcast::Receiver::recv` clones its value out for
/// every receiver independently, so a channel of an unencoded event meant
/// every connected user's forward task re-ran `serde_json::to_string` on
/// byte-identical output, and re-cloned the event's owned fields — a
/// message's text, an attachment's base64 payload — doing it. In a full room
/// (`MAX_USERS_PER_ROOM`) sending an image message
/// (`MAX_ATTACHMENT_BYTES`), that was up to a hundred redundant clones and a
/// hundred redundant serialisations of the same ~128 KiB payload, on the
/// single OS thread this server runs everything on (`main.rs`'s
/// `current_thread` runtime) — squarely the O(room size) per-message cost
/// §1 says a message must never have. `Arc<str>` makes every receiver's
/// share of that cost a refcount bump instead.
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
/// The bytes travel *in* the message rather than behind a URL, because there is
/// nowhere to put a file: no object store, no database, and no origin the CSP
/// would let the browser fetch from (§5.7). The client renders it as
/// `data:{mime};base64,{data}`, which is exactly what `img-src 'self' data:`
/// permits and nothing more.
///
/// `data` is the base64 payload alone, without the `data:` prefix — the prefix
/// is reconstructed by the client from `mime`, so a client cannot smuggle a
/// second scheme past the type check by writing its own.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Attachment {
    /// One of [`crate::config::ALLOWED_ATTACHMENT_MIMES`], verified against the
    /// payload's magic bytes rather than believed.
    pub mime: String,
    /// Base64, no prefix. Empty once the image has faded from the room
    /// (`faded` is then true) — the message survives, the picture does not.
    pub data: String,
    /// Display hints so the client can reserve space before the image decodes,
    /// which is what keeps an arriving image from shoving the conversation down
    /// the page. Bounded, not trusted.
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
    /// Whether *the recipient* is one of them. Filled in per client as history
    /// is sent, so the server never ships one user's roster of reactors to
    /// another — the client only ever needs to know its own state.
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

/// A history message together with the reactions *this viewer* should see.
///
/// Borrowed, and flattened onto the wire so the frame is byte-identical to a
/// live `Message`. Reactions are not a field of [`OutgoingMessage`] because a
/// stored message has no single answer to "did you react" — the answer differs
/// per recipient, and putting it in the stored struct would mean copying the
/// whole message per viewer to fill it in.
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
    /// Approximate heap cost of this message, used by the memory ceiling.
    ///
    /// Deliberately an over-estimate: [`ESTIMATED_MESSAGE_SIZE`] covers
    /// allocator and `Vec` overhead that the string lengths alone miss. Under-
    /// counting here would let the process exceed the container memory limit
    /// and be killed, which costs every connected user their session.
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
    /// A delta, not a re-send of the message: a reaction is O(1) to apply and
    /// must stay that way on the wire too. `count` is the bucket's new total so
    /// a client that missed an earlier delta still converges.
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
    /// Who is in the room, by name.
    ///
    /// Sent on request rather than broadcast with every join. Pushing the whole
    /// roster to everyone whenever anybody arrives is O(users) per recipient —
    /// quadratic in the size of the room, for a panel almost nobody has open.
    /// The client asks when it opens the list and keeps it current from the
    /// `UserJoined` and `UserLeft` events it already receives.
    ///
    /// Names only. A client has no use for anybody else's id, and not sending
    /// it is the same reasoning as resolving `reacted` per viewer (§5.6).
    Roster {
        users: Vec<String>,
    },
    /// Sent once, first, on every connection: tells the client who it is.
    ///
    /// Before this existed the client inferred its own identity by assuming the
    /// first `UserJoined` it saw was itself, and otherwise by parsing
    /// `document.cookie` — which is why the identity cookies could not be
    /// `HttpOnly`. An explicit identity event is what allows them to be.
    Welcome {
        user_id: String,
        animal_name: String,
    },
    /// `nova` room only (`session/nova.rs`): the server's published X3DH
    /// prekey bundle (`novachannel::prekey::PreKeyBundle::to_bytes`,
    /// base64), sent to the connection that asked. Reply-to-asker, the
    /// same pattern as [`OutgoingEvent::Roster`] and for the same reason
    /// (§5.15) — a bundle fetch belongs to one connection, and the room's
    /// broadcast channel would hand it to everybody. The client calls
    /// `x3dh::initiate` against this locally and completes its side of the
    /// session in that one call — X3DH has no second server round trip the
    /// way the handshake this replaced did.
    NovaPreKeyBundleResponse {
        bundle: String,
    },
    /// `nova` room only: every frame this connection would otherwise have
    /// received, `novachannel`-sealed under that connection's own ratchet
    /// and base64-encoded. `data` decrypts to another `OutgoingEvent` —
    /// this is a transport wrapper, not a new event shape, so nothing about
    /// `Message`/`System`/etc. changes for `nova` versus any other room.
    Sealed {
        data: String,
    },
    /// `nova` room only (`session/nova_rln.rs`): this connection's leaf
    /// index in the room's RLN membership tree, assigned once at
    /// registration and permanent for the room's lifetime.
    RlnRegistered {
        leaf_index: usize,
    },
    /// `nova` room only: this leaf's current Merkle authentication path —
    /// requested fresh before every anonymous post rather than cached,
    /// since it changes whenever another member registers (§ design note
    /// in `session/nova_rln.rs`).
    RlnPathResponse {
        path: Vec<RlnPathStep>,
        root: String,
    },
    /// `nova` room only: an anonymously-posted, rate-limited message. No
    /// `user_id`/`animal_name` at all — that omission *is* the point of an
    /// RLN proof, not a field left blank.
    NovaAnonymousMessage {
        message_id: Uuid,
        text: String,
        timestamp: String,
    },
    /// `nova` room only: a member posted a second, *different* anonymous
    /// message inside one rate-limit epoch, which is exactly the condition
    /// under which RLN's Shamir-style share leaks their identity secret to
    /// anyone holding both proofs — this server included. Broadcast so
    /// every viewer can see the mechanism work, not silently logged.
    NovaRlnSlashed {
        recovered_secret: String,
    },
    /// `nova` room only (`session/nova_operator.rs`): the result of one
    /// DKG, threshold-decryption, and FROST-signing round against a real,
    /// currently-connected quorum of `nova-operator` processes — the
    /// coordinator (this server) never held any of their secret shares.
    /// Broadcast to the whole room, since nothing in it is per-connection
    /// state or a secret belonging to whoever asked.
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
    /// `nova` room only (`session/nova_operator.rs`): fewer than
    /// `needed` real `nova-operator` processes are currently connected to
    /// run a demo round. Distinct from silently falling back to a
    /// simulation — there is no simulation to fall back to any more.
    NovaMpcQuorumUnavailable {
        live: u32,
        needed: u32,
    },
}

/// One step of an RLN Merkle authentication path, wire-shaped:
/// [`novachannel_rln::merkle::PathStep`] mirrored as hex/tag rather than the
/// library's own type, so `protocol.rs` stays the one place that defines
/// what crosses the socket (this file's own module doc) rather than
/// re-exporting a cryptography crate's internal representation.
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
        /// An image to send with the message. A message may be an image with no
        /// text at all, which is why `text` alone no longer decides whether
        /// there is anything to send.
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
    /// `nova` room only: asks for the server's X3DH prekey bundle, sent by
    /// the client immediately after receiving `Welcome`. No payload — the
    /// server has exactly one bundle to offer.
    #[serde(rename = "NovaPreKeyBundleRequest")]
    NovaPreKeyBundleRequest,
    /// `nova` room only: the X3DH init message
    /// (`novachannel::x3dh::InitMessage::bytes`, base64) produced by the
    /// client's local `x3dh::initiate` call against
    /// `NovaPreKeyBundleResponse`. Completes the session server-side in
    /// one step — unlike the synchronous handshake this replaced, there is
    /// no further reply the client waits for; it already has its own
    /// session the moment it built this message.
    #[serde(rename = "NovaX3dhInit")]
    NovaX3dhInit { message: String },
    /// `nova` room only: a `novachannel`-sealed, base64-encoded
    /// `ClientEvent` — the client-to-server mirror of
    /// [`OutgoingEvent::Sealed`]. `data` decrypts to another `ClientEvent`,
    /// dispatched exactly as if it had arrived unsealed; every other room
    /// never sends this variant.
    #[serde(rename = "Sealed")]
    Sealed { data: String },
    /// `nova` room only: registers this connection's RLN identity
    /// commitment in the room's membership tree. Once per connection —
    /// `session/nova_rln.rs` assigns a permanent leaf index in reply.
    #[serde(rename = "RlnRegister")]
    RlnRegister { commitment: String },
    /// `nova` room only: asks for this leaf's *current* Merkle path.
    /// Requested fresh before every anonymous post, not cached — see
    /// `session/nova_rln.rs`'s design note on why the path changes whenever
    /// another member registers.
    #[serde(rename = "RlnPathRequest")]
    RlnPathRequest { leaf_index: usize },
    /// `nova` room only: an anonymous, rate-limited message. `proof` is the
    /// base64-encoded STARK proof; `y`/`nullifier` are the RLN share this
    /// proof carries (hex of each field element's big-endian bytes) — the
    /// two values the server cannot recompute itself, since deriving them
    /// needs the secret key this proof exists specifically not to reveal.
    /// Everything else the proof is checked against — the room's current
    /// membership root, the current rate-limit epoch, and the message
    /// binding `x` — the server recomputes independently rather than
    /// trusting a client-supplied copy (`session/nova_rln.rs`).
    #[serde(rename = "RlnMessage")]
    RlnMessage {
        proof: String,
        y: String,
        nullifier: String,
        text: String,
    },
    /// `nova` room only: cover traffic (`novachannel-dp`, driven by
    /// `client.js`'s periodic scheduler tick). Carries no content that
    /// matters — `padding` only exists so a sealed dummy frame is close in
    /// size to a sealed real one, since the differential-privacy guarantee
    /// is about the *decision to transmit*, not the frame's contents, and
    /// is void if a size difference lets an observer tell them apart
    /// anyway. The server does nothing with this but discard it: nobody
    /// downstream of admission ever sees it (`session/nova.rs::dispatch`).
    #[serde(rename = "Dummy")]
    Dummy { padding: String },
    /// `nova` room only: run one FROST/DKG protocol-demonstration round
    /// (`session/nova_mpc.rs`). No payload — every parameter of the demo
    /// is fixed, on purpose, so there is nothing here for a client to lie
    /// about.
    #[serde(rename = "NovaMpcDemoRequest")]
    NovaMpcDemoRequest,
}
