//! The constant-time bearer/Basic credential check `/metrics` and `/admin`
//! share.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// Byte-for-byte equality that takes the same time regardless of where the
/// first difference falls.
///
/// `==` on `&[u8]` short-circuits at the first mismatch, so comparing a
/// guessed token against the real one byte-by-byte is measurably faster for a
/// closer guess — a timing side channel an attacker can use to recover a
/// secret one byte at a time without ever seeing it. `bitwise-OR every
/// difference, decide once` visits every byte of the shorter input every time
/// regardless of where a mismatch is, so there is nothing for a timing
/// measurement to distinguish. The length check that follows leaks only the
/// length, which the token's transport (an `Authorization` header, sized
/// however the client likes) already does not hide.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether a request to `/metrics` may see it.
///
/// `/metrics` names nothing private — no room name, no message, no IP — but it
/// is still real operational disclosure: room and connection counts an
/// attacker could watch to infer traffic patterns or time an attempt against
/// `MAX_CONCURRENT_USERS`. Cloudflare terminates in front of this server, so
/// there is no network boundary to rely on here the way there might be for an
/// internal-only Prometheus target — the check has to be the server's own.
///
/// Fails closed: with no `METRICS_TOKEN` set, the answer is always no, so an
/// operator who forgets to configure one gets an endpoint that behaves as
/// though it does not exist rather than one that is silently public. There is
/// deliberately no distinction in the response between "not configured" and
/// "wrong token" — both read as a 404, not a 401, so the route's very
/// existence is not confirmed to a request that guessed wrong.
pub fn is_authorized_for_metrics(headers: &axum::http::HeaderMap) -> bool {
    is_authorized_by_env_token(headers, "METRICS_TOKEN", |v| {
        v.strip_prefix("Bearer ").map(str::to_string)
    })
}

/// The shape both token checks share: read the expected value from `env_var`,
/// pull whatever credential the request actually sent out of its
/// `Authorization` header via `extract`, compare in constant time. Fails
/// closed if `env_var` is unset — `extract` is never even called.
///
/// `extract` is the one thing that differs between the two callers (a bearer
/// token vs. a Basic-encoded password), so it is the one thing passed in
/// rather than duplicated.
fn is_authorized_by_env_token(
    headers: &axum::http::HeaderMap,
    env_var: &str,
    extract: impl FnOnce(&str) -> Option<String>,
) -> bool {
    let Ok(expected) = std::env::var(env_var) else {
        return false;
    };

    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract)
        .is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes()))
}

/// Whether a request to the admin dashboard (`/admin`) may see it.
///
/// This is a different, larger disclosure than `/metrics`: the dashboard names
/// every open room, which `/metrics` deliberately does not (see its own doc
/// comment). It therefore carries its own token, `ADMIN_TOKEN`, rather than
/// reusing `METRICS_TOKEN` — a leak of one must not compromise the other.
///
/// It also deliberately answers differently on failure. `/metrics` is scraped
/// by machines, so it 404s uniformly to keep the route's existence
/// unconfirmed. `/admin` is a page a human opens by clicking a bookmarked
/// link, so an unauthorized request gets a real `401` with
/// `WWW-Authenticate: Basic` — the response the browser needs to raise its
/// native password prompt. HTTP Basic rather than a bearer header for the same
/// reason: a header cannot be attached by clicking a link, but a browser will
/// prompt for and remember Basic credentials against an origin.
///
/// Still fails closed: with no `ADMIN_TOKEN` set, every request is refused,
/// whatever credentials it carries. The comparison is constant-time for the
/// same reason as the metrics token above.
pub fn is_authorized_for_admin(headers: &axum::http::HeaderMap) -> bool {
    is_authorized_by_env_token(headers, "ADMIN_TOKEN", |v| {
        v.strip_prefix("Basic ")
            .and_then(|encoded| BASE64.decode(encoded).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|credentials| {
                credentials
                    .split_once(':')
                    .map(|(_, pass)| pass.to_string())
            })
    })
}
