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
    ///
    /// Takes ids rather than the messages themselves so every removal path can
    /// call it — `cleanup_messages` used to reimplement this exact loop by
    /// hand over a `Vec<Uuid>` it collected itself, a second copy of the one
    /// thing this function exists to be the single place for.
    fn forget_reactions_for(&mut self, removed_ids: impl IntoIterator<Item = Uuid>) {
        for id in removed_ids {
            self.reactions.remove(&id);
            self.message_ids.remove(&id);
        }
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
        // No length check here: `retain_newest` makes exactly this comparison
        // and returns immediately when there is nothing to drop. A guard whose
        // condition can never differ from the one behind it is unkillable by
        // mutation testing, which is the signal that it is not a guard (§6.6d).
        self.retain_newest(MAX_MESSAGES_PER_ROOM, memory_tracker);
    }

    /// Keeps the newest `keep` messages, releasing the rest from both the room
    /// and the global tracker.
    ///
    /// Returns how many messages were dropped, so callers can report a trim
    /// without having to ask whether one was needed first — a second comparison
    /// against the same cap that this one already makes.
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

        // Unconditional: `remove_bytes` saturates, so returning zero bytes is
        // already a no-op. The guard could be flipped either way without any
        // test noticing, because there was nothing to notice (§6.6d).
        memory_tracker.remove_bytes(removed_bytes);

        trace!(
            dropped = drop_count,
            bytes_freed = removed_bytes,
            keep,
            "trimmed to newest messages"
        );
        drop_count
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

        // Rebuilt from the same walk, so a drift in the id index is corrected
        // here rather than leaving reactions attached to nothing.
        self.message_ids = self.chat_history.iter().map(|m| m.message_id).collect();
    }
}
