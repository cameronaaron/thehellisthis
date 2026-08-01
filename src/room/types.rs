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
use crate::protocol::{OutgoingEvent, OutgoingMessage};

/// Broadcast channel depth. A slow client that falls this far behind is lagged
/// off the channel rather than allowed to grow the server's memory.
const BROADCAST_CHANNEL_CAPACITY: usize = 1000;

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
    /// The last text this user sent, as it arrived, and when.
    ///
    /// Used only to recognise a double-send. It was called
    /// `last_sanitized_message`, which it never held: the comparison is against
    /// the raw client text, before rendering. A name that claims a string has
    /// been through the sanitiser is the one name it must not have on a server
    /// whose job is turning user Markdown into HTML — the next person to reach
    /// for it would have had no reason to sanitise it again (§8).
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
    pub sender: broadcast::Sender<OutgoingEvent>,
    /// Messages are immutable once stored, so history holds `Arc`s and the
    /// join path copies refcounts rather than payloads.
    ///
    /// Measured on a full room (500 messages, ~2 MB of attachments): cloning
    /// the owned history took **119 µs**, and it was done *under the room write
    /// lock* that every user in every room contends on. The same copy as
    /// `Arc` clones is **1.3 µs** — the single largest lock hold in the server,
    /// cut by ~90x, for a type change.
    pub chat_history: Vec<Arc<OutgoingMessage>>,
    /// Who has reacted to what, keyed by message id.
    ///
    /// Beside the history rather than inside it, because a reaction arrives as
    /// its own event and applying it must be O(1) (§1.1). Held in the message
    /// it belongs to, finding that message would be a scan of the history on
    /// every reaction — linear work on a per-event path.
    ///
    /// Keyed by message id, so it needs an eviction path like any other
    /// unbounded map (§3.5): `forget_reactions_for` runs from every place that
    /// removes messages from `chat_history`, and
    /// `reactions_never_outlive_the_messages_they_belong_to` fails if a new
    /// removal path forgets to call it.
    pub reactions: HashMap<Uuid, HashMap<String, HashSet<String>>>,
    /// The ids of the messages in `chat_history`.
    ///
    /// Exists so `toggle_reaction` can answer "is there such a message" in O(1)
    /// without scanning the history. Without it, a reaction was accepted for
    /// *any* uuid: a client sending `React` frames with random ids — which the
    /// 100 ms throttle still allows ten times a second — grew `reactions`
    /// without limit, storing buckets for messages that never existed and that
    /// no removal path could ever clean up, none of it visible to the memory
    /// ceiling.
    ///
    /// A derived copy, which §1.4a warns about, so it is maintained in exactly
    /// the places that already maintain the reaction map and rebuilt by
    /// `recompute_memory` — the same self-healing the byte totals get (§3.4).
    pub message_ids: HashSet<Uuid>,
    /// Running total of attachment payload bytes still held in `chat_history`.
    ///
    /// Tracked rather than recomputed so the fade check on the message path is
    /// O(1); `recompute_memory` puts it back in step whenever it runs (§3.4).
    pub attachment_bytes: usize,
}

/// True once a user has gone [`USER_IDLE_MESSAGE_TIMEOUT`] without speaking.
///
/// This is a game mechanic, not a resource limit: the user count is meant to
/// read "people actually here" (ENGINEERING-STANDARDS.md §7).
pub fn user_idle_for_too_long(user: &UserData, now: Instant) -> bool {
    now.duration_since(user.last_message_time) >= USER_IDLE_MESSAGE_TIMEOUT
}

/// Builds an empty room with a freshly shuffled name pool.
///
/// The shuffle is per room so two rooms do not hand out names in the same
/// order; it is the one linear cost here, and it runs once per room ever.
pub fn create_room() -> RoomState {
    let mut animals: Vec<&'static str> = ANIMAL_NAMES.to_vec();
    animals.shuffle(&mut rand::rng());

    let (sender, _) = broadcast::channel::<OutgoingEvent>(BROADCAST_CHANNEL_CAPACITY);

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
