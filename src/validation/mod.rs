//! Everything that turns untrusted input into something the server will store.
//!
//! Two rules hold throughout: reject before allocating, and never store a
//! string that has not been through [`render_message_html`].
//!
//! Split by what is being validated: [`text`] is room names and message
//! bodies, [`attachments`] is image payloads, [`client_address`] is where a
//! connection actually came from.

mod attachments;
mod client_address;
mod text;

pub use attachments::sanitize_attachment;
#[cfg(test)]
pub(crate) use attachments::{decode_base64_prefix, sniff_image_mime};

pub use client_address::{extract_client_ip, hash_client_address};

/// Only [`validate_and_render_message`] renders now. The session used to call
/// this directly for an attachment's caption, which was the one path that
/// could hand the renderer an untrimmed `MAX_PAYLOAD_SIZE` — see
/// `session/events.rs`'s blank-caption branch. Still exported to the tests
/// that pin the pipeline itself (constraint #8: sanitise after rendering).
#[cfg(test)]
pub(crate) use text::render_message_html;
pub use text::{
    matches_room_name_shape, sanitize_reply, validate_and_render_message, validate_input,
};
