//! Everything that turns untrusted input into something the server will store.
//!
//! Two rules hold throughout: reject before allocating, and never store a
//! string that has not been through [`render_message_html`].

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::extract::ConnectInfo;
use comrak::{Options as ComrakOptions, markdown_to_html};
use http::HeaderMap;
use regex::Regex;

use uuid::Uuid;

use crate::config::{
    MAX_MESSAGE_LEN, MAX_REPLY_AUTHOR_LEN, MAX_REPLY_PREVIEW_LEN, ROOM_NAME_REGEX,
};
use crate::error::ChatError;
use crate::protocol::ReplyInfo;

/// Compiled once on first use.
///
/// `LazyLock` is std since 1.80 and replaces the `lazy_static!` macro this
/// server used to carry a dependency for.
static ROOM_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(ROOM_NAME_REGEX).expect("ROOM_NAME_REGEX must be a valid regex"));

/// Whether a room name matches the permitted shape.
///
/// Length is checked separately by callers that must report *which* rule the
/// name broke — the page handler distinguishes "too short" from "bad
/// characters" because the visitor typed it.
pub fn matches_room_name_shape(room: &str) -> bool {
    ROOM_REGEX.is_match(room)
}

/// Validates a short identifier-like input (room names on the socket path).
pub fn validate_input(text: &str, max_len: usize) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("Input cannot be empty");
    }
    if text.len() > max_len {
        return Err("Input too long");
    }
    if !text
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Input contains invalid characters");
    }
    Ok(())
}

/// Accepts or rejects a chat message, returning its sanitised plain form.
///
/// The returned string is the *guard* value, not what gets broadcast: a
/// message consisting only of markup sanitises to nothing and must be rejected
/// here, before [`render_message_html`] turns the original Markdown into HTML.
pub fn validate_message(text: &str) -> Result<String, ChatError> {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return Err(ChatError::InvalidMessage("Message cannot be empty".into()));
    }
    if trimmed.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage("Message too long".into()));
    }

    let clean_text = ammonia::clean(trimmed);

    if clean_text.trim().is_empty() {
        return Err(ChatError::InvalidMessage(
            "Message cannot be empty after sanitization".into(),
        ));
    }
    if clean_text.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage(
            "Sanitized message too long".into(),
        ));
    }

    Ok(clean_text)
}

/// Truncates to at most `max_bytes`, never splitting a UTF-8 character.
///
/// `String::truncate` panics on a non-boundary index, and every string here is
/// attacker-supplied — a multi-byte character straddling the limit would be a
/// remotely triggerable panic.
fn truncate_on_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Validates and sanitises the quoted-reply block attached to a message.
///
/// Every field is client-supplied and was previously stored and rebroadcast
/// exactly as received: unbounded in length, never sanitised, and with a
/// `message_id` that did not have to be a message id. The only reason it was
/// not an XSS vector is that the current client happens to escape these fields
/// when rendering them — a property of one client, not of the server.
///
/// Returns `None` when the reply cannot refer to a real message, in which case
/// the message is delivered without its quote rather than rejected.
pub fn sanitize_reply(reply: ReplyInfo) -> Option<ReplyInfo> {
    // A reply that does not point at a message id is not a reply.
    let message_id = Uuid::parse_str(reply.message_id.trim()).ok()?;

    Some(ReplyInfo {
        message_id: message_id.to_string(),
        author_name: truncate_on_char_boundary(
            ammonia::clean_text(reply.author_name.trim()),
            MAX_REPLY_AUTHOR_LEN,
        ),
        preview_text: truncate_on_char_boundary(
            ammonia::clean_text(reply.preview_text.trim()),
            MAX_REPLY_PREVIEW_LEN,
        ),
    })
}

/// Renders user Markdown to the HTML that will be stored and broadcast.
///
/// Sanitising happens *after* rendering, never before: `ammonia` runs on the
/// generated HTML, which is the only representation that reaches a browser.
/// Sanitising the Markdown first would both mangle legitimate syntax and leave
/// whatever the renderer subsequently produced unchecked.
pub fn render_message_html(text: &str) -> String {
    let rendered = markdown_to_html(text, &ComrakOptions::default());
    ammonia::clean(&rendered)
}

/// Keyed hasher for client addresses, seeded once per process.
///
/// `RandomState` is SipHash-1-3 with keys drawn at startup. The keys never
/// leave this process and change on every restart, so the stored digests are
/// not correlatable across restarts or against a precomputed table — which
/// matters, because the IPv4 space is small enough to enumerate against an
/// unkeyed hash.
static ADDRESS_HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);

/// A client address reduced to an opaque, stable-per-process identifier.
///
/// The rate limiter, connection pool and ban list only ever compare addresses
/// for equality — none of them needs to know the actual address. Hashing at the
/// boundary means the raw address exists only as a local in
/// [`extract_client_ip`]'s caller and is never stored, logged, or held in any
/// map: a memory dump of a running server yields no visitor addresses.
///
/// This is a privacy decision, not a security one. Per-IP limits remain a
/// courtesy bound (§5.6) — hashing changes nothing about their strength.
pub fn hash_client_address(ip: &str) -> String {
    format!("{:016x}", ADDRESS_HASHER.hash_one(ip))
}

/// The client's address, preferring proxy headers.
///
/// Cloudflare terminates TLS in front of the container, so the socket address
/// is the proxy for every request and the real client only appears in a header.
///
/// `CF-Connecting-IP` is checked first because that is what actually arrives in
/// production: Cloudflare sets it on every proxied request, and it is the one
/// header the edge will not let a client forge. Checking only
/// `X-Forwarded-For`, as this used to, meant every visitor looked like the same
/// address to the per-IP limits — so those limits were, in effect, global.
///
/// All of these are attacker-controlled if the container is ever reached
/// directly, which is why per-IP limits are a courtesy bound and the
/// per-connection and global ceilings are the real protection.
pub fn extract_client_ip(
    headers: &HeaderMap,
    conn_info: Option<&ConnectInfo<SocketAddr>>,
) -> Option<String> {
    if let Some(cf_ip) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
    {
        let ip = cf_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    // X-Forwarded-For is a list, client first.
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(client_ip) = forwarded.split(',').next()
    {
        let ip = client_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = real_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    conn_info.map(|ci| ci.0.ip().to_string())
}
