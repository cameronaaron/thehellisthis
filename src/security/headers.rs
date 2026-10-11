//! The Content-Security-Policy and the fixed set of headers stamped on every
//! response.

use std::sync::LazyLock;

use axum::http::{HeaderName, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};
use tower_http::set_header::SetResponseHeaderLayer;

/// The page's one `<style>` block exactly as a browser reads it: the text
/// between the tags. A CSP hash is computed over this substring, and there is
/// one `<style>` element (`the_page_has_exactly_one_style_block`; a second would
/// be hashed under a name nothing points at), so one extraction is unambiguous.
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
/// `script-src 'self'` is the directive that matters: the message pipeline
/// turns user input into HTML, so if the sanitiser is bypassed, injected script
/// must still not run. That is why the client's script is `/app.js`; inline, a
/// workable policy needed `'unsafe-inline'`, exactly what an injected
/// `<script>` needs.
///
/// `style-src` has no `'unsafe-inline'` either and names its one inline
/// surface by hash. Runtime styling is all CSSOM property assignment
/// (`element.style.top = '4px'`), which `style-src` never restricted; a
/// `securitypolicyviolation` listener across every call site (banner dismiss,
/// typing indicator, attachment boxes, reaction bar) saw no violations. Only
/// the page's single `<style>` block needed the hash; removing
/// `'unsafe-inline'` alone unstyled the whole page. [`STYLE_BLOCK_HASH`] is
/// computed at startup so it can't drift: a stale hand-copied hash fails
/// closed with nothing but unstyled HTML to show for it.
///
/// `cross-origin-resource-policy: same-origin`: nothing here is meant to be
/// embedded by another origin, like `frame-ancestors 'none'` one layer down.
/// `cross-origin-embedder-policy: require-corp` was rejected (§9.4): it broke
/// the WebSocket under Chrome's Private Network Access locally, and couldn't
/// be verified against production.
///
/// `'wasm-unsafe-eval'`, not `'unsafe-eval'`: CSP3 requires it for
/// `WebAssembly.instantiate`/`compile` even same-origin, and it allows nothing
/// for `eval()` or `new Function()`. Its one consumer is the `nova` room's
/// `/nova.js` and `/nova_wasm_bg.wasm`, loaded only in `client.js`'s
/// `roomName === NOVA_ROOM_NAME` branch; no other room compiles WebAssembly.
///
/// Everything else is `'self'` or `'none'`, with no third-party origin. The
/// page once loaded fonts from Google, widening the policy and disclosing
/// every visitor and room; icons are now an inline SVG sprite and text uses
/// the system stack, so `font-src 'none'` holds.
///
/// `connect-src 'self'` with no bare `ws:`/`wss:`: a host-less scheme source
/// matches any host, so `connect-src 'self' ws: wss:` would let script under
/// an XSS open a socket to `wss://attacker.example`. `'self'` already covers
/// the page's same-origin WebSocket (CSP treats `ws`/`wss` as matching
/// `http`/`https`), and the browser only ever addresses the Worker's port,
/// never the container's :3000.
///
/// `require-trusted-types-for 'script'` with `trusted-types chat-html` is a
/// second backstop: `client.js` assigns `.innerHTML` at seven sites, all
/// through its one `trustedHtml` policy, and an enforcing browser throws on any
/// assignment that bypasses it, including one a future edit adds. It doesn't
/// re-sanitise; the server is the trust boundary (constraint #8). Browsers
/// without Trusted Types ignore the directive (see `client.js`'s fallback).
static CONTENT_SECURITY_POLICY: LazyLock<String> = LazyLock::new(|| {
    format!(
        "default-src 'self'; \
         script-src 'self' 'wasm-unsafe-eval'; \
         style-src 'self' {}; \
         font-src 'none'; \
         img-src 'self' data:; \
         connect-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'none'; \
         form-action 'none'; \
         object-src 'none'; \
         require-trusted-types-for 'script'; \
         trusted-types chat-html",
        *STYLE_BLOCK_HASH
    )
});

/// Headers on every response besides the CSP, which
/// [`security_header_layers`] builds at startup. Each is here because something
/// concrete goes wrong without it.
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
    // Nothing here needs hardware access. `interest-cohort=()` (opting
    // out of FLoC) was removed: Chrome dropped FLoC, and the unknown
    // feature only produced a console warning.
    (
        "permissions-policy",
        "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
    ),
    // Identity cookies are Secure; without HSTS the first request over http is
    // still an opportunity to strip TLS.
    (
        "strict-transport-security",
        "max-age=31536000; includeSubDomains; preload",
    ),
    ("cross-origin-opener-policy", "same-origin"),
    ("x-permitted-cross-domain-policies", "none"),
    // Nothing here is meant to be embedded by another origin, like
    // `frame-ancestors 'none'` one layer down.
    ("cross-origin-resource-policy", "same-origin"),
];

/// Layers stamping [`SECURITY_HEADERS`] and the CSP onto every response.
/// `overriding`, not `if_not_present`, so nothing downstream can weaken a
/// security header by setting it first.
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
