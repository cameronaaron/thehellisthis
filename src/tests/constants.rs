//! Individual backend constants, pinned to their current value.
//!
//! Low-ceremony by design: each of these asserts one `config.rs` number back
//! at itself, so a change to it is a deliberate diff here rather than a
//! silent renumbering nobody notices. They do not explain *why* the number is
//! what it is — that reasoning lives on the constant itself in `config.rs` —
//! and they do not replace `frontend_parity.rs`, which checks a number
//! against something *else* that has to agree with it (the class of bug §7.4
//! found twice: a constant checked against itself proves nothing about
//! whether the thing mirroring it has drifted).

use super::*;

#[tokio::test]
async fn test_empty_room_cleanup_delay_constant() {
    // A spawned room is a spark: gone 5 minutes after the last person leaves
    // it empty (§7.4).
    assert_eq!(EMPTY_ROOM_CLEANUP_DELAY.as_secs(), 300);
}

#[tokio::test]
async fn test_max_messages_constant_for_history_limit() {
    // Frontend displays "max 500" - verify constant matches
    assert_eq!(MAX_MESSAGES_PER_ROOM, 500);
}

#[tokio::test]
async fn test_max_rooms_constant() {
    assert_eq!(MAX_ROOMS, 100);
}

#[tokio::test]
async fn test_max_room_name_len_constant() {
    assert_eq!(MAX_ROOM_NAME_LEN, 50);
}

#[tokio::test]
async fn test_min_room_name_len_constant() {
    assert_eq!(MIN_ROOM_NAME_LEN, 3);
}

#[tokio::test]
async fn test_max_message_len_constant() {
    assert_eq!(MAX_MESSAGE_LEN, 8000);
}

#[tokio::test]
async fn test_max_messages_per_room_constant() {
    assert_eq!(MAX_MESSAGES_PER_ROOM, 500);
}

#[tokio::test]
async fn test_max_concurrent_connections_per_ip_constant() {
    assert_eq!(MAX_CONCURRENT_CONNECTIONS_PER_IP, 3);
}

#[tokio::test]
async fn test_max_concurrent_users_constant() {
    assert_eq!(MAX_CONCURRENT_USERS, 400);
}

#[tokio::test]
async fn test_max_messages_per_window_constant() {
    assert_eq!(MAX_MESSAGES_PER_WINDOW, 30);
}

#[tokio::test]
async fn test_rate_limit_window_constant() {
    assert_eq!(RATE_LIMIT_WINDOW.as_secs(), 60);
}

#[tokio::test]
async fn test_heartbeat_interval_constant() {
    assert_eq!(HEARTBEAT_INTERVAL.as_secs(), 5);
}

#[tokio::test]
async fn test_inactive_timeout_constant() {
    assert_eq!(INACTIVE_TIMEOUT.as_secs(), 3600);
}

#[tokio::test]
async fn test_max_payload_size_constant() {
    assert_eq!(MAX_PAYLOAD_SIZE, 512 * 1024);
}

#[tokio::test]
async fn test_sanitize_timeout_constant() {
    assert_eq!(DUPLICATE_MESSAGE_WINDOW.as_millis(), 50);
}

#[tokio::test]
async fn test_typing_event_min_interval_constant() {
    assert_eq!(TYPING_EVENT_MIN_INTERVAL.as_millis(), 200);
}

#[tokio::test]
async fn test_read_receipt_min_interval_constant() {
    assert_eq!(READ_RECEIPT_MIN_INTERVAL.as_millis(), 200);
}

#[tokio::test]
async fn test_max_room_join_attempts_constant() {
    assert_eq!(MAX_ROOM_JOIN_ATTEMPTS, 10);
}

#[tokio::test]
async fn test_cleanup_batch_size_constant() {
    assert_eq!(CLEANUP_BATCH_SIZE, 100);
}

#[tokio::test]
async fn test_max_message_age_constant() {
    assert_eq!(MAX_MESSAGE_AGE.as_secs(), 86400 * 30);
}

#[tokio::test]
async fn test_max_total_rooms_memory_constant() {
    assert_eq!(MAX_TOTAL_ROOMS_MEMORY, 400_000_000);
}

#[tokio::test]
async fn test_estimated_message_size_constant() {
    assert_eq!(ESTIMATED_MESSAGE_SIZE, 1024);
}
