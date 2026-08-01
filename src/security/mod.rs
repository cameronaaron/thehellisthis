//! Transport-level hardening: the response headers every request gets, and the
//! origin check on the WebSocket handshake.
//!
//! The server previously sent no security headers at all. Cloudflare adds none
//! on its own for a Worker-proxied origin, so the browser was applying nothing
//! but its defaults to a page that renders user-submitted Markdown as HTML.
//!
//! Split by concern: [`headers`] builds the CSP and the fixed header set,
//! [`origin`] is the WebSocket handshake's own same-origin check, [`tokens`]
//! is the constant-time credential check `/metrics` and `/admin` share.

mod headers;
mod origin;
mod tokens;

pub use headers::security_header_layers;
pub use origin::is_allowed_origin;
pub use tokens::{is_authorized_for_admin, is_authorized_for_metrics};
