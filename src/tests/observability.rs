//! What the logs say, treated as a contract.
//!
//! An operator reads the log to understand a running server: who arrived, who
//! left, which room faded, why a connection was refused. None of that is
//! checked by any behavioural test — the server can do exactly the right thing
//! and say nothing about it, and that is indistinguishable from a server doing
//! the wrong thing quietly.
//!
//! It is not a hypothetical. The reported "skink left / stinks joined" bug ran
//! for as long as it did because *nothing said why a visitor got a new name*.
//! Every step behaved correctly; the log recorded arrivals and departures and
//! never the decision between them. `admit_user` now logs its outcome —
//! reclaimed, recognised or fresh — and the test below is the reason it cannot
//! quietly stop.

use super::*;

/// Every admission says what happened to the visitor's identity.
///
/// Three outcomes, and the difference between them is the whole of §5.9a:
/// `reclaimed` is a reconnecting user taking their slot back, `recognised` is
/// somebody whose cookie is good but who is new to this room, and `fresh` is a
/// visitor with no usable identity at all. A room repeating `fresh` for the
/// same person is the flapping bug, visible at a glance.
#[tokio::test]
async fn every_admission_says_what_became_of_the_identity() {
    let state = Arc::new(AppState::new());

    // No cookie: a brand new visitor.
    let logged = capturing_logs(|| async {
        admit_user(&state, "watched-room", "c1", None).await;
    })
    .await;
    assert!(
        logged.contains("admitted") && logged.contains("fresh"),
        "a visitor with no identity must be logged as fresh: {logged}"
    );

    // A good cookie for a room they have not been in: recognised, and issued a
    // slot of their own.
    let carried = crate::identity::UserCookie {
        user_id: Uuid::new_v4().to_string(),
        animal_name: ANIMAL_NAMES[0].to_string(),
    };
    let mut issued = None;
    let logged = capturing_logs(|| async {
        issued = admit_user(&state, "watched-room", "c2", Some(&carried)).await;
    })
    .await;
    assert!(
        logged.contains("recognised"),
        "a visitor with a usable cookie must be logged as recognised: {logged}"
    );

    // Coming back with what the server actually set, which is what a browser
    // sends — the identity it was issued, not the one it arrived with.
    let (issued_id, issued_name) = issued.expect("admitted");
    let returning = crate::identity::UserCookie {
        user_id: issued_id,
        animal_name: issued_name,
    };
    let logged = capturing_logs(|| async {
        admit_user(&state, "watched-room", "c3", Some(&returning)).await;
    })
    .await;
    assert!(
        logged.contains("reclaimed"),
        "a reconnecting visitor must be logged as reclaiming, not as new — the \
         difference is exactly what a room experiences as somebody leaving and \
         a stranger arriving: {logged}"
    );
}

/// A departure is logged, with what is left behind.
#[tokio::test]
async fn a_departure_is_logged_with_the_room_it_leaves() {
    let state = Arc::new(AppState::new());
    let (user_id, _) = admit_user(&state, "leaving", "conn-1", None)
        .await
        .expect("admitted");

    let logged = capturing_logs(|| async {
        cleanup_user(&state, "leaving", &user_id, "conn-1", None).await;
    })
    .await;

    assert!(
        logged.contains("departed") && logged.contains("leaving"),
        "a departure must name the room: {logged}"
    );
    assert!(
        logged.contains("remaining"),
        "and say what is left, which is what tells an operator whether the room \
         is about to fade: {logged}"
    );
}

/// A refused connection says which ceiling refused it.
///
/// Before this, a refusal was silent: the visitor saw an error and the server
/// recorded nothing at all, so "why can nobody connect" had no answer in the
/// logs. Each refusal now names the limit and its configured value, because
/// the useful question is always whether the limit is right rather than
/// whether it fired.
#[tokio::test]
async fn a_refused_connection_says_which_ceiling_refused_it() {
    let state = Arc::new(AppState::new());
    state
        .resource_monitor
        .total_connections
        .store(MAX_CONCURRENT_USERS, Ordering::SeqCst);

    let logged = capturing_logs(|| async {
        let refused = crate::session::check_admission(&state, None, "full").await;
        assert!(refused.is_err(), "the server is at capacity");
    })
    .await;

    assert!(
        logged.contains("refused: server at capacity"),
        "a refusal must say which ceiling it hit: {logged}"
    );
    assert!(
        logged.contains(&MAX_CONCURRENT_USERS.to_string()),
        "and what that ceiling is set to, since that is the actionable part: \
         {logged}"
    );
}

/// A full room says so, rather than failing silently.
#[tokio::test]
async fn a_full_room_says_so_when_it_refuses() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        for (i, animal) in ANIMAL_NAMES.iter().take(MAX_USERS_PER_ROOM).enumerate() {
            room.users.insert(
                format!("u{i}"),
                connected_user(&format!("u{i}"), animal, "c", Instant::now()),
            );
        }
        rooms.insert("packed".to_string(), room);
    }

    let logged = capturing_logs(|| async {
        assert!(
            admit_user(&state, "packed", "c1", None).await.is_none(),
            "the room is full"
        );
    })
    .await;

    assert!(
        logged.contains("room is full") || logged.contains("refused"),
        "a full room must say so rather than refusing silently: {logged}"
    );
}

/// A session that comes and goes leaves a readable trail.
///
/// This is the shape an operator actually reads: one visitor, one arrival, one
/// departure, and nothing in between claiming otherwise. It is also the shape
/// the flapping bug broke — that log had an arrival and a departure per
/// *reconnect*, each with a different name.
#[tokio::test]
async fn a_normal_session_reads_as_one_arrival_and_one_departure() {
    let state = Arc::new(AppState::new());

    let logged = capturing_logs(|| async {
        let (user_id, _) = admit_user(&state, "quiet", "conn-1", None)
            .await
            .expect("admitted");
        cleanup_user(&state, "quiet", &user_id, "conn-1", None).await;
    })
    .await;

    assert_eq!(
        logged.matches("admitted").count(),
        1,
        "one visitor is one arrival: {logged}"
    );
    assert_eq!(
        logged.matches("departed").count(),
        1,
        "and one departure: {logged}"
    );
    assert!(!logged.contains("refused"), "and nothing refused: {logged}");
}
