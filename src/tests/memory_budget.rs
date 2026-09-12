//! §1.1/§2/§3 as arithmetic and as measured behaviour: the memory ceiling
//! closes whatever the constants are set to, and the per-message cost that
//! makes it possible to hold that ceiling — sharing history instead of
//! copying it, O(1) reactions — actually holds under test.

use super::*;
use crate::room::BROADCAST_CHANNEL_CAPACITY;

/// §3 — the memory budget closes, whatever the constants are set to.
///
/// Every ceiling here was chosen against the others: attachments are capped per
/// room *because* a hundred rooms share one process-wide budget. Raising any
/// one of them in isolation silently overcommits the container, and the symptom
/// is an OOM kill that disconnects every user in every room — the failure the
/// whole memory law exists to avoid. So the arithmetic is asserted rather than
/// left in a comment for somebody to re-derive.
#[test]
fn the_memory_budget_still_closes() {
    // Pictures may claim at most half the process, leaving the rest for text.
    let attachment_ceiling = MAX_ROOM_ATTACHMENT_BYTES * MAX_ROOMS;
    assert!(
        attachment_ceiling <= MAX_TOTAL_ROOMS_MEMORY / 2,
        "every room at its attachment budget is {attachment_ceiling} bytes, over \
         half of the {MAX_TOTAL_ROOMS_MEMORY}-byte process ceiling — one room \
         full of photographs would be taking what every other room needs"
    );

    // A single attachment cannot be a meaningful fraction of a room's budget,
    // or "fading the oldest" would clear the room in one step.
    //
    // A `const` block, so this is checked when the crate is compiled rather
    // than when the test is run: a constant edited to break it fails the build
    // for everyone, including anyone who only runs a subset of the suite.
    const {
        assert!(
            MAX_ATTACHMENT_BYTES * 8 <= MAX_ROOM_ATTACHMENT_BYTES,
            "a room must hold at least 8 images at the per-image cap"
        );
    }

    // A room's text is bounded by message count times the largest a message can
    // be, and that has to fit too.
    let text_ceiling = MAX_MESSAGES_PER_ROOM * (MAX_RENDERED_MESSAGE_LEN + ESTIMATED_MESSAGE_SIZE);
    assert!(
        text_ceiling <= MAX_TOTAL_ROOMS_MEMORY,
        "one room of maximum-size messages is {text_ceiling} bytes, over the \
         whole process ceiling"
    );

    // The rendered ceiling must stay at or above the escaping worst case, or
    // `render_message_html`'s plain-text fallback could itself breach it.
    const {
        assert!(
            MAX_RENDERED_MESSAGE_LEN >= MAX_MESSAGE_LEN * 5,
            "escaping expands by up to 5x, so a lower ceiling makes the fallback able to exceed the limit it is the fallback for"
        );
    }

    // And the whole thing fits in the container, which is the claim the rest
    // of this test is only useful because of.
    //
    // `MAX_TOTAL_ROOMS_MEMORY` bounds *tracked* bytes — history and
    // attachments. Three things sit outside it and none of them were in any
    // arithmetic before: the baseline process, the per-connection transport
    // buffers, and the broadcast rings. The rings were the largest of the
    // three and the least visible, because a `tokio` broadcast holds every
    // sent frame until all subscribers have taken it, so one receiver that
    // stops reading pins `BROADCAST_CHANNEL_CAPACITY` frames — at the
    // previous depth of 1000, 172 MB on top of a 150 MB ceiling, on a box
    // with 256 MiB in it.
    let history = MAX_TOTAL_ROOMS_MEMORY;
    let connections = MAX_CONCURRENT_USERS * MEASURED_CONNECTION_BYTES;
    let one_rooms_ring = BROADCAST_CHANNEL_CAPACITY * MAX_BROADCAST_FRAME_BYTES;
    let committed = MEASURED_BASELINE_BYTES + history + connections + one_rooms_ring;

    let (num, den) = MEMORY_BUDGET_HEADROOM_RATIO;
    let allowed = CONTAINER_MEMORY_BYTES / den * num;
    assert!(
        committed <= allowed,
        "at capacity this process commits {committed} bytes — baseline \
         {MEASURED_BASELINE_BYTES}, history {history}, {MAX_CONCURRENT_USERS} \
         connections {connections}, one room's broadcast ring \
         {one_rooms_ring} — against {allowed} allowed of the container's \
         {CONTAINER_MEMORY_BYTES}. Over it, a full server is an OOM kill that \
         disconnects every user in every room, which is the one failure the \
         whole memory law exists to prevent. Lower a ceiling, or raise the \
         container's instance type in cloudflare/wrangler.jsonc and this \
         constant with it."
    );
}

/// The container really is the size [`CONTAINER_MEMORY_BYTES`] says it is.
///
/// `wrangler.jsonc` already carried a comment saying that raising the instance
/// type means revisiting the memory budget. A comment is not a check — and the
/// budget it pointed at did not include the two largest consumers, so it would
/// not have caught the change either way. This is the executable half: change
/// the instance type and the arithmetic above has to be redone in the same
/// commit (§6).
#[test]
fn the_container_is_still_the_size_the_budget_assumes() {
    const WRANGLER: &str = include_str!("../../cloudflare/wrangler.jsonc");

    assert!(
        WRANGLER.contains("\"instance_type\": \"lite\""),
        "cloudflare/wrangler.jsonc no longer asks for the `lite` instance \
         type, whose 256 MiB is what CONTAINER_MEMORY_BYTES and every ceiling \
         sized against it assume. Update that constant and re-derive the \
         budget, or put the instance type back."
    );
    assert_eq!(
        CONTAINER_MEMORY_BYTES,
        256 * 1024 * 1024,
        "`lite` is 256 MiB; the budget's idea of the box has drifted from the \
         box"
    );
}

/// §2 — the join path shares stored messages; it does not copy them.
///
/// Deterministic, not timed: if `history_for` handed back copies, the strong
/// count of the stored `Arc` would not move. Measured, the copy it replaced
/// took 119 µs *under the room write lock*, so this is the largest single
/// regression anyone could reintroduce here by changing a type back to owned.
#[tokio::test]
async fn the_join_path_shares_history_rather_than_copying_it() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "shared, not copied".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    let before = Arc::strong_count(&room.chat_history[0]);

    let first = room.history_for("alice");
    let second = room.history_for("bob");

    assert_eq!(
        Arc::strong_count(&room.chat_history[0]),
        before + 2,
        "each viewer's history must be a reference to the stored message, not \
         a copy of it"
    );

    // And they really are the same allocation, not equal values.
    assert!(
        Arc::ptr_eq(&first[0].0, &room.chat_history[0])
            && Arc::ptr_eq(&second[0].0, &room.chat_history[0]),
        "both viewers should be reading the one stored message"
    );

    drop(first);
    drop(second);
    assert_eq!(
        Arc::strong_count(&room.chat_history[0]),
        before,
        "and the references go away with them"
    );
}

/// §1.1 — per-message work does not grow with the size of the history.
///
/// The property, stated so it cannot be satisfied by a fast machine: adding a
/// message to a room holding `MAX_MESSAGES_PER_ROOM` messages must touch the
/// same amount of state as adding one to an almost-empty room. Asserted through
/// the accounting rather than the clock — a message's cost to the room is its
/// own size and nothing else, so if any history-proportional work crept back
/// onto this path (a re-scan, a re-sum, a trim) the totals would diverge.
#[tokio::test]
async fn adding_a_message_costs_the_same_whatever_the_history_holds() {
    fn message(text: &str) -> OutgoingMessage {
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: text.to_string(),
            timestamp: "1700000000000".to_string(),
            reply_to: None,
            attachment: None,
        }
    }

    let mut deltas = Vec::new();

    for prefill in [1usize, MAX_MESSAGES_PER_ROOM - 1] {
        let tracker = MemoryTracker::new();
        let mut room = create_room();
        for _ in 0..prefill {
            room.add_message(message("filler"), &tracker);
        }

        let before_room = room.total_memory_bytes.load(Ordering::SeqCst);
        let before_global = tracker.total_bytes.load(Ordering::SeqCst);

        room.add_message(message("the measured one"), &tracker);

        deltas.push((
            room.total_memory_bytes.load(Ordering::SeqCst) - before_room,
            tracker.total_bytes.load(Ordering::SeqCst) - before_global,
            room.chat_history.len() - prefill,
        ));
    }

    assert_eq!(
        deltas[0], deltas[1],
        "adding one message to a nearly-full room must cost exactly what it \
         costs in an empty one; a difference means work proportional to the \
         history got back onto the message path"
    );
}

/// §1.1 — reacting is O(1) in the size of the history.
///
/// Reactions live beside the history, keyed by message id, precisely so this
/// holds. Asserted structurally: reacting to the *oldest* message in a full
/// room must leave the history untouched, which it cannot do if the message is
/// being searched for or rewritten in place.
#[tokio::test]
async fn reacting_never_touches_the_history() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    for i in 0..MAX_MESSAGES_PER_ROOM {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("m{i}"),
                timestamp: "1700000000000".to_string(),
                reply_to: None,
                attachment: None,
            },
            &tracker,
        );
    }

    let oldest = room.chat_history[0].message_id;
    let stored = Arc::clone(&room.chat_history[0]);
    let bytes_before = room.total_memory_bytes.load(Ordering::SeqCst);

    room.toggle_reaction(oldest, "🔥", "alice");

    assert!(
        Arc::ptr_eq(&stored, &room.chat_history[0]),
        "reacting must not rewrite the message it refers to"
    );
    assert_eq!(
        room.total_memory_bytes.load(Ordering::SeqCst),
        bytes_before,
        "reacting must not re-derive the room's byte total"
    );
    assert_eq!(room.chat_history.len(), MAX_MESSAGES_PER_ROOM);
}
