//! Visitor identity: the two cookies that make a returning browser the same
//! animal it was before.
//!
//! There is no account system. `user_id` is the identity and `animal_name` is
//! its display; both are issued by the server and both are `HttpOnly`.

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
        let mut user_id = None;
        let mut animal_name = None;

        if let Some(cookie_str) = parts.headers.get("cookie").and_then(|v| v.to_str().ok()) {
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

/// Builds the `Set-Cookie` values for a user's identity.
///
/// `HttpOnly` is set on both. The client learns its own identity from the
/// [`crate::protocol::OutgoingEvent::Welcome`] frame instead of reading
/// `document.cookie`, so script access buys nothing and only widens what an
/// XSS through the Markdown pipeline could steal.
pub fn create_user_cookies(user_id: &str, name: &str) -> (String, String) {
    let max_age = INACTIVE_TIMEOUT.as_secs();

    (
        format!("user_id={user_id}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict; Secure"),
        format!("animal_name={name}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict; Secure"),
    )
}
