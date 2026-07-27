//! A single chat room: its users, its history, and its broadcast channel.
//!
//! Everything here runs while the caller holds the room lock, which every
//! connected user in that room contends on. That makes the cost of each method
//! a latency budget rather than a micro-optimisation: work proportional to
//! history length must run rarely, and work on the message path must be O(1).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime};

use rand::prelude::SliceRandom;
use tokio::sync::broadcast;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::animals::ANIMAL_NAMES;
use crate::config::{
    CLEANUP_BATCH_SIZE, HEARTBEAT_TIMEOUT, MAX_MESSAGE_AGE, MAX_MESSAGES_PER_ROOM,
    MAX_TOTAL_ROOMS_MEMORY, MAX_USERS_PER_ROOM, MEMORY_SOFT_LIMIT_RATIO, USER_IDLE_MESSAGE_TIMEOUT,
};
use crate::limits::{MemoryTracker, RateLimiter};
use crate::protocol::{OutgoingEvent, OutgoingMessage, SystemEvent};

/// Broadcast channel depth. A slow client that falls this far behind is lagged
/// off the channel rather than allowed to grow the server's memory.
const BROADCAST_CHANNEL_CAPACITY: usize = 1000;

/// History kept beyond [`MAX_MESSAGES_PER_ROOM`] before a trim is worth the
/// lock time. Trimming on every message would be O(n) per message.
const HISTORY_TRIM_SLACK: usize = 100;

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
    pub rate_limiter: RateLimiter,
    pub last_sanitized_message: Option<(String, Instant)>,
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
    pub chat_history: Vec<OutgoingMessage>,
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
    animals.shuffle(&mut rand::thread_rng());

    let (sender, _) = broadcast::channel::<OutgoingEvent>(BROADCAST_CHANNEL_CAPACITY);

    RoomState {
        last_activity: Instant::now(),
        total_memory_bytes: AtomicUsize::new(0),
        users: HashMap::new(),
        available_animals: animals.into_iter().map(String::from).collect(),
        sender,
        chat_history: Vec::new(),
    }
}

impl RoomState {
    /// Connected (not merely known) users.
    pub fn connected_user_count(&self) -> usize {
        self.users.values().filter(|u| u.is_connected()).count()
    }

    /// Takes an unused name from the pool.
    ///
    /// Builds the in-use set once (O(users)) and then scans the pool once
    /// (O(pool)). The previous form re-scanned every user for every candidate
    /// name, which is O(pool × users) — with a full room and a full pool that
    /// is ~25,000 string comparisons while holding the room write lock.
    pub fn assign_animal(&mut self) -> String {
        let taken: HashSet<&str> = self
            .users
            .values()
            .filter(|u| u.is_connected())
            .map(|u| u.animal_name.as_str())
            .collect();

        // Rotate the pool at most once. `pop_front` cannot fail inside this
        // bound, so it is unwrapped rather than guarded by a branch nothing
        // could ever exercise.
        for _ in 0..self.available_animals.len() {
            let animal = self
                .available_animals
                .pop_front()
                .expect("pool length was just measured");

            if !taken.contains(animal.as_str()) {
                trace!(animal = %animal, "assigned animal name");
                return animal;
            }
            self.available_animals.push_back(animal);
        }

        // Pool exhausted: every name is held by a connected user.
        let name = format!("guest_{}", self.users.len() + 1);
        debug!(name = %name, "animal pool exhausted, assigned guest name");
        name
    }

    /// Appends a message, pruning first if it would breach the memory ceiling.
    pub fn add_message(&mut self, msg: OutgoingMessage, memory_tracker: &MemoryTracker) {
        self.last_activity = Instant::now();
        let msg_size = msg.estimate_size();

        let current_memory = self.total_memory_bytes.load(Ordering::Relaxed);
        if current_memory + msg_size > MAX_TOTAL_ROOMS_MEMORY {
            self.prune_old_messages(msg_size, memory_tracker);
        }

        if !memory_tracker.add_bytes(msg_size) {
            warn!("dropping message: global memory budget exhausted");
            return;
        }

        self.total_memory_bytes
            .fetch_add(msg_size, Ordering::SeqCst);
        self.chat_history.push(msg);
    }

    /// Drops oldest messages until at least `needed_space` bytes are free.
    ///
    /// Counts the messages to drop, then removes them in one `drain`. The
    /// previous form called `Vec::remove(0)` in a loop, which shifts the whole
    /// remaining history on every iteration — O(n²) in the number pruned, on
    /// the message path, under the room write lock.
    pub fn prune_old_messages(&mut self, needed_space: usize, memory_tracker: &MemoryTracker) {
        let mut removed_size = 0;
        let mut drop_count = 0;

        for msg in &self.chat_history {
            if removed_size >= needed_space {
                break;
            }
            removed_size += msg.estimate_size();
            drop_count += 1;
        }

        if drop_count == 0 {
            return;
        }

        self.chat_history.drain(..drop_count);

        let current = self.total_memory_bytes.load(Ordering::SeqCst);
        self.total_memory_bytes
            .store(current.saturating_sub(removed_size), Ordering::SeqCst);
        memory_tracker.remove_bytes(removed_size);
    }

    /// Drops messages older than [`MAX_MESSAGE_AGE`], at most
    /// [`CLEANUP_BATCH_SIZE`] per pass.
    ///
    /// Retains in place. The previous form cloned every *surviving* message
    /// into a second vector — allocating a copy of the entire history in order
    /// to delete a handful of entries from it.
    pub async fn cleanup_messages(&mut self, _now: Instant, memory_tracker: &MemoryTracker) {
        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let mut removed = 0usize;
        let mut removed_bytes = 0usize;

        self.chat_history.retain(|msg| {
            if removed >= CLEANUP_BATCH_SIZE {
                return true;
            }

            // A timestamp that will not parse is kept: an accounting bug must
            // not silently delete somebody's message.
            let Ok(msg_time_ms) = msg.timestamp.parse::<u128>() else {
                return true;
            };

            if current_time_ms.saturating_sub(msg_time_ms) <= MAX_MESSAGE_AGE.as_millis() {
                return true;
            }

            removed += 1;
            removed_bytes += msg.estimate_size();
            false
        });

        if removed > 0 {
            self.recompute_memory();
            memory_tracker.remove_bytes(removed_bytes);
        }
    }

    /// Trims history once it has drifted [`HISTORY_TRIM_SLACK`] past the cap.
    ///
    /// The slack is what keeps this off the message path: trimming exactly at
    /// the cap would move the whole history on every single message.
    pub fn preserve_messages(&mut self, memory_tracker: &MemoryTracker) {
        if self.chat_history.len() > MAX_MESSAGES_PER_ROOM + HISTORY_TRIM_SLACK {
            self.retain_newest(MAX_MESSAGES_PER_ROOM, memory_tracker);
        }
    }

    /// Trims history to exactly the cap. Used on join, where the cost is paid
    /// once by the joining user rather than by every message.
    pub fn trim_to_max_messages(&mut self, memory_tracker: &MemoryTracker) {
        if self.chat_history.len() > MAX_MESSAGES_PER_ROOM {
            self.retain_newest(MAX_MESSAGES_PER_ROOM, memory_tracker);
        }
    }

    /// Keeps the newest `keep` messages, releasing the rest from both the room
    /// and the global tracker.
    pub fn retain_newest(&mut self, keep: usize, memory_tracker: &MemoryTracker) {
        if self.chat_history.len() <= keep {
            return;
        }

        let drop_count = self.chat_history.len() - keep;
        let removed_bytes: usize = self.chat_history[..drop_count]
            .iter()
            .map(OutgoingMessage::estimate_size)
            .sum();

        self.chat_history.drain(..drop_count);
        self.recompute_memory();

        if removed_bytes > 0 {
            memory_tracker.remove_bytes(removed_bytes);
        }
    }

    /// Recomputes this room's byte total from its history.
    ///
    /// O(history), so it runs only inside a trim that was already O(history).
    /// Recomputing rather than subtracting is deliberate: it is self-healing,
    /// so an accounting slip anywhere else is corrected at the next trim
    /// instead of accumulating until the room wrongly reports itself full.
    fn recompute_memory(&self) {
        let total: usize = self
            .chat_history
            .iter()
            .map(OutgoingMessage::estimate_size)
            .sum();
        self.total_memory_bytes.store(total, Ordering::SeqCst);
    }

    /// Broadcasts the number of users whose heartbeat is still current.
    pub fn broadcast_user_count(&self) {
        let now = Instant::now();
        let count = self
            .users
            .values()
            .filter(|u| match &u.connection_state {
                ConnectionState::Connected { last_heartbeat, .. } => {
                    now.duration_since(*last_heartbeat) <= HEARTBEAT_TIMEOUT
                }
                ConnectionState::Disconnected { .. } => false,
            })
            .count();

        // A send error means nobody is subscribed, which is not an error.
        let _ = self.sender.send(OutgoingEvent::UserCount { count });
    }

    pub fn broadcast_system_event(&self, event: SystemEvent) {
        trace!(?event, "broadcasting system event");
        let _ = self.sender.send(OutgoingEvent::System { event });
    }

    /// Admission check for a user about to join.
    ///
    /// Returns `false` when the room is at capacity, or when a known user is
    /// reconnecting faster than [`crate::config::MAX_ROOM_JOIN_ATTEMPTS`]
    /// allows.
    pub fn is_user_allowed(&mut self, user_id: &str) -> bool {
        if self.connected_user_count() >= MAX_USERS_PER_ROOM {
            return false;
        }

        match self.users.get_mut(user_id) {
            Some(user) => user.rate_limiter.can_join_room(),
            None => true,
        }
    }

    /// Prunes aged messages once the room passes the soft memory threshold.
    pub async fn trigger_cleanup(&mut self, memory_tracker: &MemoryTracker) {
        let (num, den) = MEMORY_SOFT_LIMIT_RATIO;
        if self.total_memory_bytes.load(Ordering::Relaxed) > (MAX_TOTAL_ROOMS_MEMORY * num) / den {
            self.cleanup_messages(Instant::now(), memory_tracker).await;
        }
    }
}
