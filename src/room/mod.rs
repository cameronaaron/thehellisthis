//! A single chat room: its users, its history, and its broadcast channel.
//!
//! Everything here runs while the caller holds the room lock, which every
//! connected user in that room contends on. That makes the cost of each method
//! a latency budget rather than a micro-optimisation: work proportional to
//! history length must run rarely, and work on the message path must be O(1).
//!
//! `RoomState`'s methods are split across files by concern — [`names`] is
//! animal-name assignment, [`messages`] is history storage and pruning,
//! [`reactions`] is the per-message emoji buckets, [`presence`] is the
//! roster and broadcasts — but they are all still methods on the one type,
//! defined once in [`types`]. Rust does not require an `impl` block to live
//! in the same file as the struct it extends.

mod messages;
mod names;
mod presence;
mod reactions;
mod types;

pub use types::{ConnectionState, RoomState, UserData, create_room, user_idle_for_too_long};
