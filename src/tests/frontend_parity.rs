//! Frontend/backend parity: every constant the client mirrors from the
//! server, checked against the value it is mirroring — not against itself.
//!
//! §7.3/§7.4 happened twice in one session from the same root cause: a number
//! in `client.js` that was right when it was written and silently stopped
//! being right when the backend constant it was supposed to track moved.
//! `PARITY` is the fix as a mechanism rather than a habit — a table of every
//! scalar the client mirrors, and the one test that walks it. Adding a new
//! mirrored constant is a line in this table, not a bespoke test to remember
//! to write; a constant not in this table, and not covered by one of the
//! specialised parity tests elsewhere (`client_attachment_ceiling_matches_the_server`,
//! `client_reaction_roster_matches_the_server`,
//! `client_and_server_agree_on_the_idle_close_code`, each covering something
//! richer than a single scalar), has no source of truth checking it at all.

use super::*;

/// What a mirrored JS constant is expected to equal.
enum Expected {
    Number(u64),
    Text(&'static str),
}

/// One constant the client is not allowed to invent independently.
struct Mirrored {
    /// The `const NAME` as it appears in `client.js`.
    js_const: &'static str,
    expected: Expected,
    /// Why this exists on both sides at all — what breaks if they drift.
    reason: &'static str,
}

/// Reads a `const NAME = value;` declaration's right-hand side out of
/// `client.js`, stopping at the first `;`.
///
/// Looks for the exact declaration, not merely the name, so a constant that
/// is *referenced* (e.g. in a doc comment, per §7.3b) without being declared
/// is correctly reported as absent rather than matched against prose.
fn js_const_rhs(js: &str, name: &str) -> Option<String> {
    let marker = format!("const {name} = ");
    let index = js.find(&marker)?;
    let rest = &js[index + marker.len()..];
    let end = rest.find(';')?;
    Some(rest[..end].trim().to_string())
}

/// Every scalar the client mirrors from the server, and what it must equal.
///
/// This is the table to extend. A number here with no matching declaration in
/// `client.js` is caught the same as a declaration with the wrong number —
/// either way, `every_mirrored_constant_matches_its_source_of_truth` fails
/// with the name to go add or fix.
fn parity_table() -> Vec<Mirrored> {
    vec![
        Mirrored {
            js_const: "MAIN_ROOM_NAME",
            expected: Expected::Text(MAIN_ROOM),
            reason: "the client picks which fade indicator to show by \
                     comparing its own room name against this literal (§7.4)",
        },
        Mirrored {
            js_const: "MAIN_ROOM_FADE_SECONDS",
            expected: Expected::Number(MAIN_ROOM_FADE_IDLE.as_secs()),
            reason: "main's fade warning is computed as a fraction of this; \
                     wrong, and the warning fires at the wrong point in a \
                     thirty-minute window (§7.3a, §7.4)",
        },
        Mirrored {
            js_const: "IDLE_EVICTION_SECONDS",
            expected: Expected::Number(USER_IDLE_MESSAGE_TIMEOUT.as_secs()),
            reason: "a spawned room's fade warning is computed as a fraction \
                     of this instead of a room-wide clock, because that room \
                     cannot be deleted while its viewer is connected to it \
                     (constraint #3, §7.4)",
        },
        Mirrored {
            js_const: "IDLE_CLOSE_CODE",
            expected: Expected::Number(u64::from(IDLE_CLOSE_CODE)),
            reason: "without agreement the client cannot tell an idle \
                     eviction from a dropped connection and reconnects into \
                     the room it was just removed from — the exact bug that \
                     stopped rooms ever fading (constraint #3)",
        },
        Mirrored {
            js_const: "MAX_ATTACHMENT_BYTES",
            expected: Expected::Number(MAX_ATTACHMENT_BYTES as u64),
            reason: "the client's downscale loop targets this ceiling; too \
                     high and the server rejects an image the client thought \
                     was safe, after the user has waited for it to encode",
        },
    ]
}

/// No frontend constant drifts from the backend value it exists to mirror.
///
/// Proven to fail: shift `MAIN_ROOM_FADE_IDLE` or `USER_IDLE_MESSAGE_TIMEOUT`
/// by one second without touching `client.js` and this is the test that
/// catches it, by construction rather than by someone remembering to check.
#[test]
fn every_mirrored_constant_matches_its_source_of_truth() {
    let mut wrong: Vec<String> = Vec::new();

    for entry in parity_table() {
        let Some(actual) = js_const_rhs(EMBEDDED_JS, entry.js_const) else {
            wrong.push(format!(
                "{} is not declared in client.js at all ({})",
                entry.js_const, entry.reason
            ));
            continue;
        };

        let matches = match entry.expected {
            Expected::Number(n) => actual == n.to_string(),
            Expected::Text(s) => actual == format!("'{s}'") || actual == format!("\"{s}\""),
        };

        if !matches {
            let expected_display = match entry.expected {
                Expected::Number(n) => n.to_string(),
                Expected::Text(s) => s.to_string(),
            };
            wrong.push(format!(
                "{} = {actual} in client.js, but the backend value is \
                 {expected_display} ({})",
                entry.js_const, entry.reason
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "these client.js constants have drifted from the backend value they \
         mirror:\n{}",
        wrong.join("\n")
    );
}

/// The extraction helper itself is exact, not merely lucky on today's file.
///
/// A raw substring search for `const {name} = ` would find `const
/// MAIN_ROOM_FADE_SECONDS = ` inside a longer name like `const
/// SOME_MAIN_ROOM_FADE_SECONDS_LIMIT = ` if one ever existed — pinned here so
/// a future rename cannot silently make the parity check above read the wrong
/// declaration.
#[test]
fn js_const_rhs_reads_the_declaration_not_a_substring_of_a_longer_one() {
    let js = "const FOO = 1;\nconst LONGER_FOO_NAME = 2;\nconst FOO_BAR = 3;";
    assert_eq!(js_const_rhs(js, "FOO").as_deref(), Some("1"));
    assert_eq!(js_const_rhs(js, "LONGER_FOO_NAME").as_deref(), Some("2"));
    assert_eq!(js_const_rhs(js, "FOO_BAR").as_deref(), Some("3"));
    assert_eq!(js_const_rhs(js, "NOT_PRESENT"), None);
}

/// The welcome banner's "go silent for X and it fades too" describes `main`
/// fading, not a spawned room being deleted — it was checked against
/// `EMPTY_ROOM_CLEANUP_DELAY` anyway, which only ever passed because the two
/// constants happened to be equal (both 600s) before §7.4 gave them different
/// values on purpose. The same class of bug as §7.3a's fade indicator, in the
/// other direction: not a wrong number, a right number checked against the
/// wrong constant.
#[tokio::test]
async fn test_frontend_timeout_text_matches_backend_constant() {
    let fade_minutes = MAIN_ROOM_FADE_IDLE.as_secs() / 60;

    assert_eq!(
        fade_minutes, 30,
        "MAIN_ROOM_FADE_IDLE changed! Update the welcome banner text to match."
    );

    assert!(
        SHIPPED_CLIENT.contains("go silent for thirty minute"),
        "Frontend instructions don't match backend! main fades after {fade_minutes} \
         minutes but the banner doesn't say 'thirty minute'."
    );

    assert!(
        !SHIPPED_CLIENT.contains("go silent for one minute")
            && !SHIPPED_CLIENT.contains("go silent for ten minute"),
        "Frontend still contains outdated fade-timing text!"
    );
}

#[tokio::test]
async fn test_all_timing_constants_are_consistent() {
    // Meta-test: Verify all timing relationships make sense together

    // Heartbeat should be sent more frequently than timeout
    assert!(
        HEARTBEAT_INTERVAL < HEARTBEAT_TIMEOUT,
        "HEARTBEAT_INTERVAL must be less than HEARTBEAT_TIMEOUT"
    );

    // Cleanup interval should allow catching inactive rooms
    assert!(
        ROOM_CLEANUP_INTERVAL <= EMPTY_ROOM_CLEANUP_DELAY,
        "Cleanup interval must be <= delay to catch rooms"
    );

    // Message rate limit window should be reasonable
    assert!(
        RATE_LIMIT_WINDOW >= Duration::from_secs(30),
        "Rate limit window too short"
    );
    assert!(
        RATE_LIMIT_WINDOW <= Duration::from_secs(120),
        "Rate limit window too long"
    );

    // Inactive timeout should be much longer than room cleanup
    assert!(
        INACTIVE_TIMEOUT > EMPTY_ROOM_CLEANUP_DELAY,
        "User inactive timeout should exceed room cleanup delay"
    );
}

/// `client.js` declares every constant it uses that looks like it should be
/// one — SCREAMING_SNAKE_CASE is this codebase's convention for a named
/// value, so a token shaped like one that is only ever *mentioned* (a stale
/// cross-reference in a comment, a copy-pasted name) is exactly the kind of
/// dummy value this file exists to rule out.
#[test]
fn the_client_declares_every_screaming_case_constant_it_uses() {
    // SCREAMING_SNAKE identifiers are this file's convention for module-level
    // constants, which makes them the set worth checking mechanically.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();

    let bytes = EMBEDDED_JS.as_bytes();
    let mut start = None;
    for (i, &c) in bytes.iter().enumerate() {
        let wordish = c.is_ascii_alphanumeric() || c == b'_';
        if wordish && start.is_none() {
            start = Some(i);
        } else if !wordish && let Some(s) = start.take() {
            let word = &EMBEDDED_JS[s..i];
            if word.len() > 3
                && word.contains('_')
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                used.insert(word.to_string());
            }
        }
    }

    let undeclared: Vec<&String> = used
        .iter()
        .filter(|name| !EMBEDDED_JS.contains(&format!("const {name}")))
        .collect();

    assert!(
        undeclared.is_empty(),
        "client.js uses constants it never declares: {undeclared:?}"
    );
}
