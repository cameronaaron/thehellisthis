//! The reaction emoji roster — the closed set of emoji a reaction may be.
//!
//! Reactions are stored per message and broadcast to the room, so the emoji is
//! a string arriving from a client. Rather than sanitise it, it is checked
//! against this list: a closed set has no escaping bug, no length to bound and
//! no encoding to get wrong (ENGINEERING-STANDARDS.md §5.9, the same reasoning
//! that makes `ANIMAL_NAMES` the only source of display names).
//!
//! Emoji *inside a message* are ordinary text and need none of this — they go
//! through the Markdown pipeline like any other UTF-8. This list exists only
//! because a reaction is a separate, server-stored piece of state.
//!
//! Kept sorted by code point and deduplicated, enforced by
//! `reaction_roster_is_sorted_unique_and_actually_emoji`. Sorted so a duplicate
//! is visible in review; deduplicated because two entries that render
//! identically would be two separate reaction buckets on the same message.
//!
//! Sorted here means sorted *by code point*, which is what `binary_search`
//! needs and is not a sensible order to show anybody. The client groups the
//! same emoji for reading; `client_reaction_roster_matches_the_server` compares
//! the two as sets, not as sequences. A reaction the picker offers but the
//! server rejects is a button that silently does nothing, which is the drift
//! that test exists to catch.

/// Emoji that may be used as a reaction.
///
/// A `&'static [&'static str]` living in the binary, like `ANIMAL_NAMES`: the
/// set is immutable and shared by every room.
pub(crate) const REACTION_EMOJI: &[&str] = &[
    "‼️", "✅", "❌", "❤️", "⭐", "🎉", "🎯", "👀", "👇", "👋", "👍", "👎", "💀", "💜", "💡", "💯",
    "🔥", "😀", "😂", "😅", "😍", "😎", "😐", "😔", "😡", "😢", "😭", "😮", "😱", "😴", "🙄", "🙏",
    "🚀", "🤔", "🤝", "🤣", "🥳", "🫡",
];

/// Whether `emoji` is one this server accepts as a reaction.
///
/// `binary_search` is O(log n) and correct only because the roster is sorted;
/// `reaction_roster_is_sorted_unique_and_actually_emoji` is what keeps that
/// true.
pub(crate) fn is_reaction_emoji(emoji: &str) -> bool {
    REACTION_EMOJI.binary_search(&emoji).is_ok()
}
