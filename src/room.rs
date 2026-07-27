//! A single chat room: its users, its history, and its broadcast channel.
//!
//! Everything here runs while the caller holds the room lock, which every
//! connected user in that room contends on. That makes the cost of each method
//! a latency budget rather than a micro-optimisation: work proportional to
//! history length must run rarely, and work on the message path must be O(1).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime};

use rand::seq::SliceRandom;
use tokio::sync::broadcast;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::animals::{ANIMAL_NAMES, is_animal_name};
use crate::config::{
    CLEANUP_BATCH_SIZE, HEARTBEAT_TIMEOUT, MAX_MESSAGE_AGE, MAX_MESSAGES_PER_ROOM,
    MAX_REACTIONS_PER_MESSAGE, MAX_ROOM_ATTACHMENT_BYTES, MAX_TOTAL_ROOMS_MEMORY,
    MAX_USERS_PER_ROOM, MEMORY_SOFT_LIMIT_RATIO, USER_IDLE_MESSAGE_TIMEOUT,
};
use crate::limits::{MemoryTracker, RateLimiter};
use crate::protocol::{Attachment, OutgoingEvent, OutgoingMessage, Reaction, SystemEvent};

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
    pub last_reaction_event: Option<Instant>,
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

/// The names currently held by connected users in a room.
///
/// Authoritative: whether a name is free is *derived* from the users, never
/// read from separate bookkeeping that could drift out of step with them.
/// Takes the map rather than the whole room so callers can hold this set while
/// mutating the name pool — they are disjoint fields.
fn names_in_use(users: &HashMap<String, UserData>) -> HashSet<&str> {
    users
        .values()
        .filter(|u| u.is_connected())
        .map(|u| u.animal_name.as_str())
        .collect()
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
        attachment_bytes: 0,
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
    ///
    /// The pool is a **rotation cursor, not a free list**: every name drawn is
    /// pushed to the back whether or not it was handed out, so the pool is
    /// always exactly the roster in this room's own order. It used to be a free
    /// list that callers put names back into by hand, and the bookkeeping did
    /// not balance — a name that was never drawn from *this* room's pool (one
    /// carried in on a cookie, or a `guest_N` fallback) was still pushed back
    /// when its user was reclaimed, so the pool grew on every such reclaim and
    /// accumulated names that were not on the roster. Deriving "free" from
    /// `users` instead deletes the whole class: there is no second copy of the
    /// answer to get out of step.
    pub fn assign_animal(&mut self) -> String {
        // Borrows `users` rather than `self`, so the pool below stays mutable.
        let taken = names_in_use(&self.users);
        let mut chosen = None;

        // Rotate the pool at most once. `pop_front` cannot fail inside this
        // bound, so it is unwrapped rather than guarded by a branch nothing
        // could ever exercise.
        for _ in 0..self.available_animals.len() {
            let animal = self
                .available_animals
                .pop_front()
                .expect("pool length was just measured");
            let free = !taken.contains(animal.as_str());
            self.available_animals.push_back(animal);

            if free {
                chosen = self.available_animals.back().cloned();
                break;
            }
        }

        if let Some(animal) = chosen {
            trace!(animal = %animal, "assigned animal name");
            return animal;
        }

        // Every roster name is held by a connected user. Unreachable while the
        // roster is larger than `MAX_USERS_PER_ROOM`, which
        // `the_roster_is_larger_than_a_room_can_ever_be` enforces.
        let name = format!("guest_{}", self.users.len() + 1);
        debug!(name = %name, "animal pool exhausted, assigned guest name");
        name
    }

    /// The name a joining visitor should get, honouring `preferred` only if it
    /// is genuinely theirs to take.
    ///
    /// `preferred` comes from an identity cookie, which is a claim by the
    /// client and nothing more: `HttpOnly` stops a page's script from touching
    /// the cookie, but not the person driving the browser from sending any
    /// `Cookie` header they like. It is honoured only when it is on the roster
    /// *and* free in this room — so a returning visitor keeps their name, while
    /// a forged one cannot invent a display name, cannot make it a megabyte
    /// long, and cannot impersonate somebody already in the room.
    pub fn claim_animal(&mut self, preferred: Option<&str>) -> String {
        if let Some(name) = preferred
            && is_animal_name(name)
            && !names_in_use(&self.users).contains(name)
        {
            return name.to_string();
        }
        self.assign_animal()
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
        self.attachment_bytes += msg.attachment.as_ref().map_or(0, Attachment::estimate_size);
        self.chat_history.push(Arc::new(msg));

        self.fade_oldest_attachments(memory_tracker);
    }

    /// Drops the payloads of the oldest images once the room is over
    /// [`MAX_ROOM_ATTACHMENT_BYTES`], keeping their messages.
    ///
    /// This is the §7 fade aimed at the most expensive thing in a room. A photo
    /// is two orders of magnitude larger than a sentence, so without it one
    /// room's pictures would take a share of the process-wide ceiling that
    /// every other room then could not have — the failure §1.1 describes, with
    /// a much bigger constant.
    ///
    /// The message stays and the attachment is marked `faded`, so the
    /// conversation keeps its shape: readers see that a picture was here and
    /// that it has gone, rather than finding a hole or a broken image.
    ///
    /// Normally does nothing and returns after one comparison. It only walks
    /// the history when the room is actually over budget, and then only far
    /// enough to get back under it.
    fn fade_oldest_attachments(&mut self, memory_tracker: &MemoryTracker) {
        if self.attachment_bytes <= MAX_ROOM_ATTACHMENT_BYTES {
            return;
        }

        let mut freed = 0usize;
        for msg in &mut self.chat_history {
            if self.attachment_bytes - freed <= MAX_ROOM_ATTACHMENT_BYTES {
                break;
            }
            if msg.attachment.as_ref().is_none_or(|a| a.faded) {
                continue;
            }

            // `make_mut` copies only if this message is still being serialised
            // for somebody's history at this instant, which is the one case
            // where sharing it would let a fade rewrite what they are reading.
            let msg = Arc::make_mut(msg);
            let attachment = msg
                .attachment
                .as_mut()
                .expect("just checked this message has an unfaded attachment");

            freed += attachment.estimate_size();
            attachment.data = String::new();
            attachment.faded = true;
        }

        if freed == 0 {
            return;
        }

        debug!(bytes = freed, "faded oldest attachments");
        self.attachment_bytes -= freed;
        self.total_memory_bytes.fetch_sub(
            freed.min(self.total_memory_bytes.load(Ordering::SeqCst)),
            Ordering::SeqCst,
        );
        memory_tracker.remove_bytes(freed);
    }

    /// Drops reaction state for messages that no longer exist.
    ///
    /// §3.5: `reactions` is keyed by message id and would otherwise keep an
    /// entry for every message the room has ever held. Called from every path
    /// that removes messages from `chat_history` — there is no other way for a
    /// message to leave.
    fn forget_reactions_for(&mut self, removed: &[Arc<OutgoingMessage>]) {
        for msg in removed {
            self.reactions.remove(&msg.message_id);
        }
    }

    /// Adds or removes this user's reaction, returning the emoji's new total.
    ///
    /// Reacting is a **toggle**: the same emoji twice from the same person is
    /// the person changing their mind, not two reactions. Returns `None` when
    /// nothing changed — an unknown message, or a new emoji on a message that
    /// already holds [`MAX_REACTIONS_PER_MESSAGE`] of them.
    ///
    /// O(1): the message is never searched for. `reactions` is keyed by id
    /// precisely so this stays off the history (§1.1).
    pub fn toggle_reaction(
        &mut self,
        message_id: Uuid,
        emoji: &str,
        user_id: &str,
    ) -> Option<(bool, usize)> {
        let buckets = self.reactions.entry(message_id).or_default();

        let active = match buckets.get_mut(emoji) {
            Some(reactors) => {
                if reactors.remove(user_id) {
                    false
                } else {
                    reactors.insert(user_id.to_string());
                    true
                }
            }
            None => {
                if buckets.len() >= MAX_REACTIONS_PER_MESSAGE {
                    debug!(%message_id, "message already holds its maximum distinct reactions");
                    // No empty entry to clean up: reaching this means the
                    // message already holds MAX_REACTIONS_PER_MESSAGE buckets,
                    // which is more than none. A guard for the empty case here
                    // was a branch nothing could ever take.
                    return None;
                }
                buckets
                    .entry(emoji.to_string())
                    .or_default()
                    .insert(user_id.to_string());
                true
            }
        };

        let count = buckets.get(emoji).map_or(0, HashSet::len);

        // An emoji nobody is in is not a reaction; drop the bucket so the map
        // is bounded by *live* reactions rather than by every emoji ever tried.
        if count == 0 {
            buckets.remove(emoji);
        }
        if buckets.is_empty() {
            self.reactions.remove(&message_id);
        }

        self.last_activity = Instant::now();
        Some((active, count))
    }

    /// This message's reaction buckets, as `viewer` should see them.
    ///
    /// `reacted` is resolved per viewer here rather than shipping the reactor
    /// list, so one user's identity is never sent to another.
    pub fn reactions_for(&self, message_id: Uuid, viewer: &str) -> Vec<Reaction> {
        let Some(buckets) = self.reactions.get(&message_id) else {
            return Vec::new();
        };

        let mut out: Vec<Reaction> = buckets
            .iter()
            .map(|(emoji, reactors)| Reaction {
                emoji: emoji.clone(),
                count: reactors.len(),
                reacted: reactors.contains(viewer),
            })
            .collect();

        // A stable order, so the same message does not shuffle its reactions
        // between one reader's screen and another's.
        out.sort_by(|a, b| a.emoji.cmp(&b.emoji));
        out
    }

    /// History as `viewer` should receive it: each message paired with the
    /// reactions resolved for them.
    ///
    /// O(history) in refcount bumps, not in bytes — the messages themselves are
    /// never copied. That matters because this runs under the room write lock,
    /// so its cost is charged to every user in every room, not just the one
    /// joining (§2, §6). Cloning the owned history here measured 119 µs on a
    /// full room; this measures 1.3 µs.
    pub fn history_for(&self, viewer: &str) -> Vec<(Arc<OutgoingMessage>, Vec<Reaction>)> {
        // Most rooms have never seen a reaction, and for those the per-message
        // lookup is the bulk of what is left of this function.
        if self.reactions.is_empty() {
            return self
                .chat_history
                .iter()
                .map(|msg| (Arc::clone(msg), Vec::new()))
                .collect();
        }

        self.chat_history
            .iter()
            .map(|msg| {
                let reactions = self.reactions_for(msg.message_id, viewer);
                (Arc::clone(msg), reactions)
            })
            .collect()
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

        let removed: Vec<Arc<OutgoingMessage>> = self.chat_history.drain(..drop_count).collect();
        self.forget_reactions_for(&removed);
        self.release_attachment_bytes(&removed);

        let current = self.total_memory_bytes.load(Ordering::SeqCst);
        self.total_memory_bytes
            .store(current.saturating_sub(removed_size), Ordering::SeqCst);
        memory_tracker.remove_bytes(removed_size);
    }

    /// Subtracts the attachment payloads of messages that have just left.
    fn release_attachment_bytes(&mut self, removed: &[Arc<OutgoingMessage>]) {
        let freed: usize = removed
            .iter()
            .filter_map(|m| m.attachment.as_ref())
            .map(Attachment::estimate_size)
            .sum();
        self.attachment_bytes = self.attachment_bytes.saturating_sub(freed);
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
        let mut dropped_ids: Vec<Uuid> = Vec::new();

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
            dropped_ids.push(msg.message_id);
            false
        });

        if removed > 0 {
            for id in &dropped_ids {
                self.reactions.remove(id);
            }
            // Recomputes `attachment_bytes` too, so aged-out images stop
            // counting against the room's picture budget.
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
            .map(|m| m.estimate_size())
            .sum();

        let removed: Vec<Arc<OutgoingMessage>> = self.chat_history.drain(..drop_count).collect();
        self.forget_reactions_for(&removed);
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
    fn recompute_memory(&mut self) {
        let total: usize = self.chat_history.iter().map(|m| m.estimate_size()).sum();
        self.total_memory_bytes.store(total, Ordering::SeqCst);

        // Recomputed from the same walk for the same reason: a drift in the
        // attachment total is corrected here rather than accumulating until the
        // room fades pictures that were within budget all along.
        self.attachment_bytes = self
            .chat_history
            .iter()
            .filter_map(|m| m.attachment.as_ref())
            .map(Attachment::estimate_size)
            .sum();
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
