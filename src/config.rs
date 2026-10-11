//! Every tunable in one place.
//!
//! No magic numbers in logic modules. Each constant carries the reason for its
//! value, because several are game mechanics rather than technical limits
//! (ENGINEERING-STANDARDS.md §7).

use std::time::Duration;

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

/// Hard cap on live rooms; rooms are the only allocation not bounded by user
/// input.
///
/// `the_memory_budget_still_closes` requires every room to afford at least 8
/// images at [`MAX_ATTACHMENT_BYTES`] within [`MAX_TOTAL_ROOMS_MEMORY`], and on
/// the 256 MiB `lite` container a higher count would push the ceiling close
/// enough to the box's limit to risk an OOM kill.
pub(crate) const MAX_ROOMS: usize = 50;
pub(crate) const MAX_ROOM_NAME_LEN: usize = 50;
pub(crate) const MIN_ROOM_NAME_LEN: usize = 3;
pub(crate) const ROOM_NAME_REGEX: &str = "^[a-zA-Z0-9][a-zA-Z0-9-_]*[a-zA-Z0-9]$";

/// The one room that is never garbage-collected. Everything else is ephemeral.
pub(crate) const MAIN_ROOM: &str = "main";

/// Connected users allowed in a single room.
pub(crate) const MAX_USERS_PER_ROOM: usize = 100;

/// The novachannel demo room (session/nova.rs). Never garbage-collected, like
/// [`MAIN_ROOM`], but it doesn't fade on `main`'s schedule either. See
/// [`NOVA_MAX_USERS`] for its other difference.
pub(crate) const NOVA_ROOM: &str = "nova";

/// Connected users allowed in [`NOVA_ROOM`]. Nova once resealed each message per
/// recipient (O(room size)); it now uses `novachannel::group::Group`, sealing a
/// broadcast once, so the limit is set by [`NOVA_GROUP_CAPACITY`]: a group's
/// leaf count is fixed at creation, and this must not exceed it.
pub(crate) const NOVA_MAX_USERS: usize = 60;

/// The `nova` room's `Group` capacity, fixed for the process's lifetime at
/// [`Group::create`](novachannel::group::Group::create). Must be a power of two
/// and at least [`NOVA_MAX_USERS`]; 64 is the next one up.
pub(crate) const NOVA_GROUP_CAPACITY: usize = 64;

/// Length of an RLN rate-limit epoch (`session/nova_rln.rs`). Posting two
/// different anonymous messages in one epoch reveals the member's identity
/// secret; that is RLN's mechanism, not a bug. Thirty seconds is short enough
/// to demonstrate live and long enough not to cramp one post per member.
pub(crate) const NOVA_RLN_EPOCH_SECONDS: u64 = 30;

/// Participants in the `nova` MPC/FROST DKG (`session/nova_operator.rs`), the
/// same size as the earlier in-process simulation, so going distributed changed
/// only where the math runs.
pub(crate) const NOVA_OPERATOR_COUNT: u32 = 5;

/// How many of [`NOVA_OPERATOR_COUNT`] live operators are needed for a
/// ceremony or a demo round.
pub(crate) const NOVA_OPERATOR_THRESHOLD: u32 = 3;

/// How long the coordinator waits for one operator's reply
/// (`session/nova_operator.rs::request_reply`) before treating the quorum as
/// unavailable: generous for a home connection, bounded so a silent operator
/// can't hang a demo forever.
pub(crate) const NOVA_OPERATOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The connected-user ceiling for `room`: [`MAX_USERS_PER_ROOM`], or
/// [`NOVA_MAX_USERS`] for [`NOVA_ROOM`].
pub(crate) fn room_user_limit(room: &str) -> usize {
    if room == NOVA_ROOM {
        NOVA_MAX_USERS
    } else {
        MAX_USERS_PER_ROOM
    }
}

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

/// Ceiling on the rendered HTML a message may become.
///
/// [`MAX_MESSAGE_LEN`] bounds the Markdown typed, not what it renders to, and the
/// HTML is what history stores and every socket receives. Amplification reaches
/// 7.2x (`[a](b)` repeated to the cap renders to 57 KB).
///
/// 5x is the escaping worst case (`&` to `&amp;`; measured 8,000 to 40,000), so
/// `render_message_html`'s plain-text fallback always fits. Ordinary formatted
/// prose measures under 4x.
pub(crate) const MAX_RENDERED_MESSAGE_LEN: usize = MAX_MESSAGE_LEN * 5;
pub(crate) const MAX_MESSAGES_PER_ROOM: usize = 500;
pub(crate) const MAX_MESSAGE_AGE: Duration = Duration::from_secs(86_400 * 30);
pub(crate) const MAX_PAYLOAD_SIZE: usize = 512 * 1024;

/// Caps on the quoted-reply block, which is all client-supplied. Without them a
/// one-character message could carry a half-megabyte "preview" into history
/// and every socket.
pub(crate) const MAX_REPLY_AUTHOR_LEN: usize = 64;
pub(crate) const MAX_REPLY_PREVIEW_LEN: usize = 200;

/// Per-message overhead charged on top of the payload, for `Vec` and allocator
/// costs the string lengths can't see.
pub(crate) const ESTIMATED_MESSAGE_SIZE: usize = 1024;

/// Messages removed per cleanup pass. Bounds how long a GC sweep can hold the
/// room write lock — the lock is what every connected user contends on.
pub(crate) const CLEANUP_BATCH_SIZE: usize = 100;

// Image attachments
//
// No object store or database: an image is bytes in the room's history and
// disappears with it, so every constant here is a memory decision. It can't be
// a remote link either, since the CSP names no external origin and fetching
// one would disclose every visitor (§5.7); `img-src 'self' data:` is enough.

/// Largest base64 payload one attachment may carry: a ceiling on the client's
/// downscaled re-encode, not the picked file (a 12 MP photo arrives well under
/// it). About 96 KB of image.
pub(crate) const MAX_ATTACHMENT_BYTES: usize = 128 * 1024;

/// Largest pixel dimension the client may report on either axis. Dimensions let
/// the client reserve space before decoding (§10.3); they come from an
/// untrusted source, so they're bounded.
pub(crate) const MAX_ATTACHMENT_DIMENSION: u32 = 4096;

/// Attachment bytes one room keeps before its oldest images fade.
///
/// Derived, not picked: [`MAX_TOTAL_ROOMS_MEMORY`] / 2 / [`MAX_ROOMS`], so every
/// room at budget is exactly half the ceiling and one room of photographs can't
/// stop the others accepting messages (§1.1, §3).
///
/// Decimal on purpose: it was `2 * 1024 * 1024` (2.097 MB), so with 100 rooms
/// the "half" of a 400 MB ceiling came to 209.7 MB.
/// `the_memory_budget_still_closes` asserts the arithmetic and caught that on
/// its first run.
///
/// Past the budget the oldest payloads are dropped while their messages stay:
/// §7's fade applied to the most expensive thing in the room.
pub(crate) const MAX_ROOM_ATTACHMENT_BYTES: usize = MAX_TOTAL_ROOMS_MEMORY / 2 / MAX_ROOMS;

/// Image types an attachment may declare, checked against its magic bytes.
///
/// **`image/svg+xml` must stay absent.** An SVG is a document that can carry
/// `<script>`; these four are raster formats a decoder accepts or rejects.
pub(crate) const ALLOWED_ATTACHMENT_MIMES: &[&str] =
    &["image/gif", "image/jpeg", "image/png", "image/webp"];

// ---------------------------------------------------------------------------
// Reactions
// ---------------------------------------------------------------------------

/// Distinct emoji per message. Reactions are the one message state that grows
/// after storage, so they can drift from the memory accounting; this bounds how
/// far.
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

// Game mechanics
//
// Product decisions, not technical limits: a room nobody talks in disappears,
// and a lurker is disconnected so the count means people actually here (§7).
// A spawned room (a spark: brief and private) and `main` (the hearth: always
// findable) were both timed at 600 seconds by coincidence until §7.4.

/// Grace period before an empty room (nobody connected) is deleted: long enough
/// to click back in, short enough that an abandoned room doesn't linger (§7.4).
pub(crate) const EMPTY_ROOM_CLEANUP_DELAY: Duration = Duration::from_secs(300);

/// A user who has sent nothing for this long is disconnected, so the count
/// means people actually here (§7.1). It is the silent person who stops
/// counting, not the room.
pub(crate) const USER_IDLE_MESSAGE_TIMEOUT: Duration = Duration::from_secs(600);

/// WebSocket close code for an idle eviction (4000-4999 is reserved for
/// applications).
///
/// Without it the client can't tell eviction from a dropped connection and
/// reconnects at once. That stopped rooms ever fading: the room was empty only
/// for the instant before the browser came back, never for
/// `EMPTY_ROOM_CLEANUP_DELAY`, so one open tab kept it alive forever.
///
/// The client must use the same number (constraint #12;
/// `client_and_server_agree_on_the_idle_close_code`).
pub(crate) const IDLE_CLOSE_CODE: u16 = 4001;

/// WebSocket close code for "a newer connection under your identity took your
/// place".
///
/// A second connection with the same cookie (a second tab) is `reclaimed`: the
/// room moves the identity onto the new `connection_id`. Without this code the
/// first tab was never told, and its socket stayed open indefinitely, still
/// subscribed and refreshing its idle timer while `connected_user_count` no
/// longer counted it.
///
/// Distinct from [`IDLE_CLOSE_CODE`] because it wasn't an idle eviction, and the
/// client must not treat it as a dropped connection either, or the old tab
/// reconnects and steals the identity back.
///
/// The client must use the same number; pinned by its `PARITY` entry
/// (`every_mirrored_constant_matches_its_source_of_truth`).
pub(crate) const SUPERSEDED_CLOSE_CODE: u16 = 4002;

/// `main` is never deleted, so after this long idle its history trims to
/// [`MAIN_ROOM_FADE_KEEP`].
///
/// Thirty minutes, the hearth's patience. It applies on room-wide idle whoever
/// is connected (constraint #3), so it is the one mechanic a present viewer can
/// watch happen (§7.4).
pub(crate) const MAIN_ROOM_FADE_IDLE: Duration = Duration::from_secs(1800);

/// How much of `main` survives a fade: enough for a real thread (raised from
/// 50).
pub(crate) const MAIN_ROOM_FADE_KEEP: usize = 75;

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

/// Process-wide ceiling on tracked room memory (history, attachments).
///
/// Sized for the 256 MiB `lite` container (`cloudflare/wrangler.jsonc`). This
/// counts tracked bytes only; the binary, runtime, OS and connection buffers sit
/// on top. The old 400 MB was never checked against a deployment target and
/// would have left under 60 MB for everything else.
pub(crate) const MAX_TOTAL_ROOMS_MEMORY: usize = 150_000_000;

/// Fraction of the ceiling at which a room prunes proactively, as (numerator,
/// denominator); hitting the hard cap drops live messages, so this keeps that
/// rare.
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
