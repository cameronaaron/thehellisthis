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

/// Compiled once on first use (`LazyLock`, std since 1.80, replaced the
/// `lazy_static!` dependency).
static ROOM_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(ROOM_NAME_REGEX).expect("ROOM_NAME_REGEX must be a valid regex"));

/// Whether a room name has the permitted shape. Length is checked separately,
/// so the page handler can tell a visitor "too short" from "bad characters".
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

/// Validates and renders a message in one pass: the raw shape checks, then
/// [`render_message_html`] once, whose output both decides whether anything is
/// there and is what gets stored and broadcast.
///
/// This was two `ammonia::clean` passes over two representations, and they
/// disagreed: `<img src=x onerror=alert(1)>` or `<div></div>` survived the raw
/// pass (ammonia keeps some tags, minus dangerous attributes) but rendered to
/// nothing, because Markdown's safe default omits raw HTML first. A message
/// judged non-empty was broadcast as a blank bubble (§1.4d).
pub fn validate_and_render_message(text: &str) -> Result<String, ChatError> {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return Err(ChatError::InvalidMessage("Message cannot be empty".into()));
    }
    if trimmed.len() > MAX_MESSAGE_LEN {
        return Err(ChatError::InvalidMessage("Message too long".into()));
    }

    let html = render_message_html(trimmed);

    if html.trim().is_empty() {
        return Err(ChatError::InvalidMessage(
            "Message cannot be empty after sanitization".into(),
        ));
    }

    Ok(html)
}

/// Truncates to at most `max_bytes` without splitting a UTF-8 character.
/// `String::truncate` panics off a boundary, and these strings are
/// attacker-supplied: that would be a remotely triggerable panic.
fn truncate_on_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }

    // No `end > 0` guard: index 0 is always a char boundary, so the walk
    // stops there at the latest, and mutation testing found the guard
    // unkillable (§6.6d). At most three steps; UTF-8 characters are at
    // most four bytes.
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Validates and sanitises a message's quoted-reply block.
///
/// Every field is client-supplied and was once stored and rebroadcast as
/// received: unbounded, unsanitised, with a `message_id` that needn't be one.
/// It wasn't XSS only because this client escapes these fields, a property of
/// one client, not the server.
///
/// `None` when the reply can't refer to a real message; the message is then
/// delivered without its quote rather than rejected.
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

/// Renders user Markdown to the HTML that is stored and broadcast.
///
/// Sanitising follows rendering: ammonia runs on the generated HTML, the only
/// form a browser sees; sanitising Markdown first would mangle syntax and leave
/// the renderer's output unchecked.
///
/// The result is also bounded, which the input cap isn't (`[a](b)` repeated to
/// [`MAX_MESSAGE_LEN`] renders to 57 KB). Past [`MAX_RENDERED_MESSAGE_LEN`] the
/// message goes out as escaped plain text rather than being rejected, and that
/// fallback can't overflow: escaping expands by at most 5x, where the ceiling
/// is set.
pub fn render_message_html(text: &str) -> String {
    let rendered = markdown_to_html(text, &ComrakOptions::default());
    let clean = ammonia::clean(&rendered);

    if clean.len() <= MAX_RENDERED_MESSAGE_LEN {
        return clean;
    }

    ammonia::clean_text(text)
}
