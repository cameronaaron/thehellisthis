//! Transport-level hardening: the response headers every request gets, and the
//! origin check on the WebSocket handshake.
//!
//! The server previously sent no security headers at all. Cloudflare adds none
//! on its own for a Worker-proxied origin, so the browser was applying nothing
//! but its defaults to a page that renders user-submitted Markdown as HTML.

use std::sync::LazyLock;

use axum::http::{HeaderName, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};
use tower_http::set_header::SetResponseHeaderLayer;

/// The page's one `<style>` block, exactly as a browser will read it — the
/// text between the tags, not including them.
///
/// A CSP hash source is computed over this exact substring, so this has to
/// extract precisely what the browser's HTML parser treats as the element's
/// text content. There is exactly one `<style>` element in the page
/// (`the_page_has_exactly_one_style_block` — a second one would be hashed
/// into a name nothing points at, silently unstyled) so a single
/// find-the-tags extraction is unambiguous.
fn embedded_style_block() -> &'static str {
    let html = crate::routes::CLIENT_HTML;
    let start = html
        .find("<style>")
        .map(|i| i + "<style>".len())
        .expect("the page has a <style> tag");
    let end = html[start..]
        .find("</style>")
        .map(|i| start + i)
        .expect("the <style> tag is closed");
    &html[start..end]
}

/// `'sha256-<base64>'`, computed once, over the exact bytes of
/// [`embedded_style_block`].
static STYLE_BLOCK_HASH: LazyLock<String> = LazyLock::new(|| {
    let digest = Sha256::digest(embedded_style_block().as_bytes());
    format!("'sha256-{}'", BASE64.encode(digest))
});

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
/// `style-src` carries no `'unsafe-inline'` either, and takes the same
/// approach `script-src` already established rather than a new one: name the
/// one inline surface by its exact content hash instead of granting a
/// capability to *any* inline style. Two things had to be true first. Every
/// style the client sets at runtime is a direct CSSOM property assignment —
/// `element.style.top = '4px'` — which `style-src` was never restricting in
/// the first place; only `<style>` elements and `style=""` attributes are.
/// Verified in a real browser with a `securitypolicyviolation` listener
/// across every call site (the welcome banner's dismiss, the typing
/// indicator, an attachment's reserved aspect-ratio box, the reaction bar's
/// position): zero violations, and none of those paths needed the hash.
/// The one thing that *did* need it was the page's own embedded stylesheet —
/// the entire CSS for the page is one `<style>` block in `index.html`, found
/// by actually loading the page with `'unsafe-inline'` removed and nothing
/// else, which broke every rule on the page until the hash was added back.
/// The hash is [`STYLE_BLOCK_HASH`], computed at startup so it can never drift
/// from the content it names — hand-copying a hash into a string constant is
/// exactly the kind of number that goes stale the next time someone edits the
/// stylesheet, silently, since a wrong hash just fails closed with no visible
/// error beyond unstyled HTML.
///
/// `cross-origin-resource-policy` is `same-origin`: nothing this page serves
/// is meant to be embedded as a sub-resource by another origin, the same
/// reasoning as `frame-ancestors 'none'` one layer lower. `require-corp` for
/// `cross-origin-embedder-policy` was tried alongside it and rejected — see
/// §9.4: it broke the WebSocket connection under a Private Network Access
/// interaction in local testing, and the connection this server exists to
/// serve was not a trade worth making without being able to verify the
/// production origin directly.
///
/// Every other directive is `'self'` or `'none'`. There are no third-party
/// origins at all: the page used to load its typeface and icon font from
/// Google, which both widened this policy and told a third party the address of
/// every visitor and the room they opened. Icons are now an inline SVG sprite
/// and text uses the system stack, so `font-src 'none'` is achievable rather
/// than aspirational — the browser is told, in the policy itself, that this
/// page has no business talking to anyone else.
static CONTENT_SECURITY_POLICY: LazyLock<String> = LazyLock::new(|| {
    format!(
        "default-src 'self'; \
         script-src 'self'; \
         style-src 'self' {}; \
         font-src 'none'; \
         img-src 'self' data:; \
         connect-src 'self' ws: wss:; \
         frame-ancestors 'none'; \
         base-uri 'none'; \
         form-action 'none'; \
         object-src 'none'",
        *STYLE_BLOCK_HASH
    )
});

/// Headers applied to every response, other than the CSP — computed
/// separately in [`security_header_layers`] since it is the one header built
/// at startup rather than known at compile time.
///
/// Each one is here because something concrete goes wrong without it, noted
/// alongside.
const SECURITY_HEADERS: &[(&str, &str)] = &[
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
    // Nothing this page ever serves is meant to be embedded as a sub-resource
    // by another origin — the same reasoning as `frame-ancestors 'none'`, one
    // layer lower.
    ("cross-origin-resource-policy", "same-origin"),
];

/// Builds the layers that stamp [`SECURITY_HEADERS`] and the computed CSP onto
/// every response.
///
/// `SetResponseHeaderLayer::overriding` rather than `if_not_present`: a header
/// this server considers a security control must not be weakenable by anything
/// downstream setting it first.
pub fn security_header_layers() -> Vec<SetResponseHeaderLayer<HeaderValue>> {
    let csp = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_str(&CONTENT_SECURITY_POLICY)
            .expect("the computed CSP is valid header ASCII"),
    );

    std::iter::once(csp)
        .chain(SECURITY_HEADERS.iter().map(|(name, value)| {
            SetResponseHeaderLayer::overriding(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            )
        }))
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
    let Ok(expected) = std::env::var("METRICS_TOKEN") else {
        return false;
    };

    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes()))
}
