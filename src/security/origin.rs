//! The WebSocket handshake's own same-origin check.
//!
//! Upgrades are not covered by the browser's same-origin policy, so this is
//! the one place that enforcement has to be the server's own.

/// Whether a WebSocket handshake carrying this `Origin` should be accepted.
///
/// **WebSocket upgrades are not covered by the same-origin policy.** Any page
/// on the internet can open a socket to this server; the browser sends an
/// `Origin` header and leaves the decision to the server. Without this check a
/// third-party site can embed the chat's traffic, drive messages from a
/// visitor's browser, and consume the room and connection budgets.
///
/// A missing `Origin` is allowed: non-browser clients (the integration tests,
/// `websocat`, monitoring) do not send one, and rejecting them would break the
/// test suite without stopping any browser-based attack, since browsers always
/// send it.
pub fn is_allowed_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };

    let Some(origin_host) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        // Anything that is not an http(s) origin (`null`, a file:, an
        // extension) is not this application.
        return false;
    };

    // Compare host-only, ignoring the port: the container is reached on :3000
    // behind a Worker serving :443, so the ports legitimately differ.
    let origin_host = origin_host.split('/').next().unwrap_or(origin_host);
    let origin_name = origin_host.split(':').next().unwrap_or(origin_host);

    match host.map(|h| h.split(':').next().unwrap_or(h)) {
        Some(request_host) => origin_name.eq_ignore_ascii_case(request_host),
        // No Host header to compare against: fall back to the known domains.
        None => is_known_domain(origin_name),
    }
}

fn is_known_domain(host: &str) -> bool {
    const KNOWN: &[&str] = &["thehellisthis.com", "www.thehellisthis.com", "localhost"];
    KNOWN.iter().any(|k| host.eq_ignore_ascii_case(k)) || host == "127.0.0.1"
}
