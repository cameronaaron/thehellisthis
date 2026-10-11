//! The shared half of `nova-operator`: the wire protocol and the sealing
//! primitive, used by the operator binary and by the chat server's
//! `session/nova_operator.rs`, so both sides use the same types.

pub mod crypto;
pub mod protocol;
pub mod wire;
