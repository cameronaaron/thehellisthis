//! Every tunable in one place.
//!
//! No magic numbers live in logic modules — a bare `3600` in a cleanup branch
//! is unreadable and untestable. Each constant here carries the *reason* it has
//! the value it has, because several of them are deliberate game mechanics
//! rather than technical limits (ENGINEERING-STANDARDS.md §7).

use std::time::Duration;

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

/// Hard cap on simultaneously live rooms. Bounds total memory: rooms are the
/// only unbounded-by-user-input allocation in the server.
pub(crate) const MAX_ROOMS: usize = 100;
pub(crate) const MAX_ROOM_NAME_LEN: usize = 50;
pub(crate) const MIN_ROOM_NAME_LEN: usize = 3;
pub(crate) const ROOM_NAME_REGEX: &str = "^[a-zA-Z0-9][a-zA-Z0-9-_]*[a-zA-Z0-9]$";

/// The one room that is never garbage-collected. Everything else is ephemeral.
pub(crate) const MAIN_ROOM: &str = "main";

/// Connected users allowed in a single room.
pub(crate) const MAX_USERS_PER_ROOM: usize = 100;

/// Path segments that can never be a room, because they are real routes or
/// well-known files. Kept sorted for readability.
pub(crate) const RESERVED_ROOM_NAMES: &[&str] = &[
    ".well-known",
    "admin",
    "api",
    "favicon.ico",
    "health",
    "main",
    "metrics",
    "robots.txt",
    "sitemap.xml",
    "ws",
];

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

pub(crate) const MAX_MESSAGE_LEN: usize = 8_000;
pub(crate) const MAX_MESSAGES_PER_ROOM: usize = 500;
pub(crate) const MAX_MESSAGE_AGE: Duration = Duration::from_secs(86_400 * 30);
pub(crate) const MAX_PAYLOAD_SIZE: usize = 512 * 1024;

/// Caps on the quoted-reply block, which is composed entirely of
/// client-supplied strings.
///
/// [`MAX_MESSAGE_LEN`] bounds a message's own text and nothing else, so without
/// these a client could attach a half-megabyte "preview" to a one-character
/// message, and the server would store it in history and broadcast it to
/// everyone in the room.
pub(crate) const MAX_REPLY_AUTHOR_LEN: usize = 64;
pub(crate) const MAX_REPLY_PREVIEW_LEN: usize = 200;

/// Per-message bookkeeping overhead charged on top of the actual bytes, so the
/// memory ceiling accounts for `Vec`/allocator overhead rather than only the
/// string payload it can see.
pub(crate) const ESTIMATED_MESSAGE_SIZE: usize = 1024;

/// Messages removed per cleanup pass. Bounds how long a GC sweep can hold the
/// room write lock — the lock is what every connected user contends on.
pub(crate) const CLEANUP_BATCH_SIZE: usize = 100;

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

pub(crate) const MAX_MESSAGES_PER_WINDOW: usize = 30;
pub(crate) const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
pub(crate) const MAX_ROOM_JOIN_ATTEMPTS: usize = 10;
pub(crate) const TYPING_EVENT_MIN_INTERVAL: Duration = Duration::from_millis(200);
pub(crate) const READ_RECEIPT_MIN_INTERVAL: Duration = Duration::from_millis(200);

/// Window in which an identical message from the same user is treated as an
/// accidental double-send rather than intent.
pub(crate) const DUPLICATE_MESSAGE_WINDOW: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

pub(crate) const MAX_CONCURRENT_USERS: usize = 400;
pub(crate) const MAX_CONCURRENT_CONNECTIONS_PER_IP: usize = 3;
pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(6);

/// Lifetime of the identity cookies — how long a returning visitor keeps the
/// same animal name.
pub(crate) const INACTIVE_TIMEOUT: Duration = Duration::from_secs(3_600);

// ---------------------------------------------------------------------------
// Game mechanics
//
// These are product decisions, not technical limits. Scarcity is the point:
// a room that nobody talks in disappears, and a user who lurks is disconnected
// so the user count means "people actually here" (ENGINEERING-STANDARDS.md §7).
// ---------------------------------------------------------------------------

/// Grace period before an empty room is deleted.
pub(crate) const EMPTY_ROOM_CLEANUP_DELAY: Duration = Duration::from_secs(600);

/// A user who has sent nothing for this long is disconnected.
pub(crate) const USER_IDLE_MESSAGE_TIMEOUT: Duration = Duration::from_secs(600);

/// `main` is never deleted, so it fades instead: once idle this long its
/// history is trimmed to [`MAIN_ROOM_FADE_KEEP`].
pub(crate) const MAIN_ROOM_FADE_IDLE: Duration = Duration::from_secs(600);
pub(crate) const MAIN_ROOM_FADE_KEEP: usize = 50;

// ---------------------------------------------------------------------------
// Housekeeping intervals
// ---------------------------------------------------------------------------

pub(crate) const ROOM_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const RESOURCE_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Minimum spacing between global memory GC sweeps.
pub(crate) const MEMORY_GC_MIN_INTERVAL: Duration = Duration::from_secs(300);

/// How long a disconnected user's slot (and their animal name) is held before
/// being reclaimed, so a refresh does not cost you your identity.
pub(crate) const DISCONNECTED_USER_RETENTION: Duration = Duration::from_secs(3_600);

/// How long an idle per-IP connection counter is kept before being dropped.
pub(crate) const IP_COUNTER_RETENTION: Duration = Duration::from_secs(3_600);

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

pub(crate) const MAX_TOTAL_ROOMS_MEMORY: usize = 400_000_000;

/// Fraction of the memory ceiling at which a room prunes proactively, as
/// (numerator, denominator) — hitting the hard cap drops live messages, so the
/// soft threshold exists to make that rare.
pub(crate) const MEMORY_SOFT_LIMIT_RATIO: (usize, usize) = (9, 10);

// ---------------------------------------------------------------------------
// Security
// ---------------------------------------------------------------------------

pub(crate) const IP_BAN_DURATION: Duration = Duration::from_secs(3_600);
pub(crate) const SUSPICIOUS_ACTIVITY_WINDOW: Duration = Duration::from_secs(60);
pub(crate) const MAX_SUSPICIOUS_EVENTS: usize = 10;

// ---------------------------------------------------------------------------
// Build metadata
// ---------------------------------------------------------------------------

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
