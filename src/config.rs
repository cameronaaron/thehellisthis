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
///
/// Sized against [`MAX_TOTAL_ROOMS_MEMORY`] and the container it runs in —
/// `the_memory_budget_still_closes` requires every room to afford at least 8
/// images at [`MAX_ATTACHMENT_BYTES`], which on the 256 MiB `lite` Cloudflare
/// Container instance this deploys to left no room for a higher count without
/// pushing the memory ceiling close enough to the box's own limit to risk an
/// OOM kill from the runtime, OS and connection overhead alone.
pub(crate) const MAX_ROOMS: usize = 50;
pub(crate) const MAX_ROOM_NAME_LEN: usize = 50;
pub(crate) const MIN_ROOM_NAME_LEN: usize = 3;
pub(crate) const ROOM_NAME_REGEX: &str = "^[a-zA-Z0-9][a-zA-Z0-9-_]*[a-zA-Z0-9]$";

/// The one room that is never garbage-collected. Everything else is ephemeral.
pub(crate) const MAIN_ROOM: &str = "main";

/// Connected users allowed in a single room.
pub(crate) const MAX_USERS_PER_ROOM: usize = 100;

/// The novachannel proof-of-concept room (session/nova.rs). Never
/// garbage-collected — same reasoning as [`MAIN_ROOM`], a demo link should
/// stay alive — but unlike `main` it does not fade: there is no reason a
/// research demo's history should shrink on the flagship room's schedule.
/// See [`NOVA_MAX_USERS`] for the other way it deliberately differs from
/// every other room.
pub(crate) const NOVA_ROOM: &str = "nova";

/// Connected users allowed in [`NOVA_ROOM`] — far below
/// [`MAX_USERS_PER_ROOM`], because nova's message path is not O(1) the way
/// every other room's is: `novachannel` is a pairwise channel, not a group
/// one, so the server reseals each outgoing message once per connected
/// recipient (`session/nova.rs::forward_sealed`). That cost is paid only by
/// this one room, and only up to this ceiling.
pub(crate) const NOVA_MAX_USERS: usize = 12;

/// How long an RLN rate-limit epoch lasts (`session/nova_rln.rs`). A member
/// who posts a second *different* anonymous message inside one epoch leaks
/// their identity secret to anyone who observes both proofs — that's the
/// mechanism RLN is named for, not a bug. Thirty seconds is short enough to
/// demonstrate live (send two anonymous messages a few seconds apart and
/// watch the second one recover the first sender's key) without being so
/// short that a genuinely single anonymous post per member feels cramped.
pub(crate) const NOVA_RLN_EPOCH_SECONDS: u64 = 30;

/// Total participants in the `nova` MPC/FROST DKG (`session/nova_operator.rs`)
/// — matches the size the earlier in-process simulation used, so the only
/// thing that changed when it became genuinely distributed is *where* the
/// math runs, not what it demonstrates.
pub(crate) const NOVA_OPERATOR_COUNT: u32 = 5;

/// How many of [`NOVA_OPERATOR_COUNT`] live operators are needed for a
/// ceremony or a demo round.
pub(crate) const NOVA_OPERATOR_THRESHOLD: u32 = 3;

/// How long the coordinator waits for one operator's reply to a
/// decrypt/sign request before treating the quorum as unavailable
/// (`session/nova_operator.rs::request_reply`) — generous for a real
/// network round trip to a process that might be on someone's home
/// connection, but bounded so one silent operator cannot hang a demo
/// request forever.
pub(crate) const NOVA_OPERATOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The connected-user ceiling for `room`. Every room uses
/// [`MAX_USERS_PER_ROOM`] except [`NOVA_ROOM`], which is far lower — see
/// [`NOVA_MAX_USERS`] for why.
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
//
// A room you create and `main` are not the same kind of thing, and until
// §7.4 they were timed as though they were: both 600 seconds, coincidentally.
// A spawned room is a spark — struck on purpose, private, meant to be brief.
// `main` is the hearth — the one place always findable, and its whole point is
// that it does not go out. Giving them the same clock told neither story;
// see §7.4.
// ---------------------------------------------------------------------------

/// Grace period before an *empty* room is deleted — nobody connected, not
/// merely nobody talking. A spark, not a hearth: five minutes is long enough
/// for someone to realise they meant to stay and click back in, and short
/// enough that an abandoned room does not linger as a locked door with a
/// light seemingly still on. See §7.4.
pub(crate) const EMPTY_ROOM_CLEANUP_DELAY: Duration = Duration::from_secs(300);

/// A user who has sent nothing for this long is disconnected — the one timer
/// here that was already honest and stays as it was. It is the real, personal
/// stake behind "the user count means people actually here" (§7.1): sit
/// silent for ten minutes in a room that is otherwise talking, and you are the
/// one who stops counting as present, not the room that is punished for you.
pub(crate) const USER_IDLE_MESSAGE_TIMEOUT: Duration = Duration::from_secs(600);

/// WebSocket close code for "you were disconnected for being quiet".
///
/// In the 4000-4999 range, which the protocol reserves for the application.
/// The code is the whole point: without it the client cannot tell an idle
/// eviction from a dropped connection, and its reconnect logic — correctly,
/// for a dropped connection — immediately reconnects.
///
/// That is what stopped rooms ever fading. Evicting the user for idleness set
/// `has_connected_users` to false for the instant it took the browser to come
/// back, so the room never spent `EMPTY_ROOM_CLEANUP_DELAY` empty and was never
/// deleted. One tab left open on a room kept it alive for the life of the
/// process, and the scarcity that is the entire product (§7) quietly stopped
/// happening.
///
/// `index.html`/`client.js` must use the same number; constraint #12, pinned by
/// `client_and_server_agree_on_the_idle_close_code`.
pub(crate) const IDLE_CLOSE_CODE: u16 = 4001;

/// WebSocket close code for "a newer connection under your identity took your
/// place".
///
/// A second connection with the same identity cookie — typically a second tab
/// — does not get refused: `admit_user` treats it as `reclaimed` and the room
/// moves the identity's `ConnectionState` onto the new `connection_id`. Found
/// by reproducing two simultaneous connections under one identity directly:
/// without this code, the *first* tab's socket was never told, so it sat open
/// on the wire indefinitely — still subscribed to the room's broadcast and
/// still refreshing its own idle timer every heartbeat, while
/// `connected_user_count` had already stopped counting it. A silent, ownerless
/// connection the server itself had forgotten it was still holding open.
///
/// Distinct from [`IDLE_CLOSE_CODE`] on purpose: reusing it would tell the
/// superseded tab it was evicted for being quiet, which is not what happened
/// and is not the message a client should show. Same reasoning as that
/// code's own doc comment — the number is the whole mechanism, and the client
/// must not treat this as an ordinary dropped connection either, or the
/// superseded tab immediately reconnects and steals the identity straight
/// back, which is the same flapping this exists to stop.
///
/// `index.html`/`client.js` must use the same number; pinned by its entry in
/// `frontend_parity.rs`'s `PARITY` table, checked by
/// `every_mirrored_constant_matches_its_source_of_truth`.
pub(crate) const SUPERSEDED_CLOSE_CODE: u16 = 4002;

/// `main` is never deleted, so it fades instead: once idle this long its
/// history is trimmed to [`MAIN_ROOM_FADE_KEEP`].
///
/// Thirty minutes, not ten — `main` gets the patience the hearth deserves.
/// This is the one timer that applies to people who are *present and silent*,
/// not merely absent: `cleanup_rooms` fades `main` on room-wide idle
/// regardless of who is still connected (constraint #3), so it is the one
/// mechanic a viewer can genuinely watch happen to them. See §7.4.
pub(crate) const MAIN_ROOM_FADE_IDLE: Duration = Duration::from_secs(1800);

/// How much of `main`'s history survives a fade. 75, not 50 — thirty minutes
/// of patience earns a real surviving thread, not a near-total wipe.
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
/// Sized against the container this deploys to, not chosen in isolation: the
/// production container is Cloudflare's 256 MiB `lite` instance type
/// (`cloudflare/wrangler.jsonc`), and this is the *tracked* bytes only — the
/// Rust binary, its runtime, the OS, and every connection's own buffers are
/// on top of it. 150 MB leaves comfortable headroom in that box; the
/// straight-line 400 MB this used to be was never checked against an actual
/// deployment target and would have left under 60 MB for everything else.
pub(crate) const MAX_TOTAL_ROOMS_MEMORY: usize = 150_000_000;

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
