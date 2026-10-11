//! infinite-chat: an ephemeral, room-based WebSocket chat server.
//!
//! Rooms exist once visited and are deleted when empty. No database or
//! accounts: all state is in memory and meant to disappear. See `AGENTS.md`
//! for the map and `ENGINEERING-STANDARDS.md` for why each rule exists.
//!
//! **This file is the entry point and nothing else.** No test can call `main`,
//! so the work lives in `startup.rs`, which is what keeps this file's coverage
//! exemption honest (scripts/coverage-exemptions.toml).

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    startup::main_inner(tokio::signal::ctrl_c()).await
}
