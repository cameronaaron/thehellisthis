//! The WebSocket wire format — every type that crosses the socket.
//!
//! Kept in one module with no dependency on server state, so the contract the
//! browser codes against is readable in a single file. Changing anything here
//! is a protocol change: `index.html` must change with it, in the same commit.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ESTIMATED_MESSAGE_SIZE;

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
    /// Populated only when history is sent; live reaction changes arrive as
    /// [`SystemEvent::Reaction`] deltas rather than as a whole message again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
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
}
