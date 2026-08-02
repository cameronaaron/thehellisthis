//! Reactions: a closed emoji set, stored beside the history.

use super::*;

#[tokio::test]
async fn test_message_with_unicode_emoji() {
    let text = "Hello 👋 World 🌍";
    let result = validate_message(text);
    assert!(result.is_ok());
}

/// Reacting is a toggle, and the same emoji twice is one person changing their
/// mind rather than two reactions.
#[test]
fn reacting_twice_with_the_same_emoji_removes_the_reaction() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    let id = message_in(&mut room, &tracker, "react to me");

    assert_eq!(room.toggle_reaction(id, "👍", "alice"), Some((true, 1)));
    assert_eq!(room.toggle_reaction(id, "👍", "bob"), Some((true, 2)));
    assert_eq!(
        room.toggle_reaction(id, "👍", "alice"),
        Some((false, 1)),
        "the same person reacting again takes their reaction back"
    );

    let seen = room.reactions_for(id, "bob");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].count, 1);
    assert!(seen[0].reacted, "bob is still in the bucket");
    assert!(!room.reactions_for(id, "alice")[0].reacted, "alice is not");
}

/// An emoji nobody is in is not a reaction, and a message nobody reacted to
/// holds no entry at all.
///
/// §3.5: `reactions` is keyed by message id, so anything it keeps once and
/// never releases grows for the life of the room.
#[test]
fn empty_reaction_buckets_are_not_retained() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    let id = message_in(&mut room, &tracker, "react to me");

    room.toggle_reaction(id, "🔥", "alice");
    room.toggle_reaction(id, "🔥", "alice");

    assert!(
        room.reactions.is_empty(),
        "the last person leaving a bucket should leave nothing behind, found {:?}",
        room.reactions
    );
    assert!(room.reactions_for(id, "alice").is_empty());
}

/// §3.5 — reaction state never outlives the message it belongs to.
///
/// Every path that removes a message must release its reactions; this walks
/// each of them rather than the one that happened to be written first.
#[tokio::test]
async fn reactions_never_outlive_the_messages_they_belong_to() {
    let tracker = MemoryTracker::new();

    // Path 1: trimming to the history cap.
    let mut room = create_room();
    let mut ids = Vec::new();
    for i in 0..(MAX_MESSAGES_PER_ROOM + 50) {
        let id = Uuid::new_v4();
        ids.push(id);
        room.add_message(
            OutgoingMessage {
                message_id: id,
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("m{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            },
            &tracker,
        );
        room.toggle_reaction(id, "👍", "alice");
    }
    room.retain_newest(10, &tracker);
    assert_eq!(
        room.reactions.len(),
        10,
        "trimming history must drop the reactions of the messages it removed"
    );

    // Path 2: age-based cleanup.
    let mut room = create_room();
    let id = Uuid::new_v4();
    room.add_message(
        OutgoingMessage {
            message_id: id,
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "ancient".to_string(),
            timestamp: "1".to_string(), // 1970 — far older than MAX_MESSAGE_AGE
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );
    room.toggle_reaction(id, "👍", "alice");
    room.cleanup_messages(Instant::now(), &tracker).await;
    assert!(
        room.chat_history.is_empty(),
        "the aged message should have gone"
    );
    assert!(
        room.reactions.is_empty(),
        "and its reactions with it, found {:?}",
        room.reactions
    );

    // Path 3: pruning under memory pressure.
    let mut room = create_room();
    let id = Uuid::new_v4();
    room.add_message(
        OutgoingMessage {
            message_id: id,
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "doomed".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );
    room.toggle_reaction(id, "👍", "alice");
    room.prune_old_messages(usize::MAX, &tracker);
    assert!(room.chat_history.is_empty());
    assert!(
        room.reactions.is_empty(),
        "pruning must release reactions too, found {:?}",
        room.reactions
    );
}

/// §5.9 — a reaction is a closed set, like an animal name.
#[tokio::test]
async fn a_reaction_must_be_on_the_roster() {
    let state = Arc::new(AppState::new());
    let message_id = Uuid::new_v4();
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        // Through `add_message`, not straight into the vector: the id index is
        // maintained there, and a message the room does not know it has is one
        // nobody can react to.
        room.add_message(
            OutgoingMessage {
                message_id,
                user_id: "u1".to_string(),
                animal_name: "otter".to_string(),
                text: "hi".to_string(),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            },
            &state.memory_tracker,
        );
        rooms.insert("react-room".to_string(), room);
    }

    for forged in [
        "<img src=x onerror=alert(1)>",
        "",
        "A".repeat(5000).as_str(),
    ] {
        apply_client_event(
            &state,
            "react-room",
            "u1",
            "otter",
            ClientEvent::React {
                message_id: message_id.to_string(),
                emoji: forged.to_string(),
            },
        )
        .await;
    }

    let rooms = state.rooms.read().await;
    assert!(
        rooms.get("react-room").unwrap().reactions.is_empty(),
        "nothing off the roster may become a reaction"
    );
}

/// A message holds a bounded number of distinct emoji.
#[test]
fn a_message_holds_a_bounded_number_of_distinct_reactions() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    let id = message_in(&mut room, &tracker, "react to me");

    let mut accepted = 0;
    for (i, emoji) in REACTION_EMOJI.iter().enumerate() {
        if room
            .toggle_reaction(id, emoji, &format!("user-{i}"))
            .is_some()
        {
            accepted += 1;
        }
    }

    assert_eq!(
        accepted, MAX_REACTIONS_PER_MESSAGE,
        "a message must stop accepting new emoji at its cap"
    );
    assert_eq!(
        room.reactions.get(&id).map(std::collections::HashMap::len),
        Some(MAX_REACTIONS_PER_MESSAGE)
    );
}

/// The reaction roster is sorted, unique, and made of emoji.
///
/// Sorted because `is_reaction_emoji` binary-searches it, and because a
/// duplicate is visible in review. This is the same sweep
/// `animal_roster_is_sorted_unique_and_well_formed` runs over the animal names,
/// for the same reason (§6.3).
#[test]
fn reaction_roster_is_sorted_unique_and_actually_emoji() {
    let mut sorted = REACTION_EMOJI.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        REACTION_EMOJI,
        &sorted[..],
        "the roster must be sorted; binary_search depends on it"
    );

    let unique: std::collections::HashSet<&&str> = REACTION_EMOJI.iter().collect();
    assert_eq!(
        unique.len(),
        REACTION_EMOJI.len(),
        "two identical entries would be two buckets that render the same"
    );

    for emoji in REACTION_EMOJI {
        assert!(!emoji.is_empty(), "an empty string is not an emoji");
        assert!(
            !emoji.is_ascii(),
            "{emoji:?} is ASCII, so it is punctuation rather than an emoji"
        );
        assert!(
            emoji.chars().count() <= 4,
            "{emoji:?} is longer than any single emoji should be"
        );
        assert!(
            is_reaction_emoji(emoji),
            "{emoji:?} must be findable in its own roster"
        );
    }

    assert!(!is_reaction_emoji("not-an-emoji"));
    assert!(!is_reaction_emoji(""));
}

/// The whole reaction path over `apply_client_event`, including its refusals.
#[tokio::test]
async fn the_reaction_event_broadcasts_throttles_and_refuses() {
    let state = Arc::new(AppState::new());
    let message_id;
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        // Something to react *to*: a reaction for a message the room does not
        // hold is refused, so a bare uuid would exercise nothing.
        message_id = message_in(&mut room, &state.memory_tracker, "react to me");
        rooms.insert("react-flow".to_string(), room);
    }

    let mut receiver = state
        .rooms
        .read()
        .await
        .get("react-flow")
        .unwrap()
        .sender
        .subscribe();

    // A message id that is not a uuid changes nothing.
    apply_client_event(
        &state,
        "react-flow",
        "u1",
        "otter",
        ClientEvent::React {
            message_id: "not-a-uuid".to_string(),
            emoji: "🔥".to_string(),
        },
    )
    .await;
    assert!(
        state
            .rooms
            .read()
            .await
            .get("react-flow")
            .unwrap()
            .reactions
            .is_empty()
    );

    // A real one is applied and broadcast.
    apply_client_event(
        &state,
        "react-flow",
        "u1",
        "otter",
        ClientEvent::React {
            message_id: message_id.to_string(),
            emoji: "🔥".to_string(),
        },
    )
    .await;

    let frame = receiver.try_recv().expect("a reaction should be broadcast");
    let event = decode_broadcast(&frame);
    match event {
        OutgoingEvent::System {
            event:
                SystemEvent::Reaction {
                    emoji,
                    active,
                    count,
                    ..
                },
        } => {
            assert_eq!(emoji, "🔥");
            assert!(active);
            assert_eq!(count, 1);
        }
        other => panic!("expected a Reaction event, got {other:?}"),
    }

    // Immediately again: the per-user throttle drops it, so nothing is sent.
    apply_client_event(
        &state,
        "react-flow",
        "u1",
        "otter",
        ClientEvent::React {
            message_id: message_id.to_string(),
            emoji: "👍".to_string(),
        },
    )
    .await;
    assert!(
        receiver.try_recv().is_err(),
        "a reaction inside the throttle window must not be broadcast"
    );
}

/// A message at its reaction cap refuses new emoji without leaving state behind.
#[tokio::test]
async fn a_refused_reaction_leaves_no_empty_bucket() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();
    let id = message_in(&mut room, &tracker, "react to me");

    // Fill the message to its cap.
    for (i, emoji) in REACTION_EMOJI
        .iter()
        .take(MAX_REACTIONS_PER_MESSAGE)
        .enumerate()
    {
        assert!(room.toggle_reaction(id, emoji, &format!("u{i}")).is_some());
    }

    let over_cap = REACTION_EMOJI[MAX_REACTIONS_PER_MESSAGE];
    assert_eq!(
        room.toggle_reaction(id, over_cap, "someone"),
        None,
        "a message at its cap must refuse a new emoji"
    );
    assert_eq!(
        room.reactions.get(&id).map(std::collections::HashMap::len),
        Some(MAX_REACTIONS_PER_MESSAGE),
        "and must not record the one it refused"
    );

    // The refusal must not have left the over-cap emoji behind as an empty
    // bucket — a bucket nobody is in is not a reaction, and this map is keyed
    // by message id (§3.5).
    assert!(
        !room
            .reactions
            .get(&id)
            .expect("the message still holds its reactions")
            .contains_key(over_cap),
        "a refused emoji must leave no trace"
    );
}

/// History carries each viewer's own reaction state, and nobody else's.
///
/// The stored message has no `reactions` field precisely because the answer to
/// "did you react" differs per recipient; it is resolved as history is sent.
/// This is also the path that pairs each message with its buckets, which the
/// no-reactions fast path skips.
#[tokio::test]
async fn history_resolves_reactions_for_the_viewer_receiving_it() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    for i in 0..3 {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "author".to_string(),
                animal_name: "otter".to_string(),
                text: format!("m{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: None,
            },
            &tracker,
        );
    }

    // With nothing reacted to, every message comes back with no buckets.
    let bare = room.history_for("alice");
    assert_eq!(bare.len(), 3);
    assert!(bare.iter().all(|(_, r)| r.is_empty()));

    let target = room.chat_history[1].message_id;
    room.toggle_reaction(target, "🔥", "alice");
    room.toggle_reaction(target, "🔥", "bob");
    room.toggle_reaction(target, "🎉", "bob");

    let for_alice = room.history_for("alice");
    let (_, alice_reactions) = &for_alice[1];
    assert_eq!(alice_reactions.len(), 2, "both buckets should be visible");

    let fire = alice_reactions.iter().find(|r| r.emoji == "🔥").unwrap();
    assert_eq!(fire.count, 2);
    assert!(fire.reacted, "alice is in the fire bucket");

    let party = alice_reactions.iter().find(|r| r.emoji == "🎉").unwrap();
    assert_eq!(party.count, 1);
    assert!(!party.reacted, "alice is not in the party bucket");

    // Same room, different viewer, different answer — from the same stored
    // messages, which were never copied to say so.
    let for_bob = room.history_for("bob");
    let (_, bob_reactions) = &for_bob[1];
    assert!(
        bob_reactions.iter().all(|r| r.reacted),
        "bob is in both buckets"
    );

    // Messages with no reactions still come back, in order, with empty vecs.
    assert!(for_alice[0].1.is_empty() && for_alice[2].1.is_empty());
}

/// §3.5 — a reaction for a message that does not exist is refused, not stored.
///
/// `reactions` is keyed by message id, and until this check existed *any* uuid
/// was accepted. A client sending `React` frames with random ids — which the
/// 100 ms throttle still permits ten times a second, from every connection —
/// grew the map for the life of the room with buckets for messages that never
/// existed. Nothing could evict them, because eviction is driven by messages
/// leaving the history and these had never been in it, and none of it was
/// visible to the memory ceiling.
///
/// The check is O(1) against the id index, so refusing costs no more than
/// accepting (§1.1).
#[tokio::test]
async fn a_reaction_for_a_message_that_does_not_exist_is_refused() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    for _ in 0..1000 {
        assert_eq!(
            room.toggle_reaction(Uuid::new_v4(), "🔥", "attacker"),
            None,
            "a reaction must be refused when there is no such message"
        );
    }

    assert!(
        room.reactions.is_empty(),
        "refused reactions must leave nothing behind; found {} entries",
        room.reactions.len()
    );

    // A real message is still reactable, and stops being so once it is gone.
    let id = message_in(&mut room, &tracker, "real");
    assert!(room.toggle_reaction(id, "🔥", "alice").is_some());
    assert_eq!(room.reactions.len(), 1);

    room.retain_newest(0, &tracker);
    assert!(
        room.toggle_reaction(id, "🎉", "alice").is_none(),
        "a message trimmed out of history is no longer reactable"
    );
    assert!(room.reactions.is_empty());
}

/// Reacting to a message the room does not hold changes nothing and says
/// nothing, through the event path a client actually uses.
///
/// `toggle_reaction` refusing is tested directly; this is the arm in
/// `apply_client_event` that acts on the refusal — without it the server would
/// broadcast a reaction it had not recorded.
#[tokio::test]
async fn a_react_event_for_an_unknown_message_broadcasts_nothing() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        rooms.insert("ghost-react".to_string(), room);
    }

    let mut receiver = state
        .rooms
        .read()
        .await
        .get("ghost-react")
        .unwrap()
        .sender
        .subscribe();

    apply_client_event(
        &state,
        "ghost-react",
        "u1",
        "otter",
        ClientEvent::React {
            message_id: Uuid::new_v4().to_string(),
            emoji: "🔥".to_string(),
        },
    )
    .await;

    assert!(
        receiver.try_recv().is_err(),
        "a refused reaction must not be broadcast as though it had happened"
    );
    assert!(
        state
            .rooms
            .read()
            .await
            .get("ghost-react")
            .unwrap()
            .reactions
            .is_empty()
    );
}
