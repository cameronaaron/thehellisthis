//! Room names and message bodies: shape checks, the Markdown-to-HTML
//! pipeline, and the quoted-reply block that rides along with a message.

use std::sync::LazyLock;

use comrak::{Options as ComrakOptions, markdown_to_html};
use regex::Regex;
use uuid::Uuid;

use crate::config::{
    MAX_MESSAGE_LEN, MAX_RENDERED_MESSAGE_LEN, MAX_REPLY_AUTHOR_LEN, MAX_REPLY_PREVIEW_LEN,
    ROOM_NAME_REGEX,
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

    // No `end > 0` guard: index 0 is a char boundary of every string, including
    // the empty one, so the walk always terminates there at the latest. The
    // guard was there, and mutation testing found it unkillable — a condition
    // no input can make false is not a safety net, it is a claim nothing checks
    // (§6.6d). At most three steps are taken; UTF-8 characters are four bytes.
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
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
/// The rendered form is also *bounded*, which the input cap alone does not do.
/// Markdown expands: `[a](b)` repeated to [`MAX_MESSAGE_LEN`] renders to 57 KB,
/// and that is what would be stored in history and pushed to every socket in
/// the room. Past [`MAX_RENDERED_MESSAGE_LEN`] the message is delivered as
/// escaped plain text rather than rejected — the user still said something, it
/// just does not get to be formatted. That fallback cannot itself overflow:
/// escaping expands by at most 5x, which is exactly where the ceiling is set.
pub fn render_message_html(text: &str) -> String {
    let rendered = markdown_to_html(text, &ComrakOptions::default());
    let clean = ammonia::clean(&rendered);

    if clean.len() <= MAX_RENDERED_MESSAGE_LEN {
        return clean;
    }

    ammonia::clean_text(text)
}
