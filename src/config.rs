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

/// Ceiling on the *rendered* HTML a message may become.
///
/// [`MAX_MESSAGE_LEN`] bounds the Markdown a user types; it does not bound what
/// the renderer makes of it, and the rendered HTML is what gets stored in
/// history and broadcast to every socket in the room. Measured amplification
/// for input that passes validation reaches 7.2x — `[a](b)` repeated to the
/// input cap renders to 57 KB — so on its own the input cap bounded nothing
/// that costs memory or bandwidth.
///
/// 5x is not a round number, it is the escaping worst case: `&` becomes
/// `&amp;`, so the plain-text fallback in `render_message_html` produces at
/// most 5x its input (measured: 8 000 → 40 000). Setting the ceiling there
/// means the fallback always fits under it, which is what makes the fallback
/// total rather than something that can itself overflow. Ordinary
/// prose-with-formatting measures under 4x, so nothing legible is affected.
pub(crate) const MAX_RENDERED_MESSAGE_LEN: usize = MAX_MESSAGE_LEN * 5;
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
// Image attachments
//
// There is no object store and no database, so an image is not a file with a
// URL — it is bytes held in the room's history, and it disappears with the room
// like everything else. That makes every constant here a memory decision.
//
// Nor can an image be a *link* to one: the CSP names no external origin, and
// fetching a remote image would tell that host the address of every visitor in
// the room (§5.7). `img-src 'self' data:` is what an inline attachment needs
// and nothing more.
// ---------------------------------------------------------------------------

/// Largest base64 payload a single attachment may carry.
///
/// The client downscales and re-encodes before sending, so this is a ceiling on
/// the *encoded* result rather than on what the user picked — a 12 MP phone
/// photo arrives well under it. Base64 is 4 bytes per 3, so this is ~96 KB of
/// actual image.
pub(crate) const MAX_ATTACHMENT_BYTES: usize = 128 * 1024;

/// Largest pixel dimension the client may report, in either axis.
///
/// Dimensions are sent so the client can reserve the right space before the
/// image decodes, which is what stops a message arriving and shoving the
/// conversation down the page (§10.3). They are display hints from an untrusted
/// source, so they are bounded rather than believed.
pub(crate) const MAX_ATTACHMENT_DIMENSION: u32 = 4096;

/// Attachment bytes a single room keeps before the oldest images fade.
///
/// Deliberately small, and **derived rather than picked**:
/// [`MAX_TOTAL_ROOMS_MEMORY`] / 2 / [`MAX_ROOMS`]. Every room at its budget is
/// then exactly half the process ceiling, leaving the rest for text — one room
/// full of photographs must not be able to stop every other room accepting
/// messages (§1.1, §3).
///
/// Decimal, not binary. This was `2 * 1024 * 1024`, which reads as "2 MB" and
/// is 2.097 MB, so a hundred rooms came to 209.7 MB against a 400 MB ceiling
/// whose half is 200 MB. The comment claiming it was half was wrong by 5%.
/// `the_memory_budget_still_closes` now asserts the arithmetic instead of
/// leaving it to be re-derived by eye — and caught this on its first run.
///
/// Past it the *payloads* of the oldest attachments are dropped while their
/// messages stay, so a conversation keeps its shape and only the pictures age
/// out. That is the §7 fade applied to the most expensive thing in the room,
/// and it is the reason this can be a small number without deleting anything a
/// reader still needs.
pub(crate) const MAX_ROOM_ATTACHMENT_BYTES: usize = MAX_TOTAL_ROOMS_MEMORY / 2 / MAX_ROOMS;

/// Image types an attachment may declare, checked against the payload's own
/// magic bytes rather than trusted.
///
/// **`image/svg+xml` is deliberately absent and must stay absent.** An SVG is a
/// document: it can carry `<script>`, and a browser rendering one from a
/// `data:` URL in an `<img>` is one policy mistake away from executing it. The
/// four here are raster formats a decoder either understands or rejects.
pub(crate) const ALLOWED_ATTACHMENT_MIMES: &[&str] =
    &["image/gif", "image/jpeg", "image/png", "image/webp"];

// ---------------------------------------------------------------------------
// Reactions
// ---------------------------------------------------------------------------

/// Distinct emoji a single message may carry.
///
/// Reactions are the one piece of message state that grows *after* the message
/// is stored, so they are the one piece that can drift out of the memory
/// accounting. This bounds how far.
pub(crate) const MAX_REACTIONS_PER_MESSAGE: usize = 8;

/// Minimum spacing between reaction events from one user, like the typing and
/// read-receipt throttles. A reaction is a click, not a keystroke.
pub(crate) const REACTION_MIN_INTERVAL: Duration = Duration::from_millis(100);

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
