//! History storage, attachment fading, and the three pruning paths that keep
//! a room inside its memory budget.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime};

use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::config::{
    CLEANUP_BATCH_SIZE, MAX_MESSAGE_AGE, MAX_MESSAGES_PER_ROOM, MAX_ROOM_ATTACHMENT_BYTES,
    MAX_TOTAL_ROOMS_MEMORY,
};
use crate::limits::MemoryTracker;
use crate::protocol::{Attachment, OutgoingMessage, Reaction};

use super::RoomState;

impl RoomState {
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

        let total = self
            .total_memory_bytes
            .fetch_add(msg_size, Ordering::SeqCst)
            + msg_size;
        self.attachment_bytes += msg.attachment.as_ref().map_or(0, Attachment::estimate_size);
        self.message_ids.insert(msg.message_id);
        let message_id = msg.message_id;
        let history_len = self.chat_history.len() + 1;
        trace!(
            %message_id,
            bytes = msg_size,
            room_bytes = total,
            history_len,
            "message stored"
        );
        self.chat_history.push(Arc::new(msg));

        self.fade_oldest_attachments(memory_tracker);
    }

    /// Drops the payloads of the oldest images once the room is over
    /// [`MAX_ROOM_ATTACHMENT_BYTES`], keeping their messages.
    ///
    /// §7's fade aimed at the most expensive thing in a room: a photo is two
    /// orders of magnitude larger than a sentence, so one room's pictures could
    /// otherwise take ceiling every other room needs (§1.1). The message stays,
    /// marked `faded`, so readers see a picture was there and has gone.
    ///
    /// Usually one comparison and a return; it walks history only when over
    /// budget, and only far enough to get back under.
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

            // `make_mut` copies only if this message is being serialised for
            // someone's history right now, the one case where sharing would let a
            // fade rewrite what they're reading.
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

    /// Drops reaction state for messages that no longer exist (§3.5): every path
    /// that removes messages from `chat_history` calls this. It takes ids so each
    /// path can; `cleanup_messages` used to reimplement the loop by hand.
    fn forget_reactions_for(&mut self, removed_ids: impl IntoIterator<Item = Uuid>) {
        for id in removed_ids {
            self.reactions.remove(&id);
            self.message_ids.remove(&id);
        }
    }

    /// History as `viewer` should receive it, each message with its reactions
    /// resolved for them. Refcount bumps, not copies: this runs under the room
    /// write lock, so its cost falls on every user (§2, §6). The owned clone
    /// measured 119 µs on a full room; this measures 1.3 µs.
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

    /// Drops oldest messages until `needed_space` bytes are free: count first,
    /// then one `drain`. `Vec::remove(0)` in a loop was O(n²) on the message path
    /// under the room write lock.
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
        self.forget_reactions_for(removed.iter().map(|m| m.message_id));
        self.release_attachment_bytes(&removed);

        let current = self.total_memory_bytes.load(Ordering::SeqCst);
        self.total_memory_bytes
            .store(current.saturating_sub(removed_size), Ordering::SeqCst);
        memory_tracker.remove_bytes(removed_size);

        debug!(
            dropped = drop_count,
            bytes_freed = removed_size,
            needed_space,
            "pruned oldest messages to free space"
        );
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
    /// [`CLEANUP_BATCH_SIZE`] per pass, in place; it used to clone every surviving
    /// message into a second vector to delete a few.
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
            self.forget_reactions_for(dropped_ids);
            // Recomputes `attachment_bytes` too, so aged-out images stop
            // counting against the room's picture budget.
            self.recompute_memory();
            memory_tracker.remove_bytes(removed_bytes);
            debug!(
                removed,
                bytes_freed = removed_bytes,
                "dropped aged messages"
            );
        }
    }

    /// Trims history to exactly the cap. Used on join, where the cost is paid
    /// once by the joining user rather than by every message.
    pub fn trim_to_max_messages(&mut self, memory_tracker: &MemoryTracker) {
        // No length check: `retain_newest` makes this comparison itself. A
        // guard that can't differ from the one behind it survives every
        // mutation, the sign it isn't a guard (§6.6d).
        self.retain_newest(MAX_MESSAGES_PER_ROOM, memory_tracker);
    }

    /// Keeps the newest `keep` messages, releasing the rest from the room and the
    /// global tracker. Returns how many were dropped, so callers can report a trim
    /// without repeating this comparison.
    pub fn retain_newest(&mut self, keep: usize, memory_tracker: &MemoryTracker) -> usize {
        if self.chat_history.len() <= keep {
            return 0;
        }

        let drop_count = self.chat_history.len() - keep;
        let removed_bytes: usize = self.chat_history[..drop_count]
            .iter()
            .map(|m| m.estimate_size())
            .sum();

        let removed: Vec<Arc<OutgoingMessage>> = self.chat_history.drain(..drop_count).collect();
        self.forget_reactions_for(removed.iter().map(|m| m.message_id));
        self.recompute_memory();

        // Unconditional: `remove_bytes` saturates, so zero bytes is a no-op,
        // and a guard here survived mutation either way (§6.6d).
        memory_tracker.remove_bytes(removed_bytes);

        trace!(
            dropped = drop_count,
            bytes_freed = removed_bytes,
            keep,
            "trimmed to newest messages"
        );
        drop_count
    }

    /// Recomputes this room's byte total from its history, only inside a trim
    /// that is already O(history). Recomputing self-heals: an accounting slip
    /// elsewhere is corrected at the next trim instead of accumulating until the
    /// room wrongly reports itself full.
    fn recompute_memory(&mut self) {
        let total: usize = self.chat_history.iter().map(|m| m.estimate_size()).sum();
        self.total_memory_bytes.store(total, Ordering::SeqCst);

        // From the same walk, for the same reason: attachment drift is
        // corrected here before it fades pictures that were within budget.
        self.attachment_bytes = self
            .chat_history
            .iter()
            .filter_map(|m| m.attachment.as_ref())
            .map(Attachment::estimate_size)
            .sum();

        // Rebuilt from the same walk, so a drift in the id index is corrected
        // here rather than leaving reactions attached to nothing.
        self.message_ids = self.chat_history.iter().map(|m| m.message_id).collect();
    }
}
