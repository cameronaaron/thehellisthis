//! The response headers every request gets: the CSP, and the "no third-party
//! origin at all" policy it enforces (§5.7).

use super::*;

/// Every response carries the security headers, and the CSP forbids inline
/// script.
///
/// The server previously sent none at all. Cloudflare adds none of its own for
/// a Worker-proxied origin, so a page that renders user-submitted Markdown as
/// HTML was protected by nothing but browser defaults.
#[tokio::test]
async fn every_response_carries_security_headers() {
    for path in ["/main", "/health", "/app.js", "/robots.txt"] {
        let app = build_router(Arc::new(AppState::new()));
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        let headers = response.headers();
        for required in [
            "content-security-policy",
            "x-frame-options",
            "x-content-type-options",
            "referrer-policy",
            "permissions-policy",
            "strict-transport-security",
        ] {
            assert!(
                headers.contains_key(required),
                "{path} is missing the {required} header"
            );
        }

        let csp = headers
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();

        // The whole point of moving the client script out of the page.
        assert!(
            csp.contains("script-src 'self'") && !csp.contains("script-src 'self' 'unsafe-inline'"),
            "CSP must not permit inline script: {csp}"
        );
        assert!(
            csp.contains("frame-ancestors 'none'"),
            "CSP must forbid framing: {csp}"
        );
        assert!(
            csp.contains("object-src 'none'"),
            "CSP must forbid plugins: {csp}"
        );
    }
}

/// The client script must not be inline, or the CSP above cannot hold.
#[test]
fn client_script_is_external_so_csp_can_forbid_inline() {
    assert!(
        !EMBEDDED_HTML.contains("<script>"),
        "the page must not contain an inline script block"
    );
    assert!(
        EMBEDDED_HTML.contains("src=\"/app.js\""),
        "the page must load its script from /app.js"
    );
    assert!(
        !EMBEDDED_HTML.contains(" onclick=") && !EMBEDDED_HTML.contains(" onload="),
        "inline event handlers are inline script and would need 'unsafe-inline'"
    );
}

/// `/app.js` is content-addressed and cacheable, and revalidates cheaply.
#[tokio::test]
async fn app_js_is_immutably_cacheable_and_revalidates() {
    let app = build_router(Arc::new(AppState::new()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let cache_control = response
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_control.contains("immutable"),
        "the script URL carries a content hash, so it can be immutable: {cache_control}"
    );

    let etag = response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // A conditional request for the same content must be answered 304.
    let app = build_router(Arc::new(AppState::new()));
    let conditional = app
        .oneshot(
            Request::builder()
                .uri("/app.js")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
}

/// The page must reference the script with its content-hash version, so a
/// deploy can never leave a browser running the previous script against a new
/// server.
#[tokio::test]
async fn page_references_the_versioned_script_url() {
    let app = build_router(Arc::new(AppState::new()));
    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&body);

    assert!(
        body.contains("src=\"/app.js?v="),
        "the served page must stamp the script version"
    );
}

// ========== SECURITY: WEBSOCKET ORIGIN ==========

/// The client makes no third-party requests, and the policy says so.
///
/// The page used to pull its typeface and icon font from Google, which meant
/// every visitor's browser announced their address and the room they were
/// opening to a third party — on a site whose entire premise is that you get an
/// animal name instead of an account. Icons are an inline SVG sprite and text
/// uses the system stack, so there is nothing left to fetch.
#[test]
fn the_client_makes_no_third_party_requests() {
    // Match on the URL as it would actually be *fetched* — inside a src/href
    // attribute or a url() — rather than anywhere in the file. The comments
    // explaining why these origins were removed necessarily name them, and a
    // test that a comment can fail is a test that gets silenced rather than
    // fixed.
    let fetched = |source: &str, origin: &str| {
        [
            format!("src=\"https://{origin}"),
            format!("href=\"https://{origin}"),
            format!("url(https://{origin}"),
            format!("//{origin}/"),
        ]
        .iter()
        .any(|pattern| source.contains(pattern.as_str()))
    };

    for (name, source) in [("index.html", EMBEDDED_HTML), ("client.js", EMBEDDED_JS)] {
        for origin in [
            "fonts.googleapis.com",
            "fonts.gstatic.com",
            "unpkg.com",
            "cdn.jsdelivr.net",
            "www.googletagmanager.com",
            "www.google-analytics.com",
        ] {
            assert!(
                !fetched(source, origin),
                "{name} must not fetch from the third-party origin {origin}"
            );
        }
    }

    // Icons are local sprite references, not a downloaded font.
    assert!(
        EMBEDDED_HTML.contains("<use href=\"#i-"),
        "icons should be inline sprite references"
    );
    assert!(
        !EMBEDDED_HTML.contains("class=\"material-icons-round\""),
        "the icon font should be gone entirely"
    );
}

/// With no third-party assets, the policy can forbid outside origins outright.
#[tokio::test]
async fn the_policy_permits_no_third_party_origins() {
    let app = build_router(Arc::new(AppState::new()));
    let response = app
        .oneshot(Request::builder().uri("/main").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let csp = response
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert!(
        !csp.contains("https://"),
        "the policy should name no external origin: {csp}"
    );
    assert!(
        csp.contains("font-src 'none'"),
        "nothing should be loadable as a font: {csp}"
    );
}

// ========== INVARIANT SWEEPS ==========
//
// Each test here pins a law from ENGINEERING-STANDARDS.md as a property of the
// whole server rather than of one call site, so the *next* violation of the
// same class fails here instead of reaching production.

/// §5.7 — every font the page names is one the machine already has.
///
/// Removing the webfonts left two declarations behind that referred to fonts
/// nobody downloads any more. One was cosmetic: `'Roboto Mono', monospace` had
/// been quietly falling back to the generic for months. The other was not —
/// `#chat:empty::before` set `content: 'chat_bubble'` in `'Material Icons
/// Round'`, a *ligature*, so with the font gone every visitor who opened an
/// empty room was shown the literal word "chat_bubble" at 80px.
///
/// That is the trap in deleting a dependency: the code that referenced it still
/// parses, still applies, and fails only in the rendering — which no test that
/// checks for network requests can see. This checks the other half: that no
/// declaration names a family the browser cannot possibly have.
#[test]
fn the_page_names_no_font_it_does_not_ship_with() {
    // Families a browser has without downloading anything: the generic
    // keywords, the system-UI aliases, and the handful of faces that ship with
    // desktop and mobile operating systems.
    const AVAILABLE: &[&str] = &[
        "-apple-system",
        "arial",
        "blinkmacsystemfont",
        "consolas",
        "cursive",
        "fantasy",
        "helvetica",
        "helvetica neue",
        "inherit",
        "liberation mono",
        "menlo",
        "monospace",
        "sans-serif",
        "serif",
        "sf mono",
        "sfmono-regular",
        "segoe ui",
        "system-ui",
        "ui-monospace",
        "ui-sans-serif",
    ];

    let mut offenders: Vec<String> = Vec::new();

    let css = embedded_html_without_comments();

    for (index, _) in css.match_indices("font-family:") {
        let rest = &css[index + "font-family:".len()..];
        let Some(end) = rest.find(';') else { continue };
        let value = &rest[..end];

        // A custom property is checked where it is defined, not where used.
        if value.contains("var(--font-") {
            continue;
        }

        for family in value.split(',') {
            let name = family.trim().trim_matches('\'').trim_matches('"').trim();
            if name.is_empty() {
                continue;
            }
            if !AVAILABLE.contains(&name.to_ascii_lowercase().as_str()) {
                offenders.push(name.to_string());
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "index.html names fonts that are never downloaded and will not resolve: \
         {offenders:?}"
    );
}
