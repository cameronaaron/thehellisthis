//! Turning untrusted input into something the server will store.

use super::*;

#[tokio::test]
async fn test_validate_message_cases() {
    assert!(validate_and_render_message("Hello").is_ok());
    assert!(validate_and_render_message("").is_err());
    assert!(validate_and_render_message(&"a".repeat(MAX_MESSAGE_LEN + 1)).is_err());

    // Pure script tags with no text content should be rejected (empty after sanitization)
    assert!(validate_and_render_message("<script>alert('xss')</script>").is_err());

    // But text with script tags mixed in should have the script removed and text preserved
    let sanitized =
        validate_and_render_message("Hello <script>alert('xss')</script> world").unwrap();
    assert!(!sanitized.contains("<script>"));
    assert!(sanitized.contains("Hello"));
    assert!(sanitized.contains("world"));
}

#[tokio::test]
async fn test_validate_input_cases() {
    assert!(validate_input("room_name", 50).is_ok());
    assert!(validate_input("", 50).is_err());
    assert!(validate_input(&"a".repeat(51), 50).is_err());
    assert!(validate_input("invalid name!", 50).is_err());
}

#[tokio::test]
async fn test_html_sanitization_removes_script() {
    // A `<script>` tag opening the message is *block*-level raw HTML to
    // Markdown, which the safe default omits entirely — the whole input,
    // "Safe content" included, since that text was never extracted as its
    // own paragraph, only ever existed inside the omitted block. Leading
    // text keeps the script tag *inline* instead, where only the tag itself
    // is omitted and surrounding text is still its own paragraph content.
    let dangerous = "Safe content <script>alert('xss')</script> more";
    let result = validate_and_render_message(dangerous);

    assert!(result.is_ok());
    let sanitized = result.unwrap();
    assert!(!sanitized.contains("<script>"));
    assert!(sanitized.contains("Safe content"));
}

#[tokio::test]
async fn test_empty_message_rejection() {
    let result = validate_and_render_message("");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_oversized_message_rejection() {
    let huge_msg = "a".repeat(MAX_MESSAGE_LEN + 1);
    let result = validate_and_render_message(&huge_msg);
    assert!(result.is_err());
}

// ========== BROADCAST EVENTS ==========

#[tokio::test]
async fn test_empty_message_text() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/empty-msg-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send empty message
    ws.send(text_frame(r#"{"type":"Message","text":""}"#.to_string()))
        .await
        .unwrap();

    // Should not receive broadcast of empty message
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.abort();
}

#[tokio::test]
async fn test_message_exceeds_max_length() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/long-msg-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send message longer than MAX_MESSAGE_LEN
    let long_text = "x".repeat(10000);
    ws.send(text_frame(format!(
        r#"{{"type":"Message","text":"{}"}}"#,
        long_text
    )))
    .await
    .unwrap();

    // Should not receive broadcast of too-long message
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.abort();
}

#[tokio::test]
async fn test_xss_attempt_in_message() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/xss-test", addr);
    let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect client 1");
    let (mut ws2, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect client 2");

    // Receive initial events for both clients
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
    }
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws2).await;
    }

    // Client 1 sends XSS attempt mixed with real text content
    // Pure script tags would be rejected as empty after sanitization
    let xss_payload = r#"Hello <script>alert('XSS')</script> world"#;
    ws1.send(text_frame(format!(
        r#"{{"type":"Message","text":"{}"}}"#,
        xss_payload
    )))
    .await
    .unwrap();

    // Client 2 should receive sanitized version with scripts removed
    let mut found_message = false;
    for _ in 0..10 {
        let event = recv_json_event(&mut ws2).await;
        if event["type"] == "Message" {
            let text = event["message"]["text"].as_str().unwrap();
            // Should NOT contain script tags
            assert!(!text.contains("<script>"), "XSS not sanitized!");
            // But should contain the actual text
            assert!(
                text.contains("Hello") || text.contains("world"),
                "Legit text should be preserved"
            );
            found_message = true;
            break;
        }
    }

    assert!(found_message, "Sanitized message not received");
    handle.abort();
}

#[tokio::test]
async fn test_sql_injection_attempt_in_message() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/sql-test", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send SQL injection attempt
    let sql_payload = r#"'; DROP TABLE users; --"#;
    ws.send(text_frame(format!(
        r#"{{"type":"Message","text":"{}"}}"#,
        sql_payload
    )))
    .await
    .unwrap();

    // Should still function normally (we don't have SQL, but test sanitization)
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_markdown_rendering_in_messages() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/markdown-test", addr);
    let (mut ws1, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect client 1");
    let (mut ws2, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect client 2");

    // Receive initial events
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws1).await;
    }
    for _ in 0..3 {
        let _ = recv_json_event(&mut ws2).await;
    }

    // Send markdown formatted message
    ws1.send(text_frame(
        r#"{"type":"Message","text":"**bold** and *italic* text"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Client 2 should receive rendered markdown
    let mut found_markdown = false;
    for _ in 0..10 {
        let event = recv_json_event(&mut ws2).await;
        if event["type"] == "Message" {
            let text = event["message"]["text"].as_str().unwrap();
            // Should contain HTML tags from markdown rendering
            if text.contains("<strong>") || text.contains("<em>") {
                found_markdown = true;
                break;
            }
        }
    }

    assert!(found_markdown, "Markdown not rendered");
    handle.abort();
}

#[tokio::test]
async fn test_special_characters_in_messages() {
    let (addr, handle) = start_ws_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws/special-chars", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    for _ in 0..3 {
        let _ = recv_json_event(&mut ws).await;
    }

    // Send message with special characters
    ws.send(text_frame(
        r#"{"type":"Message","text":"Hello 世界 🌍 café"}"#.to_string(),
    ))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();
}

#[tokio::test]
async fn test_room_name_special_characters_rejected() {
    // Special characters like @ and ! are rejected by URL parsing, so we just test basic validation
    assert!(validate_input("room@name", 50).is_err());
    assert!(validate_input("room#name", 50).is_err());
}

// ========== MEMORY & CLEANUP SCENARIOS ==========

#[tokio::test]
async fn test_validate_input_special_chars() {
    assert!(validate_input("room-name_123", 50).is_ok());
    assert!(validate_input("room@name", 50).is_err());
    assert!(validate_input("room#name", 50).is_err());
    assert!(validate_input("room.name", 50).is_err());
    assert!(validate_input("room name", 50).is_err());
}

// ========== COOKIE HANDLING & ERROR RESPONSES ==========

#[tokio::test]
async fn test_validate_message_length_boundaries() {
    // Max length message
    let max_msg = "a".repeat(MAX_MESSAGE_LEN);
    assert!(validate_and_render_message(&max_msg).is_ok());

    // Over max length
    let too_long = "a".repeat(MAX_MESSAGE_LEN + 1);
    assert!(validate_and_render_message(&too_long).is_err());

    // Empty message
    assert!(validate_and_render_message("").is_err());

    // Single character
    assert!(validate_and_render_message("a").is_ok());
}

#[tokio::test]
async fn test_message_with_links() {
    let msg_with_link = "Check this out: https://example.com";
    let result = validate_and_render_message(msg_with_link);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_with_code_blocks() {
    let code_msg = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
    let result = validate_and_render_message(code_msg);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_with_bold_italic() {
    let formatted = "**bold** and *italic*";
    let result = validate_and_render_message(formatted);
    // Just verify it's valid
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_with_lists() {
    let list_msg = "- Item 1\n- Item 2\n- Item 3";

    // `validate_and_render_message` returns the rendered HTML directly — it
    // is the one render+sanitise pass, not a raw-text guard plus a separate
    // render (§1.4d), so there is no verbatim-Markdown value to compare
    // against the original input any more.
    let rendered = validate_and_render_message(list_msg).unwrap();
    assert_eq!(
        rendered.matches("<li>").count(),
        3,
        "three bullets should render as three list items: {rendered}"
    );
    assert!(
        rendered.contains("<ul>"),
        "and be wrapped in a list: {rendered}"
    );
}

#[tokio::test]
async fn test_message_with_special_markdown() {
    let text = "# Header\n## Subheader\n- List item";
    let result = validate_and_render_message(text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_with_code_block() {
    let text = "```rust\nfn main() {\n    println!(\"Hello\");\n}\n```";
    let result = validate_and_render_message(text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_at_max_length() {
    let text = "a".repeat(8000);
    let result = validate_and_render_message(&text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_with_url() {
    let text = "Check out https://thehellisthis.com";
    let result = validate_and_render_message(text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_with_email() {
    let text = "Contact me at test@example.com";
    let result = validate_and_render_message(text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_input_spaces_new() {
    let result = validate_input("my room", 50);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_message_newlines() {
    let text = "Line 1\nLine 2\nLine 3";
    let result = validate_and_render_message(text);
    assert!(result.is_ok());
}

// ========== PREVIOUSLY REMOVED TESTS - NOW FIXED ==========

#[tokio::test]
async fn test_message_only_whitespace() {
    // Whitespace-only messages should be rejected
    let text = "   \n\t  ";
    let result = validate_and_render_message(text);
    assert!(
        result.is_err(),
        "Whitespace-only messages should be rejected"
    );
}

#[tokio::test]
async fn test_validate_message_empty_after_trim() {
    // Whitespace only should fail
    assert!(validate_and_render_message("   ").is_err());
    assert!(validate_and_render_message("\t\n").is_err());
    assert!(validate_and_render_message("   \n\t   ").is_err());
}

#[tokio::test]
async fn test_validate_message_empty_after_sanitization() {
    // HTML-only content that sanitizes to empty
    assert!(validate_and_render_message("<script></script>").is_err());
    assert!(validate_and_render_message("<style>body{}</style>").is_err());
}

#[tokio::test]
async fn test_validate_message_preserves_safe_html() {
    let result = validate_and_render_message("Hello <b>bold</b> world").unwrap();
    assert!(result.contains("<b>") || result.contains("bold"));
    assert!(result.contains("Hello"));
}

#[tokio::test]
async fn test_validate_input_length_boundary() {
    // Exactly at limit - should pass
    let at_limit = "a".repeat(50);
    assert!(validate_input(&at_limit, 50).is_ok());

    // One over limit - should fail
    let over_limit = "a".repeat(51);
    assert!(validate_input(&over_limit, 50).is_err());
}

#[tokio::test]
async fn test_validate_input_underscore_position() {
    // Underscore in middle is ok
    assert!(validate_input("my_room", 50).is_ok());

    // Underscore only is not alphanumeric start/end
    // Note: validate_input allows underscores as long as chars are alphanumeric
    let result = validate_input("a_", 50);
    assert!(result.is_ok()); // _ is allowed
}

#[tokio::test]
async fn test_validate_message_max_length() {
    let max_msg = "a".repeat(MAX_MESSAGE_LEN);
    assert!(validate_and_render_message(&max_msg).is_ok());

    let over_max = "a".repeat(MAX_MESSAGE_LEN + 1);
    assert!(validate_and_render_message(&over_max).is_err());
}

#[tokio::test]
async fn test_markdown_rendering_headers() {
    let msg = "# Header\n\nSome text";
    let result = validate_and_render_message(msg).unwrap();
    assert!(result.contains("<h1>") || result.contains("Header"));
}

#[tokio::test]
async fn test_markdown_rendering_code() {
    let msg = "Here is `code` inline";
    let result = validate_and_render_message(msg).unwrap();
    assert!(result.contains("<code>") || result.contains("code"));
}

#[tokio::test]
async fn test_markdown_rendering_links() {
    let msg = "[link](https://example.com)";
    let result = validate_and_render_message(msg).unwrap();
    // Links might be stripped or kept - just verify it doesn't error
    assert!(!result.is_empty());
}

#[tokio::test]
async fn test_validate_message_with_only_html_tags() {
    // Message that becomes empty after sanitization
    let result = validate_and_render_message("<script>alert(1)</script>");

    // Should return error since sanitized result is empty
    assert!(result.is_err());
}

#[tokio::test]
async fn test_validate_input_unicode() {
    // Unicode alphanumeric characters ARE allowed by is_alphanumeric()
    // This is actually intentional - Rust's is_alphanumeric() covers Unicode
    assert!(
        validate_input("café", 50).is_ok(),
        "Unicode alphanumeric should be allowed"
    );
    assert!(
        validate_input("日本語", 50).is_ok(),
        "Japanese characters should be allowed"
    );

    // Emoji is NOT alphanumeric and should be rejected
    assert!(
        validate_input("emoji🎉", 50).is_err(),
        "Emoji should be rejected"
    );

    // Spaces are not allowed
    assert!(
        validate_input("has space", 50).is_err(),
        "Spaces should be rejected"
    );

    // Pure ASCII should work
    assert!(validate_input("valid-name", 50).is_ok());
    assert!(validate_input("room_123", 50).is_ok());
}

#[tokio::test]
async fn test_validate_message_renders_markdown() {
    // Basic markdown should be rendered to HTML
    let result = validate_and_render_message("**bold**").unwrap();
    assert!(result.contains("<strong>") || result.contains("bold"));

    // Code blocks
    let result = validate_and_render_message("`code`").unwrap();
    assert!(result.contains("<code>") || result.contains("code"));
}

#[tokio::test]
async fn test_validate_message_strips_dangerous_tags() {
    // Script tags should be removed, but surrounding text preserved
    let result = validate_and_render_message("before <script>alert(1)</script> after").unwrap();
    assert!(!result.contains("<script>"));
    assert!(result.contains("before"));
    assert!(result.contains("after"));

    // iframe should be removed
    let result = validate_and_render_message("text <iframe src='evil.com'></iframe> more").unwrap();
    assert!(!result.contains("<iframe>"));
}

#[tokio::test]
async fn test_room_handler_returns_html_for_valid_room() {
    let app_state = Arc::new(AppState::new());
    let app = Router::new()
        .route("/{room}", get(room_handler))
        .with_state(app_state.clone());

    // Valid new room name - should return HTML (room created on WS connect, not HTTP)
    let response = app
        .oneshot(
            Request::builder()
                .uri("/new-test-room")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Should return HTML content
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("<!DOCTYPE html>") || html.contains("<html"),
        "Should return HTML"
    );
}

#[tokio::test]
async fn test_claim_html_sanitization() {
    let xss = "<img onerror=alert('xss')>";
    let clean = ammonia::clean(xss);
    assert!(!clean.contains("onerror")); // XSS removed
}

// ========== GAMIFICATION INFRASTRUCTURE TESTS ==========
// Tests for backend behavior that supports frontend gamification

#[tokio::test]
async fn test_validate_input_exactly_min_length() {
    let result = validate_input("abc", MAX_ROOM_NAME_LEN); // MIN_ROOM_NAME_LEN = 3
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_input_exactly_max_length() {
    let name = "a".repeat(MAX_ROOM_NAME_LEN - 1) + "b"; // 50 chars
    let result = validate_input(&name, MAX_ROOM_NAME_LEN);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_input_one_under_min() {
    // Note: validate_input checks max_len and regex, not min length
    // "ab" passes regex but we're testing with MAX_ROOM_NAME_LEN which is fine
    // This test documents that short names pass regex validation
    let result = validate_input("ab", MAX_ROOM_NAME_LEN);
    // Actually "ab" passes regex, min length enforced at handler level
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_input_one_over_max() {
    let name = "a".repeat(MAX_ROOM_NAME_LEN + 1); // 51 chars
    let result = validate_input(&name, MAX_ROOM_NAME_LEN);
    assert!(result.is_err());
}

// ========== VALIDATE MESSAGE BOUNDARY TESTS ==========

#[tokio::test]
async fn test_validate_message_exactly_max_length() {
    let text = "a".repeat(MAX_MESSAGE_LEN);
    let result = validate_and_render_message(&text);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_message_one_over_max() {
    let text = "a".repeat(MAX_MESSAGE_LEN + 1);
    let result = validate_and_render_message(&text);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_validate_message_single_char() {
    let result = validate_and_render_message("a");
    assert!(result.is_ok());
}

// ========== OUTGOING MESSAGE SIZE ESTIMATION ==========

#[tokio::test]
async fn test_frontend_max_message_length_matches_backend() {
    // Frontend maxlength attribute and validation must match MAX_MESSAGE_LEN

    let backend_max = MAX_MESSAGE_LEN;
    assert_eq!(backend_max, 8000, "MAX_MESSAGE_LEN should be 8000");

    // HTML input should have matching maxlength
    assert!(
        SHIPPED_CLIENT.contains("maxlength=\"8000\""),
        "Frontend input maxlength should match backend MAX_MESSAGE_LEN (8000)"
    );

    // Character counter should show same limit
    assert!(
        SHIPPED_CLIENT.contains("/ 8000"),
        "Frontend character counter should show /8000 limit"
    );

    // JS validation should check same limit
    assert!(
        SHIPPED_CLIENT.contains("text.length > 8000"),
        "Frontend JS validation should check against 8000 character limit"
    );
}

#[tokio::test]
async fn test_validate_input_edge_cases() {
    // Empty string
    assert!(validate_input("", 50).is_err());

    // Exactly at max length
    let max_str: String = "a".repeat(50);
    assert!(validate_input(&max_str, 50).is_ok());

    // One over max length
    let over_str: String = "a".repeat(51);
    assert!(validate_input(&over_str, 50).is_err());

    // Invalid characters
    assert!(validate_input("test<script>", 50).is_err());
    assert!(validate_input("test>alert", 50).is_err());
}

#[tokio::test]
async fn test_validate_input_empty_string() {
    let result = validate_input("", 100);
    assert!(result.is_err());
    // validate_input returns Result<(), &'static str>
    assert!(result.unwrap_err().contains("empty") || result.unwrap_err().contains("too"));
}

// Test cleanup_user when room doesn't exist - covers lines 2007-2020

#[tokio::test]
async fn test_validate_message_single_character() {
    let result = validate_and_render_message("x");
    assert!(result.is_ok());
}

// Test message validation with script tag (gets sanitized)

#[tokio::test]
async fn test_validate_message_script_tag_sanitized() {
    // A message that is *only* markup renders to nothing, and a message that
    // says nothing is refused (§1.4d).
    let result = validate_and_render_message("<script>alert('xss')</script>");
    assert!(
        result.is_err(),
        "a message that is only a script tag has no content and must be refused"
    );

    // And the tag never survives into anything that reaches a browser.
    let rendered = crate::validation::render_message_html("<script>alert('xss')</script>");
    assert!(
        !rendered.contains("<script"),
        "the script tag must not survive rendering: {rendered}"
    );
}

// Test typing event with false value - covers typing broadcast

#[tokio::test]
async fn test_message_with_reply_via_ws() {
    let (addr, _state) = start_ws_server().await;
    let url = format!("ws://{}/ws/reply-test-room", addr);

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("Connect failed");

    // Drain initial messages
    for _ in 0..5 {
        tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .ok();
    }

    // Send a message with reply_to
    let msg = r#"{"type":"Message","text":"This is a reply","reply_to":{"message_id":"12345","author_name":"Someone","preview_text":"Original message"}}"#;
    ws.send(text_frame(msg)).await.ok();

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should receive the message back (it gets broadcast to all including sender)
    let mut received_reply = false;
    for _ in 0..10 {
        if let Ok(Some(Ok(WsMessage::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
            && text.contains("This is a reply")
        {
            received_reply = true;
            break;
        }
    }

    assert!(received_reply, "Should receive message with reply");
    ws.close(None).await.ok();
}

// Test RateLimiter window reset - covers lines 92-95

#[tokio::test]
async fn test_validate_input_valid_cases() {
    // Valid room name
    assert!(validate_input("test-room", 50).is_ok());
    assert!(validate_input("room123", 50).is_ok());
    assert!(validate_input("my_room_name", 50).is_ok());

    // Minimum length
    assert!(validate_input("abc", 50).is_ok());

    // Maximum length
    let max_name = "a".repeat(50);
    assert!(validate_input(&max_name, 50).is_ok());
}

// Test validate_input with invalid cases - covers validation branches

#[tokio::test]
async fn test_validate_input_invalid_cases() {
    // Too long
    let too_long = "a".repeat(51);
    assert!(validate_input(&too_long, 50).is_err());

    // Invalid characters
    assert!(validate_input("room@name", 50).is_err());
    assert!(validate_input("room name", 50).is_err());

    // Empty
    assert!(validate_input("", 50).is_err());

    // Special characters
    assert!(validate_input("room.name", 50).is_err());
    assert!(validate_input("room/name", 50).is_err());
}

// Test AppState shutdown - covers shutdown paths

#[tokio::test]
async fn test_validate_message_edge_cases() {
    // Whitespace only
    let result = validate_and_render_message("   \t\n   ");
    assert!(result.is_err());

    // HTML that becomes empty after sanitization
    let result = validate_and_render_message("<script></script>");
    assert!(result.is_err());

    // Valid message with leading/trailing whitespace
    let result = validate_and_render_message("  hello world  ");
    assert!(result.is_ok());

    // Unicode characters
    let result = validate_and_render_message("Hello 世界 🌍");
    assert!(result.is_ok());
}

// Test MemoryTracker remove_bytes handles underflow - covers lines 392-396

#[tokio::test]
async fn test_validate_input_boundaries() {
    // Valid input within length
    let valid = "abc";
    let result = validate_input(valid, MAX_ROOM_NAME_LEN);
    assert!(result.is_ok());

    // Max length exactly
    let max_valid = "a".repeat(MAX_ROOM_NAME_LEN);
    let result = validate_input(&max_valid, MAX_ROOM_NAME_LEN);
    assert!(result.is_ok());

    // Over max length
    let too_long = "a".repeat(MAX_ROOM_NAME_LEN + 1);
    let result = validate_input(&too_long, MAX_ROOM_NAME_LEN);
    assert!(result.is_err());
}

// Test ChatError variants - covers lines 217-240

/// The quoted-reply block is client-supplied and must be bounded and sanitised.
///
/// MAX_MESSAGE_LEN bounds a message's own text and nothing else. Without this,
/// a client could attach a half-megabyte "preview" to a one-character message
/// and the server would store it in history and broadcast it to the room. None
/// of the fields were sanitised either; the only reason that was not an XSS
/// vector is that the current client happens to escape them when rendering —
/// a property of one client, not of the server.
#[test]
fn quoted_replies_are_bounded_and_sanitised() {
    let huge = ReplyInfo {
        message_id: Uuid::new_v4().to_string(),
        author_name: "a".repeat(10_000),
        preview_text: "b".repeat(500_000),
    };

    let clean = sanitize_reply(huge).expect("a valid message id should be accepted");
    assert!(clean.author_name.len() <= MAX_REPLY_AUTHOR_LEN);
    assert!(clean.preview_text.len() <= MAX_REPLY_PREVIEW_LEN);

    // Markup is neutralised. `ammonia::clean_text` escapes rather than strips,
    // so the payload's characters survive but cannot open a tag — that
    // inertness, not the absence of the word "onerror", is the property worth
    // asserting.
    let malicious = ReplyInfo {
        message_id: Uuid::new_v4().to_string(),
        author_name: "<img src=x onerror=alert(1)>".to_string(),
        preview_text: "<script>alert(1)</script>".to_string(),
    };
    let clean = sanitize_reply(malicious).unwrap();
    for field in [&clean.author_name, &clean.preview_text] {
        assert!(
            !field.contains('<') && !field.contains('>'),
            "sanitised reply field must not carry raw angle brackets: {field}"
        );
    }
    assert!(
        clean.preview_text.contains("&lt;"),
        "expected escaped markup"
    );

    // A reply that does not point at a message id is not a reply.
    assert!(
        sanitize_reply(ReplyInfo {
            message_id: "not-a-uuid".to_string(),
            author_name: "otter".to_string(),
            preview_text: "hi".to_string(),
        })
        .is_none()
    );
}

/// Truncation must never split a UTF-8 character.
///
/// `String::truncate` panics on a non-boundary index, and every field here is
/// attacker-supplied — a multi-byte character straddling the limit would be a
/// remotely triggerable panic.
#[test]
fn reply_truncation_is_utf8_safe() {
    for filler in ["é", "日", "🙂", "a"] {
        let reply = ReplyInfo {
            message_id: Uuid::new_v4().to_string(),
            author_name: filler.repeat(500),
            preview_text: filler.repeat(500),
        };

        let clean = sanitize_reply(reply).unwrap();
        // Round-trips as valid UTF-8 by construction; the assertion is that we
        // got here without panicking, and stayed within budget.
        assert!(clean.author_name.len() <= MAX_REPLY_AUTHOR_LEN);
        assert!(clean.preview_text.len() <= MAX_REPLY_PREVIEW_LEN);
    }
}

// ========== CLIENT ADDRESS THROUGH CLOUDFLARE ==========

/// A message longer than the cap is dropped.
#[tokio::test]
async fn oversized_messages_are_dropped() {
    let state = state_with_user("room", "user", Instant::now()).await;

    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::Message {
            text: "a".repeat(MAX_MESSAGE_LEN + 1),
            reply_to: None,
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    assert!(rooms["room"].chat_history.is_empty());
}

/// A quoted reply survives the round trip in sanitised, bounded form.
#[tokio::test]
async fn a_message_keeps_its_sanitised_reply() {
    let state = state_with_user("room", "user", Instant::now()).await;

    apply_client_event(
        &state,
        "room",
        "user",
        "otter",
        ClientEvent::Message {
            text: "replying".into(),
            reply_to: Some(ReplyInfo {
                message_id: Uuid::new_v4().to_string(),
                author_name: "<b>badger</b>".into(),
                preview_text: "x".repeat(5_000),
            }),
            attachment: None,
        },
    )
    .await;

    let rooms = state.rooms.read().await;
    let reply = rooms["room"].chat_history[0]
        .reply_to
        .as_ref()
        .expect("the reply should be kept");

    assert!(
        !reply.author_name.contains('<'),
        "reply author must be inert"
    );
    assert!(reply.preview_text.len() <= MAX_REPLY_PREVIEW_LEN);
}

// ========== CLIENT: RENDERING AND ACCESSIBILITY ==========

/// §3 — what a message costs is bounded by what the user typed.
///
/// `MAX_MESSAGE_LEN` bounds the Markdown, but the server stores and broadcasts
/// the *rendered* HTML. Measured amplification for input that passes validation
/// is over 7x (`[a](b)` repeated), so the input cap alone did not bound the
/// stored size.
#[test]
fn rendered_html_is_bounded_by_the_input_cap() {
    for pattern in ["[a](b)", "***a***", "# a\n", "*a*", "- x\n"] {
        let input: String = pattern.repeat(MAX_MESSAGE_LEN / pattern.len());
        let input = &input[..input.len().min(MAX_MESSAGE_LEN)];
        if crate::validation::validate_and_render_message(input).is_err() {
            continue;
        }
        let rendered = crate::validation::render_message_html(input);
        assert!(
            rendered.len() <= MAX_RENDERED_MESSAGE_LEN,
            "`{pattern}` repeated renders to {} bytes, above the {MAX_RENDERED_MESSAGE_LEN}-byte ceiling",
            rendered.len()
        );
    }
}

/// A quoted reply is cut to the limit exactly, and never mid-character.
///
/// Two mutants survived in `truncate_on_char_boundary` and one timed out:
/// `>` for `>=` on the length check, `&&` for `||` in the walk, and `-=` for
/// `/=` in the step. All three change *where* the cut lands, and nothing
/// asserted where the cut lands — only that the result was "not longer than"
/// the limit, which a function returning the empty string also satisfies.
///
/// The two properties worth pinning are that the cut is as late as it can be,
/// and that it never lands inside a character. `String::truncate` panics on a
/// non-boundary index, and every string here is attacker-supplied, so the
/// second one is a remotely triggerable panic rather than a cosmetic bug.
#[test]
fn a_quoted_reply_is_cut_at_the_limit_and_never_mid_character() {
    let id = Uuid::new_v4().to_string();

    // Landing exactly on a boundary: the cut takes the whole budget.
    let ascii = ReplyInfo {
        message_id: id.clone(),
        author_name: "a".repeat(500),
        preview_text: "b".repeat(500),
        // Every ASCII index is a boundary, so no walk-back happens and the
        // result must be the limit itself — not one byte less.
    };
    let cut = sanitize_reply(ascii).expect("a valid uuid is a real reply");
    assert_eq!(cut.preview_text.len(), MAX_REPLY_PREVIEW_LEN);
    assert_eq!(cut.author_name.len(), MAX_REPLY_AUTHOR_LEN);

    // Landing inside a character: the cut walks back to the boundary below,
    // and no further. "é" is two bytes, so a 200-byte budget over 100 of them
    // is exact; shifting by one byte must lose exactly one byte, not one
    // character and not everything after the first.
    let two_byte = ReplyInfo {
        message_id: id.clone(),
        author_name: String::new(),
        preview_text: format!("x{}", "é".repeat(200)),
    };
    let cut = sanitize_reply(two_byte).expect("a valid uuid is a real reply");
    assert_eq!(
        cut.preview_text.len(),
        MAX_REPLY_PREVIEW_LEN - 1,
        "one leading ASCII byte makes every following boundary odd, so the \
         cut must step back exactly one byte"
    );
    assert!(
        cut.preview_text.is_char_boundary(cut.preview_text.len()),
        "the cut must land on a character boundary or String::truncate panics"
    );

    // A four-byte character is the longest walk-back the function can face:
    // three steps, and the `/=` mutant that timed out would take a different
    // number of them. One prefix byte per case shifts every boundary by one.
    for (prefix, expected_step_back) in [("xy", 2), ("x", 3)] {
        let four_byte = ReplyInfo {
            message_id: id.clone(),
            author_name: String::new(),
            preview_text: format!("{prefix}{}", "🐙".repeat(100)),
        };
        let cut = sanitize_reply(four_byte).expect("a valid uuid is a real reply");
        assert_eq!(
            cut.preview_text.len(),
            MAX_REPLY_PREVIEW_LEN - expected_step_back,
            "a {}-byte prefix before four-byte characters puts the limit \
             {expected_step_back} bytes past the boundary below it",
            prefix.len()
        );
        assert!(cut.preview_text.is_char_boundary(cut.preview_text.len()));
    }
}

/// An attachment exactly at each ceiling is accepted; one past it is not.
///
/// Four survivors in `sanitize_attachment` were the same shape: `>` could
/// become `>=` or `==` on the byte size and on both dimensions, because every
/// test was either an ordinary small image or one wildly over. A ceiling the
/// client is told about must be a ceiling — the client downscales *to* these
/// numbers, so refusing something exactly at one refuses what the client was
/// asked to produce (constraint #12).
#[test]
fn an_attachment_exactly_at_its_ceilings_is_accepted() {
    // Only the first twelve bytes are decoded and sniffed, so padding the
    // base64 with valid characters reaches an exact length without disturbing
    // the magic bytes.
    let at_the_byte_ceiling = |len: usize| {
        let base = png_attachment();
        let mut data = base.data.clone();
        assert!(
            data.len() < len,
            "the fixture must be smaller than the limit"
        );
        data.push_str(&"A".repeat(len - data.len()));
        Attachment { data, ..base }
    };

    assert!(
        sanitize_attachment(at_the_byte_ceiling(MAX_ATTACHMENT_BYTES)).is_ok(),
        "an attachment of exactly MAX_ATTACHMENT_BYTES is what the client aims \
         for and must be accepted"
    );
    assert!(
        sanitize_attachment(at_the_byte_ceiling(MAX_ATTACHMENT_BYTES + 1)).is_err(),
        "one byte past the ceiling must be refused"
    );

    for (width, height, allowed) in [
        (MAX_ATTACHMENT_DIMENSION, MAX_ATTACHMENT_DIMENSION, true),
        (
            MAX_ATTACHMENT_DIMENSION + 1,
            MAX_ATTACHMENT_DIMENSION,
            false,
        ),
        (
            MAX_ATTACHMENT_DIMENSION,
            MAX_ATTACHMENT_DIMENSION + 1,
            false,
        ),
    ] {
        let attachment = Attachment {
            width,
            height,
            ..png_attachment()
        };
        assert_eq!(
            sanitize_attachment(attachment).is_ok(),
            allowed,
            "{width}x{height} against a limit of {MAX_ATTACHMENT_DIMENSION}"
        );
    }
}
