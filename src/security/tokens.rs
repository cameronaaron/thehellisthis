//! The constant-time bearer/Basic credential check `/metrics` and `/admin`
//! share.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// Byte equality that takes the same time wherever the first difference is.
/// `==` short-circuits, so a closer guess compares measurably slower and leaks
/// the secret a byte at a time. OR-ing every difference and deciding once
/// visits every byte of the shorter input. The length check leaks only the
/// length, which an `Authorization` header doesn't hide anyway.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether a request may see `/metrics`.
///
/// It names nothing private but is operational disclosure: counts an attacker
/// could watch to time an attempt against `MAX_CONCURRENT_USERS`. Cloudflare
/// terminates in front, so there's no internal network boundary to rely on.
///
/// Fails closed: without `METRICS_TOKEN`, always no. An unset and a wrong
/// token both read as 404, not 401, so the route isn't confirmed to a wrong
/// guess.
pub fn is_authorized_for_metrics(headers: &axum::http::HeaderMap) -> bool {
    is_authorized_by_env_token(headers, "METRICS_TOKEN", |v| {
        v.strip_prefix("Bearer ").map(str::to_string)
    })
}

/// Whether a `/ws/nova-operator` upgrade may proceed. An operator holds a real
/// DKG share, worth more than anything `/metrics` or `/admin` guard, so this
/// fails closed the same way without `NOVA_OPERATOR_TOKEN`. One shared token
/// for now; per-operator tokens would allow individual revocation but need
/// state (which token is whose) this demo doesn't.
pub fn is_authorized_for_nova_operator(headers: &axum::http::HeaderMap) -> bool {
    is_authorized_by_env_token(headers, "NOVA_OPERATOR_TOKEN", |v| {
        v.strip_prefix("Bearer ").map(str::to_string)
    })
}

/// The shape the token checks share: read the expected value from `env_var`,
/// get the request's credential with `extract`, compare in constant time.
/// Fails closed without `env_var`, never calling `extract`, the one part that
/// differs (a bearer token versus a Basic password).
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

/// Whether a request may see `/admin`.
///
/// It names every open room, a larger disclosure than `/metrics`, so it has
/// its own `ADMIN_TOKEN` and one leak doesn't open both. It answers failures
/// differently on purpose: a person opens it from a bookmark, so a refusal is a
/// real 401 with `WWW-Authenticate: Basic`, which makes the browser prompt
/// (and remember Basic credentials, where a link can't carry a header). Still
/// fails closed without `ADMIN_TOKEN`, and compares in constant time.
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
