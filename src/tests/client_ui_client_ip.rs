//! The real client address: extracting it from Cloudflare's headers, and
//! hashing it at the boundary before it is stored anywhere (§5.6).

use super::*;

#[tokio::test]
async fn test_extract_client_ip_from_x_forwarded_for() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        "203.0.113.195, 70.41.3.18, 150.172.238.178"
            .parse()
            .unwrap(),
    );

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("203.0.113.195".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_from_x_real_ip() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "192.168.1.100".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("192.168.1.100".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_empty_x_forwarded_for() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    // Should fallback to None since no conn_info provided
    assert!(ip.is_none());
}

#[tokio::test]
async fn test_extract_client_ip_priority() {
    use axum::http::HeaderMap;

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "10.0.0.1".parse().unwrap());
    headers.insert("x-real-ip", "10.0.0.2".parse().unwrap());

    // x-forwarded-for should take priority
    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("10.0.0.1".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_with_multiple_x_forwarded_for() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        "10.0.0.1, 10.0.0.2, 10.0.0.3".parse().unwrap(),
    );

    let ip = extract_client_ip(&headers, None);

    // Should return the first IP
    assert_eq!(ip, Some("10.0.0.1".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_all_sources() {
    use axum::extract::ConnectInfo;
    use axum::http::HeaderMap;
    use std::net::SocketAddr;

    // Test X-Forwarded-For priority
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
    headers.insert("x-real-ip", "9.10.11.12".parse().unwrap());
    let conn_info = ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345)));

    let ip = extract_client_ip(&headers, Some(&conn_info));
    assert_eq!(
        ip,
        Some("1.2.3.4".to_string()),
        "Should prefer X-Forwarded-For"
    );

    // Test X-Real-IP when no X-Forwarded-For
    let mut headers2 = HeaderMap::new();
    headers2.insert("x-real-ip", "9.10.11.12".parse().unwrap());
    let ip2 = extract_client_ip(&headers2, Some(&conn_info));
    assert_eq!(ip2, Some("9.10.11.12".to_string()), "Should use X-Real-IP");

    // Test fallback to ConnectInfo
    let headers3 = HeaderMap::new();
    let ip3 = extract_client_ip(&headers3, Some(&conn_info));
    assert_eq!(
        ip3,
        Some("127.0.0.1".to_string()),
        "Should fall back to ConnectInfo"
    );

    // Test no IP available
    let ip4 = extract_client_ip(&headers3, None);
    assert!(
        ip4.is_none(),
        "Should return None when no IP source available"
    );
}

#[tokio::test]
async fn test_extract_client_ip_x_forwarded_for() {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "192.168.1.1, 10.0.0.1".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("192.168.1.1".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_x_real_ip() {
    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "192.168.2.2".parse().unwrap());

    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, Some("192.168.2.2".to_string()));
}

#[tokio::test]
async fn test_extract_client_ip_fallback_to_conn_info() {
    let headers = HeaderMap::new();

    // When no headers are present, it falls back to connection info
    let ip = extract_client_ip(&headers, None);
    assert_eq!(ip, None); // No connection info provided

    // With connection info
    let addr: SocketAddr = "192.168.3.3:12345".parse().unwrap();
    let conn_info = ConnectInfo(addr);
    let ip = extract_client_ip(&headers, Some(&conn_info));
    assert_eq!(ip, Some("192.168.3.3".to_string()));
}

// Test ConnectionPool cleanup_stale behavior - covers lines 430-432

/// The client address must be read from the header Cloudflare actually sets.
///
/// The server read only `X-Forwarded-For`, which Cloudflare does not set for a
/// Worker-proxied container request. Every visitor therefore arrived as the
/// same address, which quietly turned MAX_CONCURRENT_CONNECTIONS_PER_IP into a
/// global limit of 3 rather than a per-IP one — a correctness bug in both
/// directions: real users blocked each other, and one abuser was never isolated.
#[test]
fn client_address_prefers_the_header_cloudflare_sets() {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
    headers.insert("x-forwarded-for", "198.51.100.1, 10.0.0.1".parse().unwrap());

    assert_eq!(
        extract_client_ip(&headers, None).as_deref(),
        Some("203.0.113.7"),
        "CF-Connecting-IP is the trustworthy one behind Cloudflare"
    );

    // Without it, the X-Forwarded-For chain is still honoured, client first.
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "198.51.100.1, 10.0.0.1".parse().unwrap());
    assert_eq!(
        extract_client_ip(&headers, None).as_deref(),
        Some("198.51.100.1")
    );
}

/// The Worker must forward the client address to the container.
///
/// The Rust side reading the right header only helps if the Worker passes it
/// on; these two halves are one behaviour and neither is useful alone.
#[test]
fn worker_forwards_the_client_address() {
    const WORKER: &str = include_str!("../../cloudflare/src/index.ts");

    assert!(
        WORKER.contains("CF-Connecting-IP"),
        "the worker must forward the client address to the container"
    );
    assert!(
        WORKER.contains("X-Forwarded-For"),
        "the worker should also set X-Forwarded-For for the fallback path"
    );
    assert!(
        WORKER.contains("caches.default"),
        "the immutable script should be served from the edge cache"
    );
}

// ========== COVERAGE: PREVIOUSLY UNEXERCISED BRANCHES ==========
//
// Each test here targets a branch the suite never reached. They are grouped
// because they were written together, from a coverage report, but each asserts
// a real behaviour rather than merely visiting a line.

/// Client addresses are stored as opaque digests, never in the clear.
///
/// The rate limiter, connection pool and ban list only ever compare addresses
/// for equality, so none of them needs the real value. Hashing at the boundary
/// means a memory dump of a running server yields no visitor addresses — which
/// matters for a site whose entire premise is that you get an animal name
/// instead of an account.
#[test]
fn client_addresses_are_hashed_not_stored_in_the_clear() {
    let address = "203.0.113.42";
    let digest = hash_client_address(address);

    assert_ne!(digest, address);
    assert!(
        !digest.contains("203") && !digest.contains("113"),
        "the digest must not carry the address it came from: {digest}"
    );

    // Stable within a process, so it works as a map key.
    assert_eq!(digest, hash_client_address(address));
    // Distinct addresses stay distinct.
    assert_ne!(digest, hash_client_address("203.0.113.43"));
}
