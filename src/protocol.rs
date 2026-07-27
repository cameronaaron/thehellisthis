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
    },
    #[serde(rename = "Typing")]
    Typing { is_typing: bool },
    #[serde(rename = "ReadReceipt")]
    ReadReceipt { message_id: String },
}
