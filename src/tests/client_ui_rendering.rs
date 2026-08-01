//! What the shipped page and script actually render: layout contracts, CSS
//! sweeps for dead or duplicated rules, and the frontend/backend parity
//! checks specific to attachments and reactions.

use super::*;

#[tokio::test]
async fn test_reserved_path_robots_txt() {
    let app = Router::new().route("/robots.txt", get(robots_txt_handler));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_message_with_inline_code() {
    let text = "Use `cargo test` to run tests";
    let result = validate_message(text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_robots_txt_endpoint() {
    let app = Router::new().route("/robots.txt", get(robots_txt_handler));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_robots_txt_content() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/robots.txt", get(robots_txt_handler))
        .with_state(app_state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body.contains("User-agent:"));
}

#[tokio::test]
async fn test_message_alignment_with_multiple_users() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/alignment-test", addr);

    // Connect user 1
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();

    // Get user 1's identity
    let mut user1_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            user1_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user1_id.is_empty() {
                break;
            }
        }
    }

    assert!(!user1_id.is_empty(), "User 1 should have a user_id");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect user 2
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Get user 2's identity
    let mut user2_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws2).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            user2_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user2_id.is_empty() {
                break;
            }
        }
    }

    assert!(!user2_id.is_empty(), "User 2 should have a user_id");
    assert_ne!(user1_id, user2_id, "Users should have different user_ids");

    // Drain events
    for _ in 0..5 {
        let _ = timeout(Duration::from_millis(200), recv_json_event(&mut ws1)).await;
        let _ = timeout(Duration::from_millis(200), recv_json_event(&mut ws2)).await;
    }

    // User 1 sends message
    let msg1 = r#"{"type":"Message","text":"From user 1"}"#;
    ws1.send(text_frame(msg1.to_string())).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // User 2 sends message
    let msg2 = r#"{"type":"Message","text":"From user 2"}"#;
    ws2.send(text_frame(msg2.to_string())).await.unwrap();

    // Both users should receive both messages with correct user_ids
    let mut messages_received = 0;
    for _ in 0..4 {
        if let Ok(data) = timeout(Duration::from_secs(2), recv_json_event(&mut ws1)).await
            && data["type"] == "Message"
        {
            let received_user_id = data["message"]["user_id"].as_str().unwrap_or("");
            assert!(
                !received_user_id.is_empty()
                    && (received_user_id == user1_id || received_user_id == user2_id),
                "Message should have valid user_id, got: {}",
                received_user_id
            );
            messages_received += 1;
        }
    }

    assert!(
        messages_received >= 2,
        "Should have received at least 2 messages"
    );

    handle.abort();
}

#[tokio::test]
async fn test_message_contains_user_id_for_alignment() {
    // Outgoing messages MUST contain user_id for client-side alignment
    let msg = OutgoingMessage {
        message_id: uuid::Uuid::new_v4(),
        user_id: "alignment-test-user".to_string(),
        animal_name: "Lion".to_string(),
        text: "<p>Test message</p>".to_string(),
        timestamp: "1234567890".to_string(),
        reply_to: None,
        attachment: None,
    };

    // user_id must be present and non-empty
    assert!(
        !msg.user_id.is_empty(),
        "OutgoingMessage must have user_id for alignment"
    );
    assert_eq!(msg.user_id, "alignment-test-user");

    // Serialize to JSON and verify user_id is included
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("user_id"), "JSON must include user_id field");
    assert!(
        json.contains("alignment-test-user"),
        "JSON must include actual user_id value"
    );
}

#[tokio::test]
async fn test_message_alignment_after_reconnect() {
    let (addr, handle) = start_ws_server().await;
    let ws_url = format!("ws://{}/ws/alignment-reconnect", addr);

    // First connection
    let (mut ws1, _) = connect_async(&ws_url).await.unwrap();

    let mut user_id = String::new();
    for _ in 0..10 {
        let val = recv_json_event(&mut ws1).await;
        if val.get("type") == Some(&JsonValue::String("System".to_string()))
            && let Some(event) = val.get("event")
            && let Some(payload) = extract_system_event(event, "UserJoined")
        {
            user_id = payload
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user_id.is_empty() {
                break;
            }
        }
    }

    // Drain events and send message
    for _ in 0..3 {
        let _ = timeout(Duration::from_millis(100), recv_json_event(&mut ws1)).await;
    }

    ws1.send(text_frame(
        r#"{"type":"Message","text":"Pre-reconnect message"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Wait for message to be stored
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(ws1);

    // Reconnect with new connection
    let (mut ws2, _) = connect_async(&ws_url).await.unwrap();

    // Should receive history with original user_id intact
    let mut found_historical_msg = false;
    for _ in 0..15 {
        if let Ok(val) = timeout(Duration::from_secs(2), recv_json_event(&mut ws2)).await
            && val.get("type") == Some(&JsonValue::String("Message".to_string()))
        {
            let msg_user_id = val["message"]["user_id"].as_str().unwrap_or("");
            let text = val["message"]["text"].as_str().unwrap_or("");
            if text.contains("Pre-reconnect") {
                assert_eq!(
                    msg_user_id, user_id,
                    "Historical message must preserve original user_id for alignment"
                );
                found_historical_msg = true;
                break;
            }
        }
    }

    assert!(
        found_historical_msg,
        "Should receive historical message with preserved user_id"
    );
    handle.abort();
}

#[tokio::test]
async fn test_robots_txt_handler_content() {
    use axum::response::IntoResponse;

    let response = robots_txt_handler().await.into_response();
    assert_eq!(response.status(), StatusCode::OK);

    // Check content type header
    let headers = response.headers();
    assert!(headers.get("content-type").is_some());
}

// Test health_handler returns valid JSON - covers lines 2203-2230

/// The chat column must not change its own alignment when it stops being empty.
///
/// The empty state used to be `#chat:empty { justify-content: center;
/// align-items: center }`. The instant the first history message arrived
/// `:empty` stopped matching and the entire column snapped from centred to
/// top-aligned — a visible reorientation on every page load of a room that had
/// any history. The placeholder is now an absolutely-positioned overlay, which
/// cannot affect the layout of the messages that replace it.
#[test]
fn empty_chat_placeholder_does_not_alter_container_layout() {
    // Scan selectors, not raw substrings: prose in a comment can mention the
    // old rule (this file's own comments do), and a test that a comment can
    // fail is a test that will be silenced rather than fixed.
    let opens_a_rule_on_the_container = SHIPPED_CLIENT
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("#chat:empty") && !line.contains("::") && line.ends_with('{'));

    assert!(
        !opens_a_rule_on_the_container,
        "#chat:empty must not restyle the container itself; \
         use the absolutely-positioned ::before/::after overlay"
    );
    assert!(
        SHIPPED_CLIENT.contains("#chat:empty::before"),
        "the empty-state placeholder must still exist"
    );
}

/// Trimming rendered history must be driven by the DOM, not by a counter.
///
/// The counter this replaced was incremented for system messages too, but those
/// remove themselves after eight seconds without decrementing it. On a busy
/// room the count drifted well above the number of nodes actually present and
/// began deleting live chat messages that were nowhere near the limit —
/// messages silently vanishing from a conversation, with the counter wrong and
/// the page fine.
#[test]
fn rendered_history_is_trimmed_from_the_dom_not_a_counter() {
    assert!(
        !EMBEDDED_JS.contains("this.messageCount"),
        "a hand-maintained message counter drifts; count the DOM instead"
    );
    assert!(
        EMBEDDED_JS.contains("pruneRenderedMessages()"),
        "trimming must be driven by the number of rendered messages"
    );
    assert!(
        EMBEDDED_JS.contains("querySelectorAll('.message')"),
        "the prune must measure the DOM it is trimming"
    );
}

/// The page must not block pinch-zoom.
///
/// `user-scalable=no` / `maximum-scale=1` fails WCAG 1.4.4. The usual reason to
/// set it is stopping iOS zooming when an input is focused, which is handled
/// instead by giving the message input a 16px font size.
#[test]
fn the_page_does_not_block_pinch_zoom() {
    // Inspect the viewport tag itself, not the whole file: the comment above it
    // names the settings it is explaining, and a test a comment can fail is a
    // test that gets silenced rather than fixed.
    let viewport = EMBEDDED_HTML
        .lines()
        .find(|line| line.contains("name=\"viewport\""))
        .expect("the page must declare a viewport");

    assert!(
        !viewport.contains("user-scalable=no"),
        "blocking zoom fails WCAG 1.4.4: {viewport}"
    );
    assert!(
        !viewport.contains("maximum-scale=1"),
        "capping zoom fails WCAG 1.4.4: {viewport}"
    );
    assert!(
        EMBEDDED_HTML.contains("font-size: 16px; /* Prevents iOS zoom */"),
        "the 16px input font is what makes blocking zoom unnecessary"
    );
}

// ========== BOUNDARY CONDITIONS ==========
//
// Written to kill mutants that survived `cargo mutants`: each one flipped a
// comparison at a threshold and no existing test noticed. A limit that is off
// by one at exactly the limit is the kind of bug that only appears under the
// load it was meant to protect against.

/// Every element the client asks for by id exists in the page it ships with.
///
/// The client is one HTML file and one JS file with no build step and no type
/// checker, so a `getElementById` for an id that is not in the markup is not a
/// compile error — it is `null`, and the first method that touches it throws at
/// runtime, usually somewhere far from the typo. Nothing else in the pipeline
/// would notice, which is exactly why this belongs in the gate (§9.1: verify
/// against the artifact, not the source that should have produced it).
#[test]
fn every_element_the_client_looks_up_exists_in_the_page() {
    let mut missing: Vec<&str> = Vec::new();

    for (index, _) in EMBEDDED_JS.match_indices("getElementById('") {
        let rest = &EMBEDDED_JS[index + "getElementById('".len()..];
        let Some(end) = rest.find('\'') else { continue };
        let id = &rest[..end];

        // Ids are quoted in the markup, so this cannot match a prefix of a
        // longer id the way a bare substring search would.
        if !EMBEDDED_HTML.contains(&format!("id=\"{id}\"")) {
            missing.push(id);
        }
    }

    assert!(
        missing.is_empty(),
        "client.js looks up ids that index.html does not define: {missing:?}"
    );
}

/// Constraint #12 — the client re-encodes to the ceiling the server enforces.
///
/// The client shrinks an image until it fits `MAX_ATTACHMENT_BYTES`. If its
/// copy of that number is larger than the server's, every photograph is
/// rejected *after* the user has waited for it to encode; if smaller, images
/// are needlessly degraded. Neither shows up as an error anywhere.
#[test]
fn client_attachment_ceiling_matches_the_server() {
    assert!(
        EMBEDDED_JS.contains(&format!(
            "const MAX_ATTACHMENT_BYTES = {MAX_ATTACHMENT_BYTES};"
        )),
        "client.js must declare MAX_ATTACHMENT_BYTES = {MAX_ATTACHMENT_BYTES} to match config.rs"
    );
}

/// Constraint #12 — every reaction the client offers, the server accepts.
///
/// Compared as sets: the server keeps its roster sorted by code point because
/// `is_reaction_emoji` binary-searches it, and that is not an order to show
/// anybody. A reaction button the server rejects is a button that silently does
/// nothing, which is the drift this catches.
#[test]
fn client_reaction_roster_matches_the_server() {
    let list = EMBEDDED_JS
        .split_once("const REACTION_EMOJI = [")
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(list, _)| list)
        .expect("client.js should declare a REACTION_EMOJI list");

    let client: std::collections::HashSet<&str> = list
        .split(',')
        .map(|entry| entry.trim().trim_matches('\'').trim())
        .filter(|entry| !entry.is_empty())
        .collect();

    let server: std::collections::HashSet<&str> = REACTION_EMOJI.iter().copied().collect();

    assert_eq!(
        client, server,
        "the client's reaction roster must be exactly the server's"
    );

    // The one-click bar is a subset of the same set, so no quick reaction can
    // be one the server refuses.
    let quick = EMBEDDED_JS
        .split_once("const QUICK_REACTIONS = [")
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(list, _)| list)
        .expect("client.js should declare QUICK_REACTIONS");

    for emoji in quick
        .split(',')
        .map(|e| e.trim().trim_matches('\'').trim())
        .filter(|e| !e.is_empty())
    {
        assert!(
            is_reaction_emoji(emoji),
            "quick reaction {emoji:?} is not on the server's roster"
        );
    }
}

/// §5.7 / constraint #13 — the new UI adds no inline handler and no outside
/// origin.
///
/// The emoji picker, the lightbox and the drag-and-drop overlay are all new
/// interactive surfaces, and every one of them is the sort of thing that
/// usually arrives with an `onclick=` or a CDN icon set. Either would quietly
/// undo the CSP that the whole message pipeline leans on.
#[test]
fn the_new_client_surfaces_keep_the_policy_intact() {
    for handler in [
        "onclick=",
        "onload=",
        "onchange=",
        "oninput=",
        "ondrop=",
        "ondragover=",
        "onkeydown=",
    ] {
        assert!(
            !EMBEDDED_HTML.contains(handler),
            "index.html must not carry the inline handler {handler}"
        );
    }

    assert!(
        !EMBEDDED_HTML.contains("<script>"),
        "the page must have no inline script block"
    );

    // The picker is emoji from the system font, not an icon set from a CDN.
    for origin in [
        "cdn.",
        "unpkg.com",
        "googleapis.com",
        "gstatic.com",
        "jsdelivr",
    ] {
        assert!(
            !EMBEDDED_JS.contains(origin),
            "client.js must not reference the third-party origin {origin}"
        );
    }
}

/// No `content:` string is a leftover icon-font ligature.
///
/// The bug above rendered as text because a ligature name *is* text: with the
/// font present it draws a picture, and without it the browser shows the word.
/// Icons are inline SVG here, so any `content` holding a bare identifier-like
/// word is that mistake coming back.
#[test]
fn no_css_content_string_is_an_icon_ligature() {
    let mut suspicious: Vec<String> = Vec::new();

    let css = embedded_html_without_comments();

    for (index, _) in css.match_indices("content: '") {
        let rest = &css[index + "content: '".len()..];
        let Some(end) = rest.find('\'') else { continue };
        let value = &rest[..end];

        // Ligature names are lowercase identifiers with underscores and no
        // spaces — `chat_bubble`, `arrow_downward`. Real prose has spaces, and
        // decorative content is punctuation or empty.
        let looks_like_a_ligature = !value.is_empty()
            && value.contains('_')
            && value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit());

        if looks_like_a_ligature {
            suspicious.push(value.to_string());
        }
    }

    assert!(
        suspicious.is_empty(),
        "these CSS `content` strings look like icon-font ligatures, which render \
         as literal words once the font is gone: {suspicious:?}"
    );
}

// ========== RATCHETS: THINGS THAT MUST NOT GET WORSE ==========
//
// A coverage floor is a ratchet on how much is tested. These are ratchets on
// the properties that coverage cannot see: that the memory budget still closes,
// that the join path still shares instead of copying, and that per-message work
// is still independent of how much history there is.
//
// None of them assert a wall-clock threshold. The registry rules those out as
// flaky (§6.4, and the "injectable clock" entry), and a number tuned to this
// machine says nothing about CI. Each of these is either arithmetic over the
// constants or a structural fact, so it fails for a reason rather than for a
// bad afternoon on a shared runner.

/// The reaction bar survives the pointer travelling to it.
///
/// It is `position: fixed` and a sibling of `#chat`, so moving towards it
/// *leaves* the chat. The first version hid the bar on that event, which meant
/// the buttons appeared and vanished the instant you aimed at them — visible,
/// documented, and completely unclickable with a mouse.
///
/// Three things make it reachable, and all three are asserted because any one
/// of them silently restores the bug:
///   1. Hiding is delayed, not immediate.
///   2. Entering the bar cancels the pending hide.
///   3. Leaving the chat *onto the bar* is not treated as leaving.
#[test]
fn the_reaction_bar_can_actually_be_clicked() {
    let js = EMBEDDED_JS;

    assert!(
        js.contains("const REACTION_BAR_GRACE_MS"),
        "hiding the bar must be delayed; an immediate hide is unreachable by a \
         pointer that has to travel to it"
    );
    assert!(
        js.contains("scheduleReactionBarHide") && js.contains("cancelReactionBarHide"),
        "the delayed hide must be cancellable, or the delay only postpones the bug"
    );
    assert!(
        js.contains("this.reactionBar.addEventListener('pointerenter'"),
        "arriving on the bar must cancel the hide"
    );
    assert!(
        js.contains("this.reactionBar.contains(e.relatedTarget)"),
        "leaving the chat onto the bar is not leaving"
    );

    // The chat's own pointerleave must not hide the bar outright.
    let handler = js
        .split_once("this.chat.addEventListener('pointerleave'")
        .and_then(|(_, rest)| rest.split_once("});"))
        .map(|(body, _)| body)
        .expect("client.js should handle pointerleave on the chat");

    assert!(
        !handler.contains("this.hideReactionBar()"),
        "leaving the chat must schedule the hide, not perform it — performing \
         it is what made the buttons unclickable"
    );

    // Rebuilding the bar under the cursor replaced the button between
    // pointerdown and pointerup, so the click landed on nothing.
    assert!(
        js.contains("this.reactionBarFor === messageId"),
        "the bar must not be rebuilt while it is already showing for the same \
         message"
    );
}

/// §10.3 — the conversation is a column, not the whole window.
///
/// `#chat` spanned the full width with bubbles capped at 600px, so on a wide
/// monitor one message sat against the left edge and the next against the
/// right, with a metre of nothing between them. Reading it meant tracking
/// across the screen line by line.
#[test]
fn the_conversation_is_a_readable_centred_column() {
    let css = embedded_html_without_comments();

    assert!(
        css.contains("--conversation-width"),
        "the column's width should be one named value, not repeated literals"
    );

    // The composer and the messages must share it, or the input floats free of
    // the conversation it belongs to.
    for surface in ["#chat", ".input-container"] {
        let block = css
            .split_once(&format!("\n{surface} {{"))
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(body, _)| body.to_string())
            .unwrap_or_default();
        assert!(
            block.contains("--conversation-width"),
            "{surface} must be laid out against the conversation column, or it \
             drifts away from the messages"
        );
    }
}

/// No selector is styled in two places.
///
/// A second rule for the same selector wins by being later, which is invisible
/// in either block: the first says one thing, the second quietly overrides it,
/// and the file reads as though both apply. `#chat`, `.message`,
/// `.input-container` and `.reaction-bar` all ended up defined twice while the
/// layout was being reworked, and "weirdly proportioned" is exactly what that
/// produces — a value fixed in one place and undone thirteen hundred lines
/// later.
#[test]
fn no_css_selector_is_defined_twice() {
    let css = embedded_html_without_comments();
    let Some(start) = css.find("<style>") else {
        panic!("the page should carry its stylesheet");
    };
    let sheet = &css[start..];

    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

    for line in sheet.lines() {
        // Top-level rules only: nested at-rule bodies are indented, and a
        // selector may legitimately repeat inside a different media query.
        if line.starts_with(char::is_whitespace) || !line.ends_with(" {") {
            continue;
        }
        let selector = line.trim_end_matches(" {").trim();
        if selector.is_empty()
            || selector.starts_with('@')
            || selector.starts_with(':')
            || selector.contains(',')
        {
            continue;
        }
        *counts.entry(selector).or_default() += 1;
    }

    let mut duplicated: Vec<(&str, usize)> = counts.into_iter().filter(|(_, n)| *n > 1).collect();
    duplicated.sort();

    assert!(
        duplicated.is_empty(),
        "these selectors are styled in more than one place, so one block \
         silently overrides the other: {duplicated:?}"
    );
}

/// Every class the page uses has a style — in the markup and in the renderer.
///
/// A class with no rule renders as an unstyled box: no error, no warning,
/// nothing in the console. It simply looks wrong. Both halves have now failed
/// this way during one stylesheet rework:
///
///   - `.message-image-faded`, set by the renderer, lost its box when a sweep
///     over `.message*` matched it too (`\b` matches at a hyphen).
///   - `.reply-preview`, in the markup, lost the base rule carrying
///     `display: none`, leaving only its `.active` variant — so the
///     placeholder text sat above the composer permanently, which is what a
///     reader actually reported.
#[test]
fn every_class_the_client_renders_is_styled() {
    let css = embedded_html_without_comments();
    let js = EMBEDDED_JS;

    let mut rendered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Classes in the page's own markup, too. Checking only the ones JavaScript
    // sets is how `.reply-preview` lost its base rule unnoticed: the bulk sweep
    // that removed the old `.message*` styles took it with it, leaving only
    // `.reply-preview.active { display: flex }` — so with nothing hiding it,
    // the placeholder text in the markup sat above the composer permanently.
    for (index, _) in EMBEDDED_HTML.match_indices("class=\"") {
        let rest = &EMBEDDED_HTML[index + "class=\"".len()..];
        let Some(end) = rest.find('"') else { continue };
        for word in rest[..end].split_whitespace() {
            if word.starts_with(|c: char| c.is_ascii_lowercase()) {
                rendered.insert(word.to_string());
            }
        }
    }

    // `className = 'a b'` and `` className = `a ${x} b` `` both appear. A
    // static word immediately followed by a token that is *purely* `${…}` —
    // nothing glued before or after it — marks that word as a base class
    // combined with an independently varying modifier: `` `system-message
    // ${type}` `` means every render is "system-message" plus whichever of
    // `type`'s values won, never "system-message" with nothing else. That
    // word needs a rule that names it *alone*, not merely a rule that
    // mentions it — see `bare_base` below.
    let mut bare_base: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for marker in ["className = '", "className = `"] {
        for (index, _) in js.match_indices(marker) {
            let rest = &js[index + marker.len()..];
            let end = rest.find(['\'', '`']).unwrap_or(0);
            let words: Vec<&str> = rest[..end].split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                let word = word.trim();
                if word.starts_with(|c: char| c.is_ascii_lowercase()) && !word.contains('$') {
                    rendered.insert(word.to_string());
                    let next_is_bare_placeholder = words
                        .get(i + 1)
                        .is_some_and(|w| w.starts_with("${") && w.ends_with('}'));
                    if next_is_bare_placeholder {
                        bare_base.insert(word.to_string());
                    }
                }
            }
        }
    }

    assert!(
        rendered.len() > 15,
        "expected to find the renderer's classes; found {rendered:?}"
    );

    // Every class token that appears in *any* selector, not "does the
    // selector's raw text contain this substring" — `.icon-sprite` used to
    // make a plain `css.contains(".icon")` check pass for `.icon` too.
    let styled: std::collections::HashSet<String> = css_rule_selectors(&css)
        .iter()
        .flat_map(|s| classes_in_selector(s))
        .collect();

    // `bare_base` needs more than mention: `.system-message.error {}` mentions
    // `system-message` as a token, same as a real base rule would, but only
    // ever matches an element that *also* carries `error` — it does not style
    // `system-message` on its own. `addSystemMessage`'s default `type =
    // 'info'` case rendered with nothing applied at all — no padding, no
    // radius, no centring — because no selector was ever just `.system-message`.
    let bareless: Vec<&String> = bare_base
        .iter()
        .filter(|class| {
            !css_rule_selectors(&css)
                .iter()
                .any(|s| classes_in_selector(s) == [(*class).clone()])
        })
        .collect();

    assert!(
        bareless.is_empty(),
        "these are rendered alone, with no other literal class glued to them \
         — `` `{{class}} ${{var}}` `` — so each needs a rule that is just \
         `.{{class}}`, not merely a rule that mentions it alongside a \
         modifier: {bareless:?}"
    );

    let unstyled: Vec<&String> = rendered
        .iter()
        .filter(|class| !styled.contains(class.as_str()))
        .collect();

    assert!(
        unstyled.is_empty(),
        "the client renders these classes and nothing styles them, so they \
         appear unstyled with no error anywhere: {unstyled:?}"
    );
}

/// A rule that *shows* something implies a rule that hides it.
///
/// `.reply-preview.active { display: flex }` says "visible when active", which
/// only means anything if the base `.reply-preview` is hidden. The base rule
/// was removed by a bulk sweep and nothing noticed: the class was still
/// mentioned all over the stylesheet, the page still parsed, and the reply
/// preview — with the placeholder text baked into its markup — simply sat above
/// the composer forever, reading "Replying to / Message text…" to every
/// visitor.
///
/// This is the §10.3 rule stated the other way round. That one says a
/// sometimes-present thing must not change the layout; this says a
/// sometimes-*visible* thing must actually start invisible.
#[test]
fn every_conditional_display_rule_has_a_base_that_hides_it() {
    let css = embedded_html_without_comments();

    // Rules of the shape `.thing.state { display: … }`, where the state is a
    // class the client toggles.
    let mut missing: Vec<String> = Vec::new();

    for (index, _) in css.match_indices(".active {") {
        let head = &css[..index];
        let Some(line_start) = head.rfind('\n') else {
            continue;
        };
        let selector = css[line_start + 1..index].trim();

        // Only simple `.base.active` selectors; a descendant selector is a
        // different shape and hides for different reasons.
        if !selector.starts_with('.') || selector.contains(' ') || selector.contains(',') {
            continue;
        }

        // The declaration must actually be about visibility.
        let body_end = css[index..].find('}').map_or(index, |e| index + e);
        if !css[index..body_end].contains("display:") {
            continue;
        }

        let base = selector.to_string();
        let base_rule = css
            .split_once(&format!("\n{base} {{"))
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(body, _)| body.to_string());

        match base_rule {
            Some(body) if body.contains("display: none") => {}
            _ => missing.push(base),
        }
    }

    assert!(
        missing.is_empty(),
        "these have a rule making them visible in one state but no base rule \
         hiding them, so they are visible in every state: {missing:?}"
    );
}

/// Nothing the shipped client is made of has quietly disappeared.
///
/// This is the contract the other client sweeps kept turning out to need. Each
/// of them catches one *kind* of loss after it has happened once —
/// `.message-image-faded` losing its box, `.reply-preview` losing the rule that
/// hid it, `.reaction-bar::before` losing the bridge that made a hover target
/// reachable. All three came from the same bulk edit over the stylesheet, none
/// was caught by the gate, and a visitor reported one of them.
///
/// The common factor is not any of those classes. It is that **nothing knew
/// those rules were supposed to be there.** A regex over a stylesheet does not
/// know what a rule is for, and neither does a reviewer reading a 4000-line
/// diff.
///
/// So the page's parts are written down. `scripts/client-inventory.toml` lists
/// every element id, every stylesheet rule and every client method that exists
/// today; this fails when one of them stops existing. Adding things is free —
/// only removal is a decision, and making it a decision is the whole point.
///
/// **When this fails, answer the question rather than silencing it.** If the
/// removal was an accident, restore the rule. If it was deliberate, delete the
/// line from the manifest in the same commit. `scripts/client-inventory.sh`
/// prints both sides of the difference.
#[test]
fn the_shipped_client_still_contains_everything_it_did() {
    const MANIFEST: &str = include_str!("../../scripts/client-inventory.toml");

    /// The entries of one `name = [ … ]` list in the manifest.
    fn listed(manifest: &str, name: &str) -> Vec<String> {
        let body = manifest
            .split_once(&format!("{name} = ["))
            .and_then(|(_, rest)| rest.split_once("\n]"))
            .map(|(body, _)| body)
            .unwrap_or_else(|| panic!("the manifest should define `{name}`"));

        body.lines()
            .map(str::trim)
            .filter(|line| line.starts_with('\'') || line.starts_with('"'))
            .map(|line| {
                let quote = line.chars().next().expect("just checked it is quoted");
                line.trim_start_matches(quote)
                    .rsplit_once(quote)
                    .map(|(value, _)| value.to_string())
                    .unwrap_or_default()
            })
            .collect()
    }

    let css = embedded_html_without_comments();
    let sheet = css
        .split_once("<style>")
        .map(|(_, rest)| rest)
        .expect("the page carries its stylesheet");

    let present_selectors: std::collections::HashSet<&str> = sheet
        .lines()
        .filter(|line| line.ends_with(" {") && !line.starts_with([' ', '\t', '@']))
        .map(|line| line.trim_end_matches(" {").trim())
        .collect();

    let mut gone: Vec<String> = Vec::new();

    for id in listed(MANIFEST, "ids") {
        if !EMBEDDED_HTML.contains(&format!("id=\"{id}\"")) {
            gone.push(format!("element id: {id}"));
        }
    }

    for selector in listed(MANIFEST, "selectors") {
        if !present_selectors.contains(selector.as_str()) {
            gone.push(format!("stylesheet rule: {selector}"));
        }
    }

    for method in listed(MANIFEST, "methods") {
        if !EMBEDDED_JS.contains(&format!("\n    {method}(")) {
            gone.push(format!("client method: {method}"));
        }
    }

    assert!(
        gone.is_empty(),
        "these are listed in scripts/client-inventory.toml and are no longer in \
         the shipped client. Either the removal was an accident — restore it — \
         or it was deliberate, in which case delete the line from the manifest \
         in the same commit. `scripts/client-inventory.sh` shows both sides.\n\n{gone:#?}"
    );

    // A manifest that has quietly emptied catches nothing.
    assert!(
        listed(MANIFEST, "selectors").len() > 150
            && listed(MANIFEST, "ids").len() > 40
            && listed(MANIFEST, "methods").len() > 50,
        "the manifest has shrunk to the point of not being a record of anything"
    );
}

/// No rule styles something the page never renders.
///
/// Dead CSS is not merely clutter. When the header was restructured, the rules
/// for the design it replaced — `.brand`, `.room-info`, `.room-badge` — stayed
/// behind. A rule for something that no longer exists is a rule nobody reads,
/// and the next person to reuse that name inherits it.
#[test]
fn no_rule_styles_something_the_page_never_renders() {
    let alive = classes_the_page_can_render();
    assert!(alive.len() > 40, "expected to find the page's classes");

    let css = embedded_html_without_comments();
    let mut dead: Vec<String> = Vec::new();

    for selector in css_rule_selectors(&css) {
        if !selector.starts_with('.') {
            continue;
        }

        let mentioned = classes_in_selector(&selector);

        // Every class named in the selector must be alive, not merely one of
        // them. `.old-container .icon` only ever matches an `.icon` that is a
        // descendant of `.old-container` — if the container never renders,
        // the rule is entirely dead regardless of how common `.icon` is
        // elsewhere. Requiring only one match let `.logo-icon .icon` survive
        // the header rewrite that deleted `.logo-icon`: `.icon` alone is used
        // on every SVG in the page, so the rule read as "alive" while
        // matching nothing a browser could ever select.
        if !mentioned.is_empty() && !mentioned.iter().all(|c| alive.contains(c)) {
            dead.push(selector.to_string());
        }
    }

    assert!(
        dead.is_empty(),
        "these rules style classes the page never renders — leftovers from a \
         design that was replaced, which the next person to reuse the name will \
         inherit: {dead:?}"
    );
}

/// Ids carry no layout, so a class can always win.
///
/// `#userCount` styled a small inline count. The header was restructured and
/// that id moved onto the button in the centre — a column with a name over a
/// subtitle — and the old rule beat every class on it, because an id selector
/// outranks any number of classes. The centre of the navigation bar laid itself
/// out as a row of chips inside a column: icons stacked, nothing errored, and
/// it was reported as "the icons are really not showing up right".
///
/// Ids identify; classes style. The exceptions are structural containers that
/// exist exactly once and are never restyled by role.
#[test]
fn stylesheet_ids_do_not_carry_layout() {
    const STRUCTURAL: &[&str] = &[
        "#chat",
        "#typingIndicator",
        "#replyPreview",
        "#emojiPanel",
        "#participantsSheet",
        "#lightbox",
        "#dropOverlay",
        "#jumpLatest",
        "#reactionBar",
        "#attachmentTray",
        "#welcomeBanner",
        "#messageInput",
        "#emojiSearch",
        "#emojiGrid",
        "#emojiTabs",
        "#emojiEmpty",
        "#sendBtn",
        "#charCount",
        "#participantsList",
        "#attachmentPreview",
        "#roomAvatar",
        "#lightboxImage",
        "#lightboxClose",
        "#participantsClose",
        "#jumpLatestCount",
        "#typingText",
        "#statusChip",
        "#roomHeartbeat",
    ];

    let css = embedded_html_without_comments();
    let mut offenders: Vec<String> = Vec::new();

    for selector in css_rule_selectors(&css) {
        if !selector.starts_with('#') {
            continue;
        }
        let id = selector
            .split([' ', ':', '.', '>'])
            .next()
            .unwrap_or(&selector);
        if !STRUCTURAL.contains(&id) {
            offenders.push(selector.to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these style an element by id. An id beats every class, so the next time \
         that id is reused for a different control the old layout wins silently \
         — which is how the navigation bar ended up stacked. Style by class: \
         {offenders:?}"
    );

    // Self-cleaning: an exemption must not outlive the element it names.
    for id in STRUCTURAL {
        let bare = id.trim_start_matches('#');
        assert!(
            EMBEDDED_HTML.contains(&format!("id=\"{bare}\"")),
            "{id} is exempted here but is no longer in the page — delete the entry"
        );
    }
}

/// `.nav-back` is a plain `<a href>`, not a client-side route: each click is a
/// full page load that tears down the current WebSocket and opens a new one.
/// Clicking it several times in the moment before the browser navigates away
/// fires that many overlapping loads — indistinguishable, from the room's
/// side, from a rapid-reconnect flood, and exactly what the room-join
/// throttle (`MAX_ROOM_JOIN_ATTEMPTS`) exists to catch. The fix belongs on the
/// link, not in that throttle, so the shipped script must actually debounce
/// it rather than pass it straight through.
#[test]
fn nav_back_is_debounced_against_repeated_clicks() {
    assert!(
        EMBEDDED_JS.contains("function debounceNavigation"),
        "the shipped script must define a navigation debounce guard"
    );
    assert!(
        EMBEDDED_JS.contains("debounceNavigation('.nav-back')"),
        "and must apply it to the back-to-main link specifically, or a fast \
         double-click still fires two page loads"
    );
}

/// The explore-a-random-room link is the same class of bug as `.nav-back`,
/// one level worse: it navigates via JS rather than a plain href, and a
/// second click before the first navigation lands does not just repeat the
/// same page load, it picks a *different* random room and races the two
/// navigations against each other.
#[test]
fn explore_link_is_guarded_against_repeated_clicks() {
    assert!(
        EMBEDDED_JS.contains("if (this.exploringRoom) return;")
            && EMBEDDED_JS.contains("this.exploringRoom = true;"),
        "the explore-room click handler must ignore a second click before \
         the first navigation actually leaves the page"
    );
}
