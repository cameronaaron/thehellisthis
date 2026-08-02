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

pub use text::{
    matches_room_name_shape, render_message_html, sanitize_reply, validate_and_render_message,
    validate_input,
};
