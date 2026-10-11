//! The WebSocket session: admission, the four per-connection tasks and
//! teardown.
//!
//! Every admission check that can fail runs before any counter is
//! incremented, so a rejection can't leave a reservation behind; the one
//! decision made under the room write lock releases its slot itself.
//!
//! By phase: [`admission`] (the upgrade and placing the visitor), [`tasks`]
//! (the four raced tasks), [`lifecycle`] (wiring them together), [`events`]
//! (applying one client event), [`teardown`] (releasing everything), and the
//! `nova` room's [`nova`], [`nova_rln`] and [`nova_operator`].

mod admission;
mod events;
mod lifecycle;
mod nova;
mod nova_operator;
mod nova_rln;
mod tasks;
mod teardown;

pub(crate) use axum::extract::ws::Message;

pub use admission::ws_handler;
#[cfg(test)]
pub(crate) use admission::{admit_user, attach_cookies, check_admission};

pub(crate) use crate::protocol::encode_event;
#[cfg(test)]
pub(crate) use nova::{remove_member, run_seal_loop};
#[cfg(test)]
pub(crate) use nova_operator::complaint_is_valid;
pub(crate) use nova_operator::{NovaOperatorRegistry, nova_operator_ws_handler};
pub(crate) use nova_rln::NovaRlnGroup;
#[cfg(test)]
pub(crate) use nova_rln::{NullifierSet, RlnOutcome};
pub(crate) use tasks::{
    FrameSink, beat_and_evict_idle, forward_broadcasts, send_history, send_pings,
};
#[cfg(test)]
pub(crate) use tasks::{
    idle_close_frame, is_superseded, superseded_close_frame, touch_and_check_idle,
};

#[cfg(test)]
pub(crate) use lifecycle::{join_room, run_session};

pub use events::apply_client_event;
#[cfg(test)]
pub use events::apply_client_event_at;
#[cfg(test)]
pub(crate) use events::{render_off_thread, resolve_render, within_throttle};

#[cfg(test)]
pub use teardown::cleanup_user;
