//! Shared library half of `nova-operator`: the wire protocol and the
//! point-to-point sealing primitive, used by both this crate's own `main.rs`
//! (an operator process) and the main chat server's `session/nova_operator.rs`
//! (the coordinator/relay). See `protocol.rs`'s doc comment for why sharing
//! the types is the point.

pub mod crypto;
pub mod protocol;
pub mod wire;
