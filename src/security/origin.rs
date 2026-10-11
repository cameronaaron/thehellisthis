//! The WebSocket handshake's same-origin check. Upgrades aren't covered by the
//! browser's same-origin policy, so the server has to enforce it.

/// Whether to accept a handshake with this `Origin`.
///
/// Any page can open a socket here; the browser sends `Origin` and leaves the
/// decision to the server. Without this a third-party site could drive
/// messages from a visitor's browser and spend the room and connection
/// budgets.
///
/// A missing `Origin` is allowed: non-browser clients (the tests, `websocat`,
/// monitoring) don't send one, and browsers always do, so rejecting it would
/// stop no browser attack.
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
