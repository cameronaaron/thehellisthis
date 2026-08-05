//! The Content-Security-Policy and the fixed set of headers stamped on every
//! response.

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
/// `'wasm-unsafe-eval'` is scoped, not `'unsafe-eval'`: it grants exactly
/// `WebAssembly.instantiate`/`compile`, which CSP3 requires explicitly even
/// for a same-origin module, and grants nothing about `eval()` or
/// `new Function()` — those stay refused. It exists for one consumer, the
/// `nova` room's `/nova.js`/`/nova_wasm_bg.wasm` (`nova-wasm/`,
/// `session/nova.rs`), loaded only inside `client.js`'s
/// `roomName === NOVA_ROOM_NAME` branch; every other room never triggers a
/// WebAssembly compile at all, so this permission being room-wide rather
/// than page-wide is a smaller policy than it looks.
///
/// Every other directive is `'self'` or `'none'`. There are no third-party
/// origins at all: the page used to load its typeface and icon font from
/// Google, which both widened this policy and told a third party the address of
/// every visitor and the room they opened. Icons are now an inline SVG sprite
/// and text uses the system stack, so `font-src 'none'` is achievable rather
/// than aspirational — the browser is told, in the policy itself, that this
/// page has no business talking to anyone else.
///
/// `connect-src 'self'` — no bare `ws:`/`wss:` scheme-sources. A
/// scheme-source with no host matches *any* host on that scheme: had this
/// read `connect-src 'self' ws: wss:`, script running under an XSS would be
/// free to open a socket to `wss://attacker.example` and exfiltrate whatever
/// it could read, which is exactly the capability `connect-src` exists to
/// deny. `'self'` alone already covers the one connection this page ever
/// makes — the CSP Fetch Directives spec treats `ws`/`wss` as the matching
/// pair for `http`/`https` when testing a request against `'self'`, so the
/// browser's own same-origin WebSocket to this host needs no separate grant.
/// The Worker fronts the container on 443 and the container listens on
/// :3000, but the browser never sees :3000 — every request it makes,
/// including the WebSocket upgrade, targets the Worker's port, so `'self'`'s
/// port match holds without a scheme-source carve-out.
///
/// `require-trusted-types-for 'script'` + `trusted-types chat-html` is a
/// second, independent backstop behind the same thing `script-src` already
/// guards: `client.js` calls `.innerHTML =` at seven sites, all through the
/// one `trustedHtml` policy it creates at module load. A browser that
/// enforces this directive throws on any assignment that did not go through
/// that policy — including a future call site added without remembering the
/// convention, or a hijacked reference to a sink these directives don't
/// otherwise reach. It does not re-sanitise anything itself; the server
/// already did that (constraint #8), and stays the trust boundary. A browser
/// that does not implement Trusted Types ignores the directive as
/// unrecognised, so this changes nothing observable anywhere it is not
/// enforced — see `client.js`'s `trustedHtml` for the fallback.
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
    // Nothing here needs hardware access. `interest-cohort=()` used to sit
    // here too, opting out of Chrome's FLoC trial — Chrome removed FLoC
    // entirely, so the directive names a feature no browser recognizes
    // anymore and only produced a console warning ("Unrecognized feature").
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
