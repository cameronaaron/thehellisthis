//! Image attachments: base64 decoding, magic-byte sniffing, and the checks
//! that decide whether a payload is stored as the image it claims to be.

use crate::config::{ALLOWED_ATTACHMENT_MIMES, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_DIMENSION};
use crate::error::ChatError;
use crate::protocol::Attachment;

/// Decodes the first `want` bytes of a base64 payload.
///
/// Only the prefix, because the only question asked of the bytes is "what
/// format is this really" — decoding a 128 KB payload to read twelve magic
/// bytes would be work done per message, on the message path, to reach an
/// answer that lives in the first dozen characters.
///
/// Returns `None` if the payload is not valid standard base64, which is itself
/// part of the check: a payload that will not decode is not an image.
pub(crate) fn decode_base64_prefix(data: &str, want: usize) -> Option<Vec<u8>> {
    const fn sextet(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity(want);
    let mut acc: u32 = 0;
    let mut bits = 0u32;

    for &c in data.as_bytes() {
        if c == b'=' {
            break;
        }
        let value = sextet(c)?;
        acc = (acc << 6) | u32::from(value);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            if out.len() == want {
                return Some(out);
            }
        }
    }

    Some(out)
}

/// The image format a payload actually is, from its magic bytes.
///
/// The declared MIME type is a claim by the client and is not evidence of
/// anything. Checking the bytes is what makes the allow-list mean something:
/// without it, "this is a `image/png`" is a sentence the sender wrote.
pub(crate) fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    // RIFF....WEBP — the four size bytes in between are not part of the tag.
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Accepts or rejects an image attachment.
///
/// Four things have to hold, and the order matters — each is cheaper than the
/// one after it:
///
/// 1. The payload is within [`MAX_ATTACHMENT_BYTES`]. Checked first, so an
///    oversized payload is refused before anything walks it.
/// 2. The declared type is on [`ALLOWED_ATTACHMENT_MIMES`]. Notably **not**
///    `image/svg+xml`: an SVG is a document that can carry script.
/// 3. The payload decodes as base64 and its *own magic bytes* say it is that
///    same type. A client declaring `image/png` over a payload that is really
///    something else gets nothing.
/// 4. The dimensions are plausible. They are display hints, so being wrong
///    costs layout rather than safety — but unbounded numbers reach the client
///    as a CSS size.
///
/// Returns the attachment rebuilt from *validated* parts, never the one that
/// arrived: the stored `mime` is the sniffed type, not the declared one, so
/// what the client renders is what the bytes actually are.
pub fn sanitize_attachment(attachment: Attachment) -> Result<Attachment, ChatError> {
    if attachment.data.is_empty() {
        return Err(ChatError::InvalidMessage("Attachment is empty".into()));
    }
    if attachment.data.len() > MAX_ATTACHMENT_BYTES {
        return Err(ChatError::InvalidMessage("Attachment too large".into()));
    }
    if !ALLOWED_ATTACHMENT_MIMES.contains(&attachment.mime.as_str()) {
        return Err(ChatError::InvalidMessage(
            "Attachment type not allowed".into(),
        ));
    }

    let prefix = decode_base64_prefix(&attachment.data, 12)
        .ok_or_else(|| ChatError::InvalidMessage("Attachment is not valid base64".into()))?;

    let sniffed = sniff_image_mime(&prefix)
        .ok_or_else(|| ChatError::InvalidMessage("Attachment is not a known image".into()))?;

    if sniffed != attachment.mime {
        return Err(ChatError::InvalidMessage(
            "Attachment contents do not match its type".into(),
        ));
    }

    if attachment.width == 0
        || attachment.height == 0
        || attachment.width > MAX_ATTACHMENT_DIMENSION
        || attachment.height > MAX_ATTACHMENT_DIMENSION
    {
        return Err(ChatError::InvalidMessage(
            "Attachment dimensions are out of range".into(),
        ));
    }

    Ok(Attachment {
        // The sniffed type, not the declared one.
        mime: sniffed.to_string(),
        data: attachment.data,
        width: attachment.width,
        height: attachment.height,
        faded: false,
    })
}
