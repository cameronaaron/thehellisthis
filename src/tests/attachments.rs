//! Image attachments: what may be sent, what it costs, and how it fades.

use super::*;

#[tokio::test]
async fn test_main_room_fade_thresholds_correct() {
    // Verify the constants are set for 10-minute timeout
    assert_eq!(
        EMPTY_ROOM_CLEANUP_DELAY.as_secs(),
        600,
        "Room should die at 10 minutes"
    );
    assert_eq!(
        ROOM_CLEANUP_INTERVAL.as_secs(),
        60,
        "Cleanup should run every 1 minute"
    );
}

#[tokio::test]
async fn test_main_room_message_fade_logic() {
    // Verify that the main room fade thresholds make sense for UX:
    // - 10min idle: trim to 50 messages (gentle fade at death)
    // The logic in cleanup_rooms uses this threshold
    let ten_minutes = EMPTY_ROOM_CLEANUP_DELAY;

    assert_eq!(
        ten_minutes.as_secs(),
        600,
        "Main room fades after 10 minutes"
    );
}

/// The interactive fade indicator's thresholds are honest fractions of the
/// real grace period, not independently invented numbers.
///
/// This used to hardcode 10/30/45 seconds under the belief that a room died
/// at 60 seconds — a comment in this very test said so, twice. It does not:
/// `EMPTY_ROOM_CLEANUP_DELAY` and `MAIN_ROOM_FADE_IDLE` are both 600 seconds.
/// The warning toast ("Room fading soon... say something!") fired at 30
/// seconds idle and the room read as visually dying at 45 — five percent of
/// the way into a ten-minute grace period — every single time a conversation
/// paused to think. §7.3 exists for exactly this: the UI must not lie about
/// the mechanics, and this was lying by a factor of twenty.
///
/// The three inequality assertions this test used to make — `30 < 600`, `45 <
/// 600`, `600 - 45 >= 15` — were all trivially true regardless of how early
/// the warning actually fired, so none of them would have failed even at
/// those wrong values. A weak assertion is not a check (§6.4). This instead
/// asserts the real relationship: the client's grace-period constant equals
/// the server's, and the warning/critical thresholds are late fractions of
/// it — computed from that one constant, not chosen independently.
#[tokio::test]
async fn the_fade_indicator_is_honest_about_when_the_room_actually_dies() {
    assert_eq!(
        EMPTY_ROOM_CLEANUP_DELAY, MAIN_ROOM_FADE_IDLE,
        "the client shows one set of thresholds for both a room's deletion \
         and main's fade; they must be the same duration or one of them is \
         being told the wrong story"
    );

    assert!(
        SHIPPED_CLIENT.contains(&format!(
            "ROOM_GRACE_PERIOD_SECONDS = {}",
            EMPTY_ROOM_CLEANUP_DELAY.as_secs()
        )),
        "the client's grace-period constant must equal the backend's, or \
         every fraction computed from it is a fraction of the wrong number"
    );

    assert!(
        SHIPPED_CLIENT.contains("ROOM_GRACE_PERIOD_SECONDS * 0.7"),
        "the warning threshold should be a late fraction of the real grace \
         period, not an early fixed number"
    );
    assert!(
        SHIPPED_CLIENT.contains("ROOM_GRACE_PERIOD_SECONDS * 0.9"),
        "the critical threshold should be a later fraction still, close to \
         when the room actually dies"
    );
}

/// The declared type is a claim; the bytes are the evidence.
///
/// Without sniffing, "this is an `image/png`" is a sentence the sender wrote.
/// The stored MIME is the one the payload actually is, so what a browser is
/// asked to decode is what really arrived.
#[test]
fn an_attachment_must_be_the_image_type_it_claims_to_be() {
    let honest = sanitize_attachment(png_attachment()).expect("a real PNG is accepted");
    assert_eq!(honest.mime, "image/png");

    // A GIF payload wearing a PNG label.
    let liar = Attachment {
        mime: "image/png".to_string(),
        data: TINY_GIF.to_string(),
        ..png_attachment()
    };
    assert!(
        sanitize_attachment(liar).is_err(),
        "a payload that is not the declared type must be refused"
    );

    // Not an image at all.
    let text = Attachment {
        data: "aGVsbG8gd29ybGQhIGhlbGxvIHdvcmxkIQ==".to_string(),
        ..png_attachment()
    };
    assert!(
        sanitize_attachment(text).is_err(),
        "a payload with no image magic bytes must be refused"
    );

    // Not even base64.
    let junk = Attachment {
        data: "!!!! not base64 !!!!".to_string(),
        ..png_attachment()
    };
    assert!(
        sanitize_attachment(junk).is_err(),
        "a payload that will not decode must be refused"
    );
}

/// SVG is a document, not a picture, and must never be an allowed attachment.
///
/// An SVG can carry `<script>`. Rendering one from a `data:` URL in an `<img>`
/// does not execute it in current browsers, but that is a property of the
/// element it happens to be placed in — one refactor to an `<object>`, an
/// `<iframe>` or a CSS `url()` and it is script execution on a server whose
/// whole job is turning user input into markup. The allow-list is the defence,
/// so this pins the hole shut rather than trusting the surrounding code.
#[test]
fn svg_is_not_an_allowed_attachment_type() {
    assert!(
        !ALLOWED_ATTACHMENT_MIMES.contains(&"image/svg+xml"),
        "SVG must never be attachable: it is a document that can carry script"
    );

    let svg = Attachment {
        mime: "image/svg+xml".to_string(),
        // A perfectly well-formed SVG, base64-encoded.
        data: "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPjxzY3JpcHQ+YWxlcnQoMSk8L3NjcmlwdD48L3N2Zz4=".to_string(),
        width: 10,
        height: 10,
        faded: false,
    };
    assert!(
        sanitize_attachment(svg).is_err(),
        "an SVG attachment must be refused whatever its contents"
    );
}

/// §3 — an attachment is bounded before anything walks it.
#[test]
fn an_attachment_is_bounded_in_bytes_and_in_pixels() {
    let huge = Attachment {
        data: "A".repeat(MAX_ATTACHMENT_BYTES + 1),
        ..png_attachment()
    };
    assert!(
        sanitize_attachment(huge).is_err(),
        "a payload over the byte ceiling must be refused"
    );

    for (w, h) in [
        (0, 10),
        (10, 0),
        (MAX_ATTACHMENT_DIMENSION + 1, 10),
        (10, MAX_ATTACHMENT_DIMENSION + 1),
    ] {
        let bad = Attachment {
            width: w,
            height: h,
            ..png_attachment()
        };
        assert!(
            sanitize_attachment(bad).is_err(),
            "dimensions {w}x{h} must be refused"
        );
    }
}

/// §1.1/§3 — one room's pictures cannot spend the whole server's memory.
///
/// Attachments are two orders of magnitude larger than sentences, so a room
/// full of them would take a share of the process-wide ceiling that every other
/// room then could not have. Past the room's budget the oldest *payloads* go
/// and their messages stay, which is the §7 fade aimed at the most expensive
/// thing in the room.
#[tokio::test]
async fn a_rooms_oldest_images_fade_once_it_is_over_its_attachment_budget() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    // Each attachment is a big chunk of the room budget, so a handful crosses it.
    let chunk = MAX_ATTACHMENT_BYTES;
    let needed = (MAX_ROOM_ATTACHMENT_BYTES / chunk) + 2;

    for i in 0..needed {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("picture {i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: Some(Attachment {
                    mime: "image/png".to_string(),
                    data: "A".repeat(chunk),
                    width: 10,
                    height: 10,
                    faded: false,
                }),
            },
            &tracker,
        );
    }

    assert!(
        room.attachment_bytes <= MAX_ROOM_ATTACHMENT_BYTES,
        "a room must stay inside its attachment budget: {} > {MAX_ROOM_ATTACHMENT_BYTES}",
        room.attachment_bytes
    );

    // Every message survives; only the oldest pictures went.
    assert_eq!(room.chat_history.len(), needed, "no message may be deleted");
    assert!(
        room.chat_history[0]
            .attachment
            .as_ref()
            .is_some_and(|a| a.faded && a.data.is_empty()),
        "the oldest image should have faded"
    );
    assert!(
        room.chat_history[needed - 1]
            .attachment
            .as_ref()
            .is_some_and(|a| !a.faded && !a.data.is_empty()),
        "the newest image should still be there"
    );
}

/// An image with no caption is a message; an empty message still is not.
#[tokio::test]
async fn an_image_may_be_sent_without_any_text() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        rooms.insert("photo-room".to_string(), room);
    }

    apply_client_event(
        &state,
        "photo-room",
        "u1",
        "otter",
        ClientEvent::Message {
            text: String::new(),
            reply_to: None,
            attachment: Some(png_attachment()),
        },
    )
    .await;

    // A second, identical-caption image is a second picture, not a stutter.
    apply_client_event(
        &state,
        "photo-room",
        "u1",
        "otter",
        ClientEvent::Message {
            text: String::new(),
            reply_to: None,
            attachment: Some(png_attachment()),
        },
    )
    .await;

    // Text-only and empty is still nothing.
    apply_client_event(
        &state,
        "photo-room",
        "u1",
        "otter",
        ClientEvent::Message {
            text: "   ".to_string(),
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    let history = &rooms.get("photo-room").unwrap().chat_history;
    assert_eq!(
        history.len(),
        2,
        "two captionless images should both arrive, and an empty message should not"
    );
    assert!(history.iter().all(|m| m.attachment.is_some()));
}

// ========== THE SHIPPED CLIENT: EMOJI, IMAGES, REACTIONS ==========

/// §10.3 — an image reserves its space before it decodes.
///
/// The server sends each attachment's dimensions for exactly one reason: so the
/// client can size the box before a byte of the image arrives. Without it every
/// picture shoves the conversation downward as it loads, which is the same
/// layout-shift failure the empty-chat placeholder had.
#[test]
fn images_reserve_their_space_before_they_load() {
    assert!(
        EMBEDDED_JS.contains("aspectRatio"),
        "the client must set an aspect-ratio from the server's dimensions"
    );
    assert!(
        EMBEDDED_JS.contains("attachment.width") && EMBEDDED_JS.contains("attachment.height"),
        "the reserved space must come from the attachment's own dimensions"
    );
}

/// Images are re-encoded in a canvas, which is also what strips EXIF.
///
/// The downscale exists because the server's ceiling is small. Dropping the
/// metadata is a side effect, but it is the one that matters most on a server
/// whose premise is that you get an animal name instead of an account: a
/// phone photograph carries the coordinates it was taken at, and sending that
/// to a room of strangers is a disclosure nobody intended to make (§5.6).
#[test]
fn images_are_re_encoded_rather_than_sent_as_picked() {
    assert!(
        EMBEDDED_JS.contains("createElement('canvas')"),
        "the client must re-encode through a canvas, not send the original file"
    );
    assert!(
        !EMBEDDED_JS.contains("readAsDataURL"),
        "reading the picked file straight to a data URL would ship the original \
         bytes, EXIF and all"
    );
}

/// WebP is sniffed from `RIFF....WEBP`, not from the four size bytes between.
///
/// The client encodes to WebP first because it is roughly a third smaller than
/// JPEG at the same quality — which is the difference between a photo fitting
/// under the ceiling and being refused — so this is the format most attachments
/// actually arrive as.
#[test]
fn a_webp_payload_is_recognised_by_its_riff_tag() {
    // "RIFF" + 4 size bytes + "WEBP" + "VP8 ", base64-encoded.
    let webp = Attachment {
        mime: "image/webp".to_string(),
        data: "UklGRiQAAABXRUJQVlA4IBgAAAAwAQCdASoBAAEAAQAcJaQAA3AA/v3AgAA=".to_string(),
        width: 1,
        height: 1,
        faded: false,
    };
    let clean = sanitize_attachment(webp).expect("a real WebP is accepted");
    assert_eq!(clean.mime, "image/webp");

    // The same tag with the wrong four bytes where WEBP should be is not one.
    let not_webp = Attachment {
        mime: "image/webp".to_string(),
        data: "UklGRiQAAABXQVZFZm10IBAAAAABAAEAgD4AAAB9AAACABAA".to_string(),
        width: 1,
        height: 1,
        faded: false,
    };
    assert!(
        sanitize_attachment(not_webp).is_err(),
        "a RIFF container that is not WEBP must be refused"
    );
}

/// The base64 decoder accepts the whole standard alphabet.
///
/// `+` and `/` are the two characters a hand-rolled decoder is most likely to
/// forget, and a payload containing either would then be refused as "not valid
/// base64" — an image that fails to send for no reason the user can see.
#[test]
fn the_attachment_decoder_accepts_the_whole_base64_alphabet() {
    // A PNG whose encoding exercises '+' and '/' as well as the letter and
    // digit ranges, plus '=' padding.
    let png = Attachment {
        mime: "image/png".to_string(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFUlEQVR42mP8z8BQz0AEYBxVSF+FABJADveWkH6oAAAAAElFTkSuQmCC".to_string(),
        width: 10,
        height: 10,
        faded: false,
    };
    assert!(
        sanitize_attachment(png).is_ok(),
        "a payload using '+' and '/' must still decode"
    );
}

/// A rejected attachment takes its whole message with it.
///
/// Delivering the caption without the picture would be worse than delivering
/// nothing: the sender would see their words arrive and assume the image did
/// too.
#[tokio::test]
async fn a_message_whose_attachment_is_refused_is_not_delivered() {
    let state = Arc::new(AppState::new());
    {
        let mut rooms = state.rooms.write().await;
        let mut room = create_room();
        room.users.insert(
            "u1".to_string(),
            connected_user("u1", "otter", "c1", Instant::now()),
        );
        rooms.insert("bad-image".to_string(), room);
    }

    apply_client_event(
        &state,
        "bad-image",
        "u1",
        "otter",
        ClientEvent::Message {
            text: "look at this".to_string(),
            reply_to: None,
            attachment: Some(Attachment {
                mime: "image/png".to_string(),
                data: "bm90IGFuIGltYWdlIGF0IGFsbA==".to_string(),
                width: 10,
                height: 10,
                faded: false,
            }),
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    assert!(
        rooms.get("bad-image").unwrap().chat_history.is_empty(),
        "a message must not arrive without the image it was sent with"
    );
}

/// Fading is idempotent and stops as soon as the room is back under budget.
#[tokio::test]
async fn fading_skips_images_that_already_faded() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    let chunk = MAX_ATTACHMENT_BYTES;
    let needed = (MAX_ROOM_ATTACHMENT_BYTES / chunk) + 3;
    for i in 0..needed {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("p{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: Some(Attachment {
                    mime: "image/png".to_string(),
                    data: "A".repeat(chunk),
                    width: 10,
                    height: 10,
                    faded: false,
                }),
            },
            &tracker,
        );
    }

    let faded_after_first = room
        .chat_history
        .iter()
        .filter(|m| m.attachment.as_ref().is_some_and(|a| a.faded))
        .count();

    // A text-only message cannot push the room over its picture budget, so a
    // second pass must find nothing left to do.
    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "just words".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    let faded_after_second = room
        .chat_history
        .iter()
        .filter(|m| m.attachment.as_ref().is_some_and(|a| a.faded))
        .count();

    assert_eq!(
        faded_after_first, faded_after_second,
        "an already-faded image must not be faded again"
    );
    assert!(room.attachment_bytes <= MAX_ROOM_ATTACHMENT_BYTES);
}

/// Fading is a no-op when there is nothing left to fade.
///
/// Reachable when the byte total says the room is over budget but every
/// attachment has already been emptied — an accounting drift rather than a
/// normal state, which is exactly when a loop that assumed it would find work
/// would spin or subtract something it did not free.
#[tokio::test]
async fn fading_with_nothing_left_to_fade_changes_nothing() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "already gone".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: Some(Attachment {
                mime: "image/png".to_string(),
                data: String::new(),
                width: 10,
                height: 10,
                faded: true,
            }),
        },
        &tracker,
    );

    // Claim the room is over budget with nothing un-faded to reclaim.
    room.attachment_bytes = MAX_ROOM_ATTACHMENT_BYTES + 1;
    let before = room.total_memory_bytes.load(Ordering::SeqCst);

    room.add_message(
        OutgoingMessage {
            message_id: Uuid::new_v4(),
            user_id: "u".to_string(),
            animal_name: "otter".to_string(),
            text: "just words".to_string(),
            timestamp: "1".to_string(),
            reply_to: None,
            attachment: None,
        },
        &tracker,
    );

    assert!(
        room.total_memory_bytes.load(Ordering::SeqCst) > before,
        "the new message should have been accounted for, not cancelled out by \
         a fade that freed nothing"
    );
    assert_eq!(room.chat_history.len(), 2);
}

/// The base64 prefix decoder, over its whole contract.
///
/// Tested directly rather than through an attachment because the interesting
/// inputs cannot be reached that way: an image's first twelve bytes are its
/// magic number, so `+` and `/` — the two characters a hand-rolled decoder is
/// most likely to forget — never appear that early in a PNG or a GIF. A decoder
/// that silently mishandled them would reject real images for no reason the
/// user could see, and only for *some* images.
#[test]
fn the_base64_prefix_decoder_handles_its_whole_alphabet() {
    use crate::validation::decode_base64_prefix;

    // Every sextet value 0-63 appears across this alphabet, including + and /.
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let decoded = decode_base64_prefix(alphabet, 48).expect("the standard alphabet must decode");
    assert_eq!(decoded.len(), 48, "64 base64 characters carry 48 bytes");

    // `+` and `/` specifically: "+/+/" decodes to 0xFB 0xEF 0xBE.
    assert_eq!(
        decode_base64_prefix("+/+/", 3),
        Some(vec![0xFB, 0xFF, 0xBF]),
        "'+' is 62 and '/' is 63"
    );

    // Padding ends the payload rather than decoding as data.
    assert_eq!(decode_base64_prefix("QQ==", 12), Some(vec![0x41]));

    // Running out before `want` bytes yields what there was, not a failure —
    // a short payload is not a malformed one, it just is not an image.
    let short = decode_base64_prefix("QUJD", 12).expect("valid base64");
    assert_eq!(short, b"ABC");

    // Anything outside the alphabet is a refusal.
    for bad in ["!!!!", "abc def", "AB*D", "café"] {
        assert_eq!(
            decode_base64_prefix(bad, 12),
            None,
            "{bad:?} is not base64 and must be refused"
        );
    }

    // Exactly `want` bytes stops early rather than walking the whole payload —
    // the reason this reads a prefix at all.
    let long = "A".repeat(100_000);
    assert_eq!(decode_base64_prefix(&long, 4).map(|v| v.len()), Some(4));
}

/// Every format on the allow-list is recognised from its own bytes, and
/// near-misses are not.
#[test]
fn image_sniffing_recognises_each_allowed_format() {
    use crate::validation::sniff_image_mime;

    assert_eq!(
        sniff_image_mime(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        Some("image/png")
    );
    assert_eq!(
        sniff_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]),
        Some("image/jpeg")
    );
    assert_eq!(sniff_image_mime(b"GIF87a...."), Some("image/gif"));
    assert_eq!(sniff_image_mime(b"GIF89a...."), Some("image/gif"));
    assert_eq!(
        sniff_image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
        Some("image/webp")
    );

    // Every format on the allow-list must actually be sniffable, or it is on a
    // list of things that can never be accepted.
    let sniffable = ["image/png", "image/jpeg", "image/gif", "image/webp"];
    for mime in ALLOWED_ATTACHMENT_MIMES {
        assert!(
            sniffable.contains(mime),
            "{mime} is allowed but `sniff_image_mime` can never return it, so \
             no payload of that type could ever be accepted"
        );
    }

    for not_an_image in [
        &b"RIFF\0\0\0\0WAVEfmt "[..], // RIFF, but not WEBP
        &b"\x89PNGxxxx"[..],          // PNG magic truncated
        &b"GIF88a"[..],               // not a real GIF version
        &b"<svg xmlns="[..],          // a document
        &b"hello world!"[..],
        &b""[..],
        &b"RIFF"[..], // too short to hold the WEBP tag
    ] {
        assert_eq!(
            sniff_image_mime(not_an_image),
            None,
            "{not_an_image:?} must not be recognised as an image"
        );
    }
}

/// An attachment with no payload is refused before anything walks it.
#[test]
fn an_empty_attachment_is_refused() {
    let empty = Attachment {
        mime: "image/png".to_string(),
        data: String::new(),
        width: 10,
        height: 10,
        faded: false,
    };
    assert!(sanitize_attachment(empty).is_err());
}

/// A JPEG is accepted, and its stored type comes from its bytes.
#[test]
fn a_jpeg_attachment_is_accepted() {
    // A minimal JFIF header, base64-encoded.
    let jpeg = Attachment {
        mime: "image/jpeg".to_string(),
        data: "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q==".to_string(),
        width: 1,
        height: 1,
        faded: false,
    };
    let clean = sanitize_attachment(jpeg).expect("a real JPEG is accepted");
    assert_eq!(clean.mime, "image/jpeg");
}

// ========== META-CONTRACTS ==========
//
// Ported from the contract suite on cameronaaron.com, adapted to a Rust
// project. The idea those tests encode is that a standards document's
// authority rests on one claim — every rule is enforced by a test — and that
// claim is itself something that can rot silently. So it gets a contract too.

/// Pruning gives the room's attachment budget back, exactly.
///
/// `release_attachment_bytes` could be replaced with an empty body and the
/// whole suite still passed: nothing asserted that removing messages returns
/// their picture bytes. Left unreturned, the running total only ever climbs,
/// and a room that had once been busy would fade every new image immediately
/// while holding almost none.
#[tokio::test]
async fn pruning_returns_the_attachment_bytes_it_removed() {
    let tracker = MemoryTracker::new();
    let mut room = create_room();

    let payload = 4_096;
    for i in 0..6 {
        room.add_message(
            OutgoingMessage {
                message_id: Uuid::new_v4(),
                user_id: "u".to_string(),
                animal_name: "otter".to_string(),
                text: format!("p{i}"),
                timestamp: "1".to_string(),
                reply_to: None,
                attachment: Some(Attachment {
                    mime: "image/png".to_string(), // 9 bytes of mime
                    data: "A".repeat(payload),
                    width: 1,
                    height: 1,
                    faded: false,
                }),
            },
            &tracker,
        );
    }

    let each = payload + "image/png".len();
    assert_eq!(
        room.attachment_bytes,
        each * 6,
        "six images cost exactly six payloads plus six mime strings"
    );

    // Drop the two oldest.
    room.retain_newest(4, &tracker);
    assert_eq!(
        room.attachment_bytes,
        each * 4,
        "trimming two messages must return exactly two images' worth"
    );

    // And pruning under memory pressure does the same.
    room.prune_old_messages(usize::MAX, &tracker);
    assert_eq!(
        room.attachment_bytes, 0,
        "removing every message must return every byte"
    );
}
