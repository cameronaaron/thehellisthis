//! Everything that turns untrusted input into something the server will store.
//!
//! Two rules hold throughout: reject before allocating, and never store a
//! string that has not been through [`render_message_html`].

use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::extract::ConnectInfo;
use comrak::{Options as ComrakOptions, markdown_to_html};
use http::HeaderMap;
use regex::Regex;

use crate::config::{MAX_MESSAGE_LEN, ROOM_NAME_REGEX};
use crate::error::ChatError;

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

/// The client's address, preferring proxy headers.
///
/// Cloudflare terminates TLS in front of the container, so the socket address
/// is the proxy for every request; `X-Forwarded-For` is the only place the real
/// client appears. It is attacker-controlled when the server is reached
/// directly, which is why per-IP limits are a courtesy bound and the
/// per-connection and global ceilings are the real protection.
pub fn extract_client_ip(
    headers: &HeaderMap,
    conn_info: Option<&ConnectInfo<SocketAddr>>,
) -> Option<String> {
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
