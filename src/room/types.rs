//! `RoomState` and `UserData` themselves: the fields every other file's
//! `impl` block extends, and how a room is born.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use rand::seq::SliceRandom;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::animals::ANIMAL_NAMES;
use crate::config::USER_IDLE_MESSAGE_TIMEOUT;
use crate::limits::RateLimiter;
use crate::protocol::OutgoingMessage;

/// Broadcast channel depth. A client this far behind is lagged off the
/// channel rather than allowed to grow server memory. Also the depth of
/// `state.rs`'s `nova_sealed_sender`, the channel nova connections subscribe
/// to.
pub(crate) const BROADCAST_CHANNEL_CAPACITY: usize = 1000;

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connected {
        last_heartbeat: Instant,
        connection_id: String,
    },
    Disconnected {
        since: Instant,
    },
}

#[derive(Clone)]
pub struct UserData {
    pub user_id: String,
    pub animal_name: String,
    pub last_active: Instant,
    pub last_message_time: Instant,
    pub connection_state: ConnectionState,
    pub last_read_message: Option<Uuid>,
    pub is_typing: bool,
    pub last_typing_event: Option<Instant>,
    pub last_read_receipt_event: Option<Instant>,
    pub last_reaction_event: Option<Instant>,
    pub rate_limiter: RateLimiter,
    /// The last text this user sent, raw, and when; used only to spot a double
    /// send. It was named `last_sanitized_message`, which it never held. On a
    /// server that turns Markdown into HTML, a name claiming a string was sanitised
    /// is the one it must not have (§8).
    pub last_message_text: Option<(String, Instant)>,
}

impl UserData {
    pub fn is_connected(&self) -> bool {
        matches!(self.connection_state, ConnectionState::Connected { .. })
    }
}

pub struct RoomState {
    pub last_activity: Instant,
    pub total_memory_bytes: AtomicUsize,
    pub users: HashMap<String, UserData>,
    pub available_animals: VecDeque<String>,
    /// Carries the encoded frame, not the event: `recv` clones per receiver, so a
    /// channel of events meant up to `MAX_USERS_PER_ROOM` identical encodes and
    /// payload clones per message. Each receiver now bumps a refcount; frames are
    /// encoded once, in `encode_broadcast`.
    pub sender: broadcast::Sender<Arc<str>>,
    /// Stored messages are immutable, so history holds `Arc`s and joins copy
    /// refcounts. On a full room (500 messages, about 2 MB of attachments) the owned
    /// clone took 119 µs under the room write lock; the `Arc` copy takes 1.3 µs,
    /// the server's largest lock hold cut about 90x by a type change.
    pub chat_history: Vec<Arc<OutgoingMessage>>,
    /// Who reacted with what, by message id: beside the history rather than in
    /// it, so applying a reaction is O(1) instead of a scan (§1.1). It needs an
    /// eviction path (§3.5): `forget_reactions_for` runs wherever messages leave
    /// `chat_history`, and `reactions_never_outlive_the_messages_they_belong_to`
    /// fails if a new path forgets.
    pub reactions: HashMap<Uuid, HashMap<String, HashSet<String>>>,
    /// Ids of the messages in `chat_history`, so `toggle_reaction` can check a
    /// message exists in O(1). Without it a reaction was accepted for any uuid,
    /// and random `React` frames (ten a second under the 100 ms throttle) grew
    /// `reactions` without limit, invisible to the memory ceiling and never
    /// cleaned up. A derived copy (§1.4a's warning), so it is maintained where the
    /// reaction map is and rebuilt by `recompute_memory` (§3.4).
    pub message_ids: HashSet<Uuid>,
    /// Running total of attachment bytes in `chat_history`, tracked so the
    /// per-message fade check is O(1); `recompute_memory` resyncs it (§3.4).
    pub attachment_bytes: usize,
}

/// Whether a user has gone [`USER_IDLE_MESSAGE_TIMEOUT`] without speaking: a
/// game mechanic, so the count means people actually here (§7).
pub fn user_idle_for_too_long(user: &UserData, now: Instant) -> bool {
    now.duration_since(user.last_message_time) >= USER_IDLE_MESSAGE_TIMEOUT
}

/// An empty room with a freshly shuffled name pool, so rooms don't hand out
/// names in the same order. The shuffle is the one linear cost, once per room.
pub fn create_room() -> RoomState {
    let mut animals: Vec<&'static str> = ANIMAL_NAMES.to_vec();
    animals.shuffle(&mut rand::rng());

    let (sender, _) = broadcast::channel::<Arc<str>>(BROADCAST_CHANNEL_CAPACITY);

    RoomState {
        last_activity: Instant::now(),
        total_memory_bytes: AtomicUsize::new(0),
        users: HashMap::new(),
        available_animals: animals.into_iter().map(String::from).collect(),
        sender,
        chat_history: Vec::new(),
        reactions: HashMap::new(),
        message_ids: HashSet::new(),
        attachment_bytes: 0,
    }
}

impl RoomState {
    /// Connected (not merely known) users.
    pub fn connected_user_count(&self) -> usize {
        self.users.values().filter(|u| u.is_connected()).count()
    }
}
