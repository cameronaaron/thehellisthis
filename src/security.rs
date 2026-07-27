//! Transport-level hardening: the response headers every request gets, and the
//! origin check on the WebSocket handshake.
//!
//! The server previously sent no security headers at all. Cloudflare adds none
//! on its own for a Worker-proxied origin, so the browser was applying nothing
//! but its defaults to a page that renders user-submitted Markdown as HTML.

use axum::http::{HeaderName, HeaderValue};
use tower_http::set_header::SetResponseHeaderLayer;

/// Content Security Policy.
///
/// `script-src 'self'` is the directive that earns its place. The message
/// pipeline turns user input into HTML, so if the sanitiser is ever bypassed,
/// the only remaining defence is that injected script cannot run. That
/// directive is why the client's JavaScript was moved out of the page into
/// `/app.js`: while it was an inline `<script>` block, any workable policy had
/// to include `'unsafe-inline'`, which grants exactly the capability an
/// injected `<script>` needs.
///
/// `'unsafe-inline'` remains for **styles**, where the client sets a few inline
/// `style` attributes and the exposure is CSS, not execution.
///
/// `style-src`/`font-src` allow Google Fonts because the page loads its
/// typeface and icon font from there; everything else is same-origin.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
     font-src 'self' https://fonts.gstatic.com; \
     img-src 'self' data:; \
     connect-src 'self' ws: wss:; \
     frame-ancestors 'none'; \
     base-uri 'none'; \
     form-action 'none'; \
     object-src 'none'";

/// Headers applied to every response.
///
/// Each one is here because something concrete goes wrong without it, noted
/// alongside.
const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("content-security-policy", CONTENT_SECURITY_POLICY),
    // The chat is not a frameable widget; clickjacking it into a transparent
    // overlay would let another site harvest what a user types.
    ("x-frame-options", "DENY"),
    // The page serves user-influenced bytes; MIME sniffing could turn a
    // response the server labels text/plain into something executable.
    ("x-content-type-options", "nosniff"),
    // Room names are in the URL and can be private-ish; do not leak the full
    // path to the font CDNs or to anything a user links out to.
    ("referrer-policy", "strict-origin-when-cross-origin"),
    // Nothing here needs hardware access.
    (
        "permissions-policy",
        "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()",
    ),
    // Identity cookies are Secure; without HSTS the first request over http is
    // still an opportunity to strip TLS.
    (
        "strict-transport-security",
        "max-age=31536000; includeSubDomains; preload",
    ),
    ("cross-origin-opener-policy", "same-origin"),
    ("x-permitted-cross-domain-policies", "none"),
];

/// Builds the layers that stamp [`SECURITY_HEADERS`] onto every response.
///
/// `SetResponseHeaderLayer::overriding` rather than `if_not_present`: a header
/// this server considers a security control must not be weakenable by anything
/// downstream setting it first.
pub fn security_header_layers() -> Vec<SetResponseHeaderLayer<HeaderValue>> {
    SECURITY_HEADERS
        .iter()
        .map(|(name, value)| {
            SetResponseHeaderLayer::overriding(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            )
        })
        .collect()
}

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
