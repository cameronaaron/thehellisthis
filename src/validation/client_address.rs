//! Where a connection actually came from, and the opaque form the rest of the
//! server is allowed to know it by.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::extract::ConnectInfo;
use http::HeaderMap;

/// Keyed hasher for client addresses: SipHash-1-3 (`RandomState`) with keys
/// drawn at startup that never leave the process, so digests can't be
/// correlated across restarts or reversed by enumerating IPv4 against an
/// unkeyed hash.
static ADDRESS_HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);

/// A client address reduced to an opaque identifier, stable per process.
///
/// The limiter, pool and ban list only compare for equality, so the raw address
/// exists only as a local in [`extract_client_ip`]'s caller and is never
/// stored, logged or kept in a map; a memory dump holds no visitor addresses.
/// A privacy measure: per-address limits remain a courtesy bound (§5.8).
pub fn hash_client_address(ip: &str) -> String {
    format!("{:016x}", ADDRESS_HASHER.hash_one(ip))
}

/// A header's trimmed value, or `None` if absent, not UTF-8 or blank: one
/// definition of "no address here" instead of three inline copies.
fn nonempty_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let trimmed = headers.get(name).and_then(|v| v.to_str().ok())?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `X-Forwarded-For`'s first entry (the client, by convention), checked like
/// [`nonempty_header`].
fn first_forwarded_ip(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())?;
    let first = raw.split(',').next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// The client's address, preferring proxy headers.
///
/// Cloudflare terminates in front of the container, so the socket address is
/// always the proxy. `CF-Connecting-IP` comes first: Cloudflare sets it on every
/// proxied request and won't let a client forge it. Reading only
/// `X-Forwarded-For`, as this once did, made every visitor one address and the
/// per-address limits global.
///
/// All of these are attacker-controlled if the container is reached directly;
/// the per-connection and global ceilings are the real protection.
pub fn extract_client_ip(
    headers: &HeaderMap,
    conn_info: Option<&ConnectInfo<SocketAddr>>,
) -> Option<String> {
    nonempty_header(headers, "cf-connecting-ip")
        .or_else(|| first_forwarded_ip(headers))
        .or_else(|| nonempty_header(headers, "x-real-ip"))
        .or_else(|| conn_info.map(|ci| ci.0.ip().to_string()))
}
