//! Transport-level hardening: the headers every response gets, the WebSocket
//! origin check, and endpoint tokens.
//!
//! The server once sent no security headers, and Cloudflare adds none to a
//! Worker-proxied origin, so a page rendering user Markdown got only browser
//! defaults. [`headers`] builds the CSP and fixed headers, [`origin`] checks
//! the handshake, and [`tokens`] is the constant-time credential check for
//! `/metrics`, `/admin` and the nova operators.

mod headers;
mod origin;
mod tokens;

pub use headers::security_header_layers;
pub use origin::is_allowed_origin;
pub use tokens::{
    is_authorized_for_admin, is_authorized_for_metrics, is_authorized_for_nova_operator,
};
