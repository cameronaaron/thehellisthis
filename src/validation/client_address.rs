//! Where a connection actually came from, and the opaque form the rest of the
//! server is allowed to know it by.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::extract::ConnectInfo;
use http::HeaderMap;

/// Keyed hasher for client addresses, seeded once per process.
///
/// `RandomState` is SipHash-1-3 with keys drawn at startup. The keys never
/// leave this process and change on every restart, so the stored digests are
/// not correlatable across restarts or against a precomputed table — which
/// matters, because the IPv4 space is small enough to enumerate against an
/// unkeyed hash.
static ADDRESS_HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);

/// A client address reduced to an opaque, stable-per-process identifier.
///
/// The rate limiter, connection pool and ban list only ever compare addresses
/// for equality — none of them needs to know the actual address. Hashing at the
/// boundary means the raw address exists only as a local in
/// [`extract_client_ip`]'s caller and is never stored, logged, or held in any
/// map: a memory dump of a running server yields no visitor addresses.
///
/// This is a privacy decision, not a security one. Per-IP limits remain a
/// courtesy bound (§5.6) — hashing changes nothing about their strength.
pub fn hash_client_address(ip: &str) -> String {
    format!("{:016x}", ADDRESS_HASHER.hash_one(ip))
}

/// A header's value, trimmed, or `None` if absent, not valid UTF-8, or blank.
///
/// The same three-way emptiness check three of `extract_client_ip`'s sources
/// used to repeat inline, once each — pulled out so there is one place that
/// decides what "no address here" means, not three copies that could drift.
fn nonempty_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let trimmed = headers.get(name).and_then(|v| v.to_str().ok())?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `X-Forwarded-For`'s first entry — the client, per the header's own
/// left-to-right convention — trimmed and checked the same way
/// [`nonempty_header`] checks a single-valued header.
fn first_forwarded_ip(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())?;
    let first = raw.split(',').next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// The client's address, preferring proxy headers.
///
/// Cloudflare terminates TLS in front of the container, so the socket address
/// is the proxy for every request and the real client only appears in a header.
///
/// `CF-Connecting-IP` is checked first because that is what actually arrives in
/// production: Cloudflare sets it on every proxied request, and it is the one
/// header the edge will not let a client forge. Checking only
/// `X-Forwarded-For`, as this used to, meant every visitor looked like the same
/// address to the per-IP limits — so those limits were, in effect, global.
///
/// All of these are attacker-controlled if the container is ever reached
/// directly, which is why per-IP limits are a courtesy bound and the
/// per-connection and global ceilings are the real protection.
pub fn extract_client_ip(
    headers: &HeaderMap,
    conn_info: Option<&ConnectInfo<SocketAddr>>,
) -> Option<String> {
    nonempty_header(headers, "cf-connecting-ip")
        .or_else(|| first_forwarded_ip(headers))
        .or_else(|| nonempty_header(headers, "x-real-ip"))
        .or_else(|| conn_info.map(|ci| ci.0.ip().to_string()))
}
