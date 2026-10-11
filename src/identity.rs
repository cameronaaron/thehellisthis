//! Visitor identity: the two cookies that make a returning browser the same
//! animal. There are no accounts: `user_id` is the identity and `animal_name`
//! its display, both server-issued and `HttpOnly`.

use axum::extract::FromRequestParts;
use http::request::Parts;
use tower_cookies::Cookie;
use tracing::debug;

use crate::config::INACTIVE_TIMEOUT;

/// A visitor recognised from their cookies.
#[derive(Debug, Clone)]
pub struct UserCookie {
    pub user_id: String,
    pub animal_name: String,
}

/// Extractor wrapper: absent or malformed cookies are a new visitor, never an
/// error — a first-time browser has no cookies and must still be able to chat.
#[derive(Debug, Clone)]
pub struct OptionalUserCookie(pub Option<UserCookie>);

impl<S> FromRequestParts<S> for OptionalUserCookie
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let cookie_header = parts.headers.get("cookie").and_then(|v| v.to_str().ok());
        let (user_id, animal_name) = parse_identity_cookies(cookie_header);

        if let (Some(u), Some(a)) = (user_id, animal_name)
            && !u.is_empty()
            && !a.is_empty()
        {
            debug!(user_id = %u, animal_name = %a, "recognised returning visitor");
            return Ok(OptionalUserCookie(Some(UserCookie {
                user_id: u,
                animal_name: a,
            })));
        }

        debug!("no usable identity cookies; treating as a new visitor");
        Ok(OptionalUserCookie(None))
    }
}

/// Picks the two identity fields out of a raw `Cookie` header. Pure, so tests
/// pass plain strings for malformed cookies, missing fields and duplicates
/// instead of building requests.
pub(crate) fn parse_identity_cookies(
    cookie_header: Option<&str>,
) -> (Option<String>, Option<String>) {
    let mut user_id = None;
    let mut animal_name = None;

    if let Some(cookie_str) = cookie_header {
        for raw in cookie_str.split(';') {
            let Ok(cookie) = Cookie::parse(raw.trim().to_string()) else {
                continue;
            };
            match cookie.name() {
                "user_id" => user_id = Some(cookie.value().to_string()),
                "animal_name" => animal_name = Some(cookie.value().to_string()),
                _ => {}
            }
        }
    }

    (user_id, animal_name)
}

/// Whether `host` is loopback, the one place http is all there is.
fn is_loopback(host: &str) -> bool {
    let name = host
        .rsplit_once(':')
        .map_or(host, |(name, _)| name)
        .trim_matches(['[', ']']);
    matches!(name, "localhost" | "127.0.0.1" | "::1")
}

/// Builds the `Set-Cookie` values for a user's identity.
///
/// Both are `HttpOnly`: the client learns its identity from
/// [`crate::protocol::OutgoingEvent::Welcome`], so script access would only
/// widen what an XSS through the Markdown pipeline could steal.
///
/// `Secure` is set except on loopback. In production it stops a first http
/// request being used to strip TLS and read the identity. Locally it is
/// fatal: browsers won't store a `Secure` cookie over plain http (Safari
/// refuses even on localhost), so every reconnect minted a new identity and
/// a room filled with "skink left / stinks joined".
pub fn create_user_cookies(user_id: &str, name: &str, host: &str) -> (String, String) {
    let max_age = INACTIVE_TIMEOUT.as_secs();
    let secure = if is_loopback(host) { "" } else { "; Secure" };

    (
        format!("user_id={user_id}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"),
        format!("animal_name={name}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"),
    )
}
