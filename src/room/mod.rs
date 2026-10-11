//! A single chat room: its users, history and broadcast channel.
//!
//! Everything here runs under the room lock that every user in the room
//! contends on, so each method's cost is a latency budget: work proportional
//! to history runs rarely, and message-path work is O(1).
//!
//! `RoomState`'s methods are split by concern, [`names`], [`messages`],
//! [`reactions`] and [`presence`], all on the one type defined in [`types`].

mod messages;
mod names;
mod presence;
mod reactions;
mod types;

pub(crate) use types::BROADCAST_CHANNEL_CAPACITY;
pub use types::{ConnectionState, RoomState, UserData, create_room, user_idle_for_too_long};
