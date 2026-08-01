//! Plain HTTP handlers. The chat itself is entirely over WebSocket; these
//! serve the page, the robots file, and the two operational endpoints.
//!
//! Split by what each response is: [`page`] is the client page and script,
//! [`health`] the liveness probe and robots file, [`admin`] the operator
//! dashboard, [`metrics`] the Prometheus exposition.

mod admin;
mod health;
mod metrics;
mod page;

pub(crate) use page::CLIENT_HTML;
#[cfg(test)]
pub(crate) use page::{RoomNameRejection, fnv1a, room_name_rejection};
pub use page::{app_js_handler, main_room_handler, room_handler, root_redirect};

pub use admin::admin_dashboard_handler;
#[cfg(test)]
pub(crate) use admin::render_admin_dashboard;

pub use health::{health_handler, robots_txt_handler};

pub use metrics::metrics_handler;
