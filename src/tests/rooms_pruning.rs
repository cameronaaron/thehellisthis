//! Trimming a room's history in place: pruning by age, capping the batch a
//! single pass removes, and fading the oldest attachments once a room is
//! over its picture budget.

use super::*;

#[tokio::test]
async fn test_cleanup_messages_by_age() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Old message (40 days)
    let old_timestamp = format!("{}", now_ms - (40 * 24 * 60 * 60 * 1000));
    let old_msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Old</p>".to_string(),
        timestamp: old_timestamp,
        reply_to: None,
        attachment: None,
    };

    // Fresh message
    let fresh_msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Fresh</p>".to_string(),
        timestamp: format!("{}", now_ms),
        reply_to: None,
        attachment: None,
    };

    let old_size = old_msg.estimate_size();
    room.chat_history.push(Arc::new(old_msg));
    room.chat_history.push(Arc::new(fresh_msg));
    room.total_memory_bytes
        .store(old_size + 100, Ordering::SeqCst);
    tracker.add_bytes(old_size + 100);

    // Cleanup old messages (anything > 30 days)
    room.cleanup_messages(Instant::now(), &tracker).await;

    // Old message should be removed (only fresh message remains)
    assert_eq!(room.chat_history.len(), 1);
}

#[tokio::test]
async fn test_cleanup_messages_keeps_invalid_timestamp() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "user1".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Bad time</p>".to_string(),
        timestamp: "not-a-number".to_string(),
        reply_to: None,
        attachment: None,
    };

    let size = msg.estimate_size();
    room.chat_history.push(Arc::new(msg));
    room.total_memory_bytes.store(size, Ordering::SeqCst);
    tracker.add_bytes(size);

    room.cleanup_messages(Instant::now(), &tracker).await;
    assert_eq!(room.chat_history.len(), 1);
}

// ========== CONNECTION POOL & SECURITY ==========

#[tokio::test]
async fn test_prune_old_messages_empties_when_all_old() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add messages with very old timestamps (30+ days)
    let old_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - (31 * 24 * 60 * 60); // 31 days ago

    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Old message {}", i),
            timestamp: old_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.add_message(msg, &memory_tracker);
    }

    // Cleanup should remove all old messages
    room_state
        .cleanup_messages(Instant::now(), &memory_tracker)
        .await;

    // Messages should be removed since they're > 30 days old
    assert!(room_state.chat_history.is_empty());
}

#[tokio::test]
async fn test_prune_old_messages_partial() {
    let mut room_state = create_room();
    let memory_tracker = MemoryTracker::new();

    // Add a mix of old and new messages
    let old_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - (31 * 24 * 60 * 60); // 31 days ago

    let new_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Add 5 old messages
    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("Old message {}", i),
            timestamp: old_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
        memory_tracker.add_bytes(50);
    }

    // Add 5 new messages
    for i in 0..5 {
        let msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Tiger".to_string(),
            text: format!("New message {}", i),
            timestamp: new_time.to_string(),
            reply_to: None,
            attachment: None,
        };
        room_state.chat_history.push(Arc::new(msg));
        memory_tracker.add_bytes(50);
    }

    // Cleanup should remove only old messages
    room_state
        .cleanup_messages(Instant::now(), &memory_tracker)
        .await;

    // Should have only the 5 new messages
    assert_eq!(room_state.chat_history.len(), 5);
}

#[tokio::test]
async fn test_prune_old_messages_removes_oldest_first() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Add 3 messages
    for i in 0..3 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user-{}", i),
            animal_name: "Lion".to_string(),
            text: format!("<p>Message {}</p>", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        };
        let size = msg.estimate_size();
        room.chat_history.push(Arc::new(msg));
        room.total_memory_bytes.fetch_add(size, Ordering::SeqCst);
        tracker.add_bytes(size);
    }

    assert_eq!(room.chat_history.len(), 3);

    // Prune needing space for 1 message
    let first_msg_size = room.chat_history[0].estimate_size();
    room.prune_old_messages(first_msg_size, &tracker);

    // Should have removed the first (oldest) message
    assert_eq!(room.chat_history.len(), 2);
    assert!(
        room.chat_history[0].text.contains("Message 1"),
        "First remaining should be Message 1"
    );
}

#[tokio::test]
async fn test_room_state_trim_to_max_messages() {
    let mut room = create_room();
    let tracker = MemoryTracker::new();

    // Add more than MAX_MESSAGES_PER_ROOM
    for i in 0..MAX_MESSAGES_PER_ROOM + 50 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: "user1".to_string(),
            animal_name: "Lion".to_string(),
            text: format!("<p>Message {}</p>", i),
            timestamp: "1000".to_string(),
            reply_to: None,
            attachment: None,
        };
        room.chat_history.push(Arc::new(msg));
    }

    room.trim_to_max_messages(&tracker);
    assert!(room.chat_history.len() <= MAX_MESSAGES_PER_ROOM);
}

#[tokio::test]
async fn test_prune_old_messages_with_existing_messages() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Add several messages
    for i in 0..10 {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: format!("Message {}", i),
            timestamp: format!("{}", i * 1000),
            reply_to: None,
            attachment: None,
        }));
        tracker.add_bytes(100);
        room.total_memory_bytes
            .fetch_add(100, std::sync::atomic::Ordering::SeqCst);
    }

    let initial_len = room.chat_history.len();

    // Prune messages
    room.prune_old_messages(500, &tracker);

    // Should have removed some messages
    assert!(room.chat_history.len() < initial_len);
}

// Test pong message handling - covers lines 1848-1849

#[tokio::test]
async fn test_prune_old_messages_keeps_recent() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Add messages with timestamps - each message ~100 bytes
    for i in 0..20 {
        let msg = OutgoingMessage {
            message_id: uuid::Uuid::new_v4(),
            user_id: format!("user{}", i),
            animal_name: format!("Animal{}", i),
            text: format!("Message {}", i),
            timestamp: format!("{}", i * 1000 + 1000000),
            reply_to: None,
            attachment: None,
        };
        let msg_size = msg.estimate_size();
        tracker.add_bytes(msg_size);
        room.total_memory_bytes
            .fetch_add(msg_size, Ordering::SeqCst);
        room.chat_history.push(Arc::new(msg));
    }

    let initial_count = room.chat_history.len();
    assert_eq!(initial_count, 20);

    // Prune to free up space for 500 bytes (should remove a few messages)
    room.prune_old_messages(500, &tracker);

    // Should have fewer messages after pruning
    assert!(room.chat_history.len() < initial_count);
}

// Test RoomState add_message when approaching memory limit - covers lines 848-870

/// Pruning with nothing to reclaim is a no-op, not an underflow.
#[test]
fn pruning_zero_bytes_changes_nothing() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "hello".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    let before = room.chat_history.len();
    room.prune_old_messages(0, &tracker);
    assert_eq!(room.chat_history.len(), before, "nothing needed reclaiming");

    // Likewise, trimming to more than is present.
    room.retain_newest(usize::MAX, &tracker);
    assert_eq!(room.chat_history.len(), before);
}

/// Age-based cleanup removes at most one batch per pass.
///
/// The cap bounds how long a sweep can hold the room write lock, which every
/// connected user contends on.
#[tokio::test]
async fn message_cleanup_is_capped_at_one_batch_per_pass() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Every message far older than MAX_MESSAGE_AGE.
    for _ in 0..(CLEANUP_BATCH_SIZE + 50) {
        room.chat_history.push(Arc::new(OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "old".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        }));
    }
    let before = room.chat_history.len();

    room.cleanup_messages(Instant::now(), &tracker).await;

    assert_eq!(
        room.chat_history.len(),
        before - CLEANUP_BATCH_SIZE,
        "a single pass removes exactly one batch"
    );
}

/// Fading subtracts exactly what it freed.
///
/// Two mutants survived here: `attachment_bytes - freed` becoming `+`, and
/// `attachment_bytes -= freed` becoming `/=`. Either leaves the running total
/// disconnected from what the room is holding, and the symptom is silent —
/// pictures fading while the room is nearly empty, or never fading at all.
#[tokio::test]
async fn fading_subtracts_exactly_the_bytes_it_freed() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    let payload = MAX_ATTACHMENT_BYTES;
    let each = payload + "image/png".len();
    let needed = (MAX_ROOM_ATTACHMENT_BYTES / each) + 2;

    for i in 0..needed {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("p{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: Some(Attachment {
                    mime: "image/png".to_string(),
                    data: "A".repeat(payload),
                    width: 1,
                    height: 1,
                    faded: false,
                }),
            },
            &tracker,
        );
    }

    // The running total must equal what is actually still held, counted from
    // the history rather than from the counter it is being compared against.
    let actually_held: usize = room
        .chat_history
        .iter()
        .filter_map(|m| m.attachment.as_ref())
        .filter(|a| !a.faded)
        .map(crate::protocol::Attachment::estimate_size)
        .sum();

    assert_eq!(
        room.attachment_bytes, actually_held,
        "after fading, the running total must be exactly the bytes still held"
    );
    assert!(room.attachment_bytes <= MAX_ROOM_ATTACHMENT_BYTES);

    // Faded entries keep their mime but lose their payload, so the total is a
    // whole number of surviving images.
    assert_eq!(
        actually_held % each,
        0,
        "the survivors should each be a full image"
    );
}

/// Trimming boundaries are exact: at the cap nothing moves, one past it one goes.
///
/// Both `>` comparisons mutated to `>=` and survived — the existing tests
/// trimmed comfortably-over histories and never checked the edge. Trimming at
/// exactly the cap would move the whole history on the message that reaches it,
/// which is the O(n)-per-message cost `HISTORY_TRIM_SLACK` exists to prevent.
#[tokio::test]
async fn the_history_trim_boundaries_are_exact() {
    let tracker = MemoryTracker::new();

    // `retain_newest` at exactly `keep` is a no-op.
    let mut room = create_room();
    for i in 0..10 {
        message_in(&mut room, &tracker, &format!("m{i}"));
    }
    let before: Vec<Uuid> = room.chat_history.iter().map(|m| m.message_id).collect();
    room.retain_newest(10, &tracker);
    assert_eq!(
        room.chat_history
            .iter()
            .map(|m| m.message_id)
            .collect::<Vec<_>>(),
        before,
        "keeping exactly what is there must not touch the history"
    );

    // One more than `keep` drops exactly one, the oldest.
    room.retain_newest(9, &tracker);
    assert_eq!(room.chat_history.len(), 9);
    assert_eq!(
        room.chat_history[0].message_id, before[1],
        "the oldest is the one that goes"
    );

    // `trim_to_max_messages` at exactly the cap is a no-op.
    let mut room = create_room();
    for i in 0..MAX_MESSAGES_PER_ROOM {
        message_in(&mut room, &tracker, &format!("m{i}"));
    }
    let oldest = room.chat_history[0].message_id;
    room.trim_to_max_messages(&tracker);
    assert_eq!(
        room.chat_history.len(),
        MAX_MESSAGES_PER_ROOM,
        "a history exactly at the cap must not be trimmed"
    );
    assert_eq!(room.chat_history[0].message_id, oldest);

    // One past the cap trims back to it.
    message_in(&mut room, &tracker, "one too many");
    room.trim_to_max_messages(&tracker);
    assert_eq!(room.chat_history.len(), MAX_MESSAGES_PER_ROOM);
    assert_ne!(
        room.chat_history[0].message_id, oldest,
        "and the oldest is what it dropped"
    );
}

/// A room over its picture budget fades the oldest images and stops.
///
/// `self.attachment_bytes - freed <= MAX_ROOM_ATTACHMENT_BYTES` is the loop's
/// only exit, and replacing that `-` with `+` survived: the running total then
/// grows as images are freed, the condition never becomes true, and every
/// picture in the room is faded rather than just enough of them. A room one
/// image over its budget would lose all of them.
#[tokio::test]
async fn fading_pictures_stops_as_soon_as_the_room_is_back_under_its_budget() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Each image is a tenth of the budget, so going one over needs eleven and
    // recovering needs exactly one to fade.
    let payload = "A".repeat(MAX_ROOM_ATTACHMENT_BYTES / 10);
    for i in 0..11 {
        let mut msg = OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: format!("picture {i}"),
            timestamp: "0".to_string(),
            reply_to: None,
            attachment: Some(Attachment {
                data: payload.clone(),
                ..png_attachment()
            }),
        };
        msg.timestamp = i.to_string();
        room.add_message(msg, &tracker);
    }

    let faded: Vec<bool> = room
        .chat_history
        .iter()
        .map(|m| m.attachment.as_ref().is_some_and(|a| a.faded))
        .collect();
    let faded_count = faded.iter().filter(|f| **f).count();

    assert!(
        faded_count > 0,
        "a room over its picture budget must fade something"
    );
    assert!(
        faded_count < faded.len(),
        "fading must stop once the room is back under budget — it faded all \
         {} pictures, which is the whole conversation's images for one overrun",
        faded.len()
    );
    assert!(
        faded[..faded_count].iter().all(|f| *f) && !faded[faded_count],
        "the pictures that fade are the oldest ones, in order: {faded:?}"
    );
}
