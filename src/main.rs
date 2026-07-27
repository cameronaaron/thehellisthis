//! infinite-chat — an ephemeral, room-based WebSocket chat server.
//!
//! Rooms are created by visiting them and deleted when nobody is talking. There
//! is no database and no account system: all state is in memory and all of it
//! is designed to disappear. See `CLAUDE.md` for the map and
//! `ENGINEERING-STANDARDS.md` for why each rule here exists.
//!
//! **This file is the entry point and nothing else.** `main` cannot be called
//! from a test, so every line here is a line no test can ever cover; the work
//! lives in `startup.rs`, which can be. That is what makes the coverage
//! exemption for this file honest rather than somewhere to hide things
//! (scripts/coverage-exemptions.toml).

mod animals;
mod cleanup;
mod config;
mod emoji;
mod error;
mod identity;
mod limits;
mod protocol;
mod room;
mod routes;
mod security;
mod session;
mod startup;
mod state;
mod validation;

#[cfg(test)]
mod tests;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    startup::main_inner(tokio::signal::ctrl_c()).await
}
