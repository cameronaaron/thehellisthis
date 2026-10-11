//! The reaction emoji roster: the closed set a reaction may be.
//!
//! A reaction is a client string the server stores and broadcasts, so it is
//! checked against this list rather than sanitised: no escaping bug, length or
//! encoding to get wrong (ENGINEERING-STANDARDS.md §5.9, as with
//! `ANIMAL_NAMES`). Emoji inside messages are ordinary text through the
//! Markdown pipeline.
//!
//! Sorted by code point (what `binary_search` needs, not a reading order) and
//! deduplicated, since identical-looking entries would be separate buckets;
//! `reaction_roster_is_sorted_unique_and_actually_emoji` enforces both. The
//! client groups emoji for reading, and
//! `client_reaction_roster_matches_the_server` compares the two as sets: a
//! picker button the server rejects would silently do nothing.

/// Emoji that may be a reaction: a static slice in the binary, immutable and
/// shared by every room.
pub(crate) const REACTION_EMOJI: &[&str] = &[
    "‼️", "✅", "❌", "❤️", "⭐", "🎉", "🎯", "👀", "👇", "👋", "👍", "👎", "💀", "💜", "💡", "💯",
    "🔥", "😀", "😂", "😅", "😍", "😎", "😐", "😔", "😡", "😢", "😭", "😮", "😱", "😴", "🙄", "🙏",
    "🚀", "🤔", "🤝", "🤣", "🥳", "🫡",
];

/// Whether the server accepts `emoji` as a reaction. `binary_search` is
/// correct only because the roster is sorted, which
/// `reaction_roster_is_sorted_unique_and_actually_emoji` keeps true.
pub(crate) fn is_reaction_emoji(emoji: &str) -> bool {
    REACTION_EMOJI.binary_search(&emoji).is_ok()
}
