//! Per-message reaction buckets: toggling one, and resolving them per viewer.

use std::collections::HashSet;
use std::time::Instant;

use tracing::{debug, trace};
use uuid::Uuid;

use crate::config::MAX_REACTIONS_PER_MESSAGE;
use crate::protocol::Reaction;

use super::RoomState;

impl RoomState {
    /// Adds or removes this user's reaction and returns the emoji's new total. A
    /// toggle: the same emoji twice is a change of mind. `None` when nothing
    /// changed (an unknown message, or a new emoji past
    /// [`MAX_REACTIONS_PER_MESSAGE`]). O(1): keyed by id, so the history is never
    /// searched (§1.1).
    pub fn toggle_reaction(
        &mut self,
        message_id: Uuid,
        emoji: &str,
        user_id: &str,
    ) -> Option<(bool, usize)> {
        // No message, no reaction, checked first in O(1); without it any uuid
        // was accepted and `reactions` grew with buckets nothing evicts (§3.5).
        if !self.message_ids.contains(&message_id) {
            debug!(%message_id, "reaction for a message this room does not hold");
            return None;
        }

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
                    // No empty entry to clean up: reaching here means the message
                    // already holds MAX_REACTIONS_PER_MESSAGE buckets.
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
        trace!(%message_id, emoji, %user_id, active, count, "reaction toggled");
        Some((active, count))
    }

    /// This message's reaction buckets as `viewer` should see them, with
    /// `reacted` resolved per viewer so nobody's identity reaches anyone else.
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
}
