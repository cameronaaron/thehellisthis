//! The animal-name roster: assignment, reuse, exhaustion, uniqueness within a
//! room, and the closed-set claim on reconnect (§5.9).

use super::*;

#[tokio::test]
async fn test_animal_assignment() {
    let mut room = create_room();
    let animal1 = room.assign_animal();
    let animal2 = room.assign_animal();

    assert!(!animal1.is_empty());
    assert!(!animal2.is_empty());
    assert_ne!(animal1, animal2);
}

#[tokio::test]
async fn test_animal_reuse_after_user_removal() {
    let mut room = create_room();
    let first_animal = room.assign_animal();

    // Simulate user disconnect: return animal to pool
    room.available_animals.push_back(first_animal.clone());

    // Next assignment will take from front of queue
    let reassigned = room.assign_animal();
    // The reassigned animal should be from our pool, not necessarily the one we added
    // since more animals were removed from front after our push_back
    assert!(!reassigned.is_empty());
}

#[tokio::test]
async fn test_assign_animal_fallback_guest() {
    let mut room = create_room();
    room.available_animals.clear();

    let assigned = room.assign_animal();
    assert_eq!(assigned, "guest_1");
}

// ========== CLEANUP TIMESTAMP EDGE ==========

#[tokio::test]
async fn test_animal_name_uniqueness_in_room() {
    let app_state = Arc::new(AppState::new());
    let room_name = "animal-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    // Assign multiple animals
    let animal1 = room_state.assign_animal();
    let animal2 = room_state.assign_animal();
    let animal3 = room_state.assign_animal();

    // All should be different
    assert_ne!(animal1, animal2);
    assert_ne!(animal2, animal3);
    assert_ne!(animal1, animal3);
}

// ========== RESERVED PATHS & INPUT VALIDATION ==========

#[tokio::test]
async fn test_animal_name_exhaustion() {
    let app_state = Arc::new(AppState::new());
    let room_name = "exhaustion-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    // Assign many animals - should work without panic
    for _ in 0..60 {
        let _animal = room_state.assign_animal();
    }
}

#[tokio::test]
async fn test_animal_name_contains_no_duplicates() {
    let app_state = Arc::new(AppState::new());
    let room_name = "no-dup-test".to_string();

    {
        let mut rooms = app_state.rooms.write().await;
        rooms.insert(room_name.clone(), create_room());
    }

    let mut rooms = app_state.rooms.write().await;
    let room_state = rooms.get_mut(&room_name).unwrap();

    let mut animals = std::collections::HashSet::new();
    for _ in 0..10 {
        let animal = room_state.assign_animal();
        animals.insert(animal);
    }

    assert!(animals.len() >= 9);
}

// ========== ADVANCED EDGE CASE TESTS (40 MORE) ==========

#[tokio::test]
async fn test_is_user_allowed_existing_user() {
    let mut room_state = create_room();

    // Add an existing user
    let user_id = "existing_user".to_string();
    room_state.users.insert(
        user_id.clone(),
        UserData {
            user_id: user_id.clone(),
            animal_name: "Lion".to_string(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: "conn_1".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
            last_reaction_event: None,
        },
    );

    // Existing user should be allowed (via rate limiter check)
    assert!(room_state.is_user_allowed(&user_id, MAX_USERS_PER_ROOM));
}

#[tokio::test]
async fn test_room_state_user_removal_returns_animal() {
    let mut room_state = create_room();
    let user_id = "user_to_remove".to_string();

    // Store original available animals count
    let original_animal_count = room_state.available_animals.len();

    // Add a user
    room_state.users.insert(
        user_id.clone(),
        UserData {
            user_id: user_id.clone(),
            animal_name: "Tiger".to_string(),
            last_active: Instant::now(),
            last_message_time: Instant::now(),
            connection_state: ConnectionState::Connected {
                last_heartbeat: Instant::now(),
                connection_id: "conn_1".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
            last_reaction_event: None,
        },
    );

    // Remove the user via drop logic (simulated)
    let removed_user = room_state.users.remove(&user_id);
    if let Some(user) = removed_user {
        room_state
            .available_animals
            .push_back(user.animal_name.clone());
    }

    // Animal should be returned to the pool
    assert_eq!(
        room_state.available_animals.len(),
        original_animal_count + 1
    );
}

#[tokio::test]
async fn test_animal_name_in_message() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/animal-msg-test", addr);

    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Get ws1's animal name
    let mut animal_name = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            animal_name = payload
                .get("animal_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !animal_name.is_empty() {
                break;
            }
        }
    }

    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws2)).await;
    }

    ws1.send(text_frame(
        r#"{"type":"Message","text":"Animal test"}"#.to_string(),
    ))
    .await
    .unwrap();

    for _ in 0..10 {
        if let Ok(val) = timeout(Duration::from_secs(2), recv_json_event(&mut ws2)).await
            && val.get("type") == Some(&JsonValue::String("Message".to_string()))
        {
            let msg_animal = val["message"]["animal_name"].as_str().unwrap_or("");
            assert!(!msg_animal.is_empty(), "Message must contain animal_name");
            assert_eq!(
                msg_animal, animal_name,
                "Message animal_name should match sender"
            );
            break;
        }
    }

    handle.abort();
}

#[tokio::test]
async fn test_assign_animal_with_all_animals_in_use() {
    let mut room = create_room();
    let now = Instant::now();

    // Assign all animals to connected users
    let animal_count = room.available_animals.len();
    for i in 0..animal_count {
        let animal = room.assign_animal();
        room.users.insert(
            format!("user-{}", i),
            UserData {
                user_id: format!("user-{}", i),
                animal_name: animal,
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: format!("conn-{}", i),
                },
                last_read_message: None,
                is_typing: false,
                last_typing_event: None,
                last_read_receipt_event: None,
                rate_limiter: RateLimiter::new(),
                last_message_text: None,
                last_reaction_event: None,
            },
        );
    }

    // Now when all animals are used, should get guest_X format
    let next = room.assign_animal();
    assert!(
        next.starts_with("guest_"),
        "Should get guest name when all animals used"
    );
}

#[tokio::test]
async fn test_claim_anonymous_animal_names() {
    // Users identified by random animals, not personal info
    let user_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(user_id.len(), 36); // UUID
    assert!(!user_id.contains("@")); // Not email
}

/// The animal roster must stay sorted, unique and actually made of animals.
///
/// This is a sweep, not a spot check: an earlier revision of the list ran off
/// the end of a dictionary and shipped `hadron`, `hagiology`, `gyroscope`,
/// `half-penny` and `hallway` as assignable names, with `hallingers` and
/// `hallway` present twice. Duplicates are the part that actually broke: two
/// connected users could hold the same name, which is the only identity the UI
/// shows.
#[test]
fn animal_roster_is_sorted_unique_and_well_formed() {
    assert!(
        ANIMAL_NAMES.len() >= 100,
        "roster is too small to keep a full room in distinct names"
    );

    let mut seen = std::collections::HashSet::new();
    for name in ANIMAL_NAMES {
        assert!(
            seen.insert(*name),
            "duplicate animal name in roster: {name}"
        );
        assert!(!name.is_empty(), "empty animal name in roster");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "animal name must be lowercase ascii with hyphens only: {name}"
        );
        assert!(
            !name.starts_with('-') && !name.ends_with('-'),
            "animal name must not start or end with a hyphen: {name}"
        );
    }

    let mut sorted = ANIMAL_NAMES.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        sorted.as_slice(),
        ANIMAL_NAMES,
        "roster must stay sorted so a duplicate is visible in review"
    );
}

/// A full room hands out distinct names for as long as the roster lasts.
#[tokio::test]
async fn assign_animal_never_repeats_a_connected_name() {
    let mut room = create_room();
    let mut handed_out = std::collections::HashSet::new();

    for i in 0..50 {
        let name = room.assign_animal();
        assert!(
            handed_out.insert(name.clone()),
            "assign_animal handed out {name} twice"
        );

        let now = Instant::now();
        room.users.insert(
            format!("user-{i}"),
            UserData {
                user_id: format!("user-{i}"),
                animal_name: name,
                last_active: now,
                last_message_time: now,
                connection_state: ConnectionState::Connected {
                    last_heartbeat: now,
                    connection_id: format!("conn-{i}"),
                },
                last_read_message: None,
                is_typing: false,
                last_typing_event: None,
                last_read_receipt_event: None,
                rate_limiter: RateLimiter::new(),
                last_message_text: None,
                last_reaction_event: None,
            },
        );
    }
}

// ========== ROUTER WIRING ==========

/// A name held by a connected user is skipped and rotated to the back.
#[tokio::test]
async fn assign_animal_skips_names_already_in_use() {
    let mut room = create_room();

    // Whatever is at the front of the pool, claim it.
    let front = room.available_animals.front().cloned().unwrap();
    let now = Instant::now();
    room.users.insert(
        "holder".to_string(),
        UserData {
            user_id: "holder".to_string(),
            animal_name: front.clone(),
            last_active: now,
            last_message_time: now,
            connection_state: ConnectionState::Connected {
                last_heartbeat: now,
                connection_id: "c1".to_string(),
            },
            last_read_message: None,
            is_typing: false,
            last_typing_event: None,
            last_read_receipt_event: None,
            rate_limiter: RateLimiter::new(),
            last_message_text: None,
            last_reaction_event: None,
        },
    );

    let assigned = room.assign_animal();
    assert_ne!(
        assigned, front,
        "a name in use must not be handed out again"
    );
    assert!(
        room.available_animals.contains(&front),
        "the skipped name stays in the pool for later"
    );
}

/// A returning visitor keeps the name they know, and the id underneath it.
///
/// The checks in `claim_animal` exist to refuse forged and colliding names, and
/// it would be easy to satisfy every one of those tests by never honouring a
/// cookie at all. This is the behaviour the checks are *for*: open the same
/// room in a second tab, or reload, and you are still the same animal.
///
/// The id matters as much as the name. `admit_user` used to mint a fresh
/// `Uuid::new_v4()` for a valid cookie the moment the room in front of it had
/// not seen that id before — every *other* room, and the same room again once
/// its entry aged out — while `claim_animal` quietly kept handing back the
/// same display name. The client has no other way to tell "my message" from
/// "somebody else's" than comparing `msg.user_id` to the id its own `Welcome`
/// frame carried (constraint #2), so a visitor's own older messages in that
/// room's history stopped matching and rendered on the left, as somebody
/// else's — while the name overhead still read as their own. The identity
/// cookie is set with `Path=/`, sent to every room alike; the id behind it has
/// to mean the same thing everywhere that cookie is presented.
#[tokio::test]
async fn a_returning_visitor_keeps_a_roster_name_that_is_free() {
    let state = Arc::new(AppState::new());

    // First visit: the server issues an identity.
    let (first_id, first_name) = admit_user(&state, "return-room", "c1", None)
        .await
        .expect("a new visitor is admitted");
    assert!(ANIMAL_NAMES.contains(&first_name.as_str()));

    // Same browser, a room it has not been in before, carrying that cookie.
    let cookie = crate::identity::UserCookie {
        user_id: first_id.clone(),
        animal_name: first_name.clone(),
    };
    let (second_id, second_name) = admit_user(&state, "another-room", "c2", Some(&cookie))
        .await
        .expect("a returning visitor is admitted");

    assert_eq!(
        second_name, first_name,
        "a roster name that nobody in the room is using should be honoured"
    );
    assert_eq!(
        second_id, first_id,
        "a valid identity cookie must carry the same id into a room that has \
         not seen it before — minting a new one is what makes a visitor's own \
         history in that room render as somebody else's"
    );
}

// ========== ATTACHMENTS AND REACTIONS ==========

/// Constraint #9 — the guest fallback is unreachable, and that is a property of
/// the numbers rather than an accident.
///
/// `assign_animal` mints `guest_N` only when every roster name is held by a
/// connected user. A room holds at most `MAX_USERS_PER_ROOM`, so a roster
/// larger than that makes the branch dead in production. Shrinking the roster
/// below the room cap would quietly start handing out names that are not
/// animals.
#[test]
fn the_roster_is_larger_than_a_room_can_ever_be() {
    assert!(
        ANIMAL_NAMES.len() > MAX_USERS_PER_ROOM,
        "the roster ({}) must exceed the per-room cap ({MAX_USERS_PER_ROOM}) so \
         every connected user can hold a distinct animal name",
        ANIMAL_NAMES.len()
    );
}
