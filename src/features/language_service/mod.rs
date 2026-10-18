//! The language server protocol client's rules, without any I/O: framing, JSON-RPC
//! messages, capability negotiation and the session state machine. Starting
//! processes and moving bytes belongs to `infrastructure::lsp`.
pub(crate) mod capabilities;
pub(crate) mod framing;
pub(crate) mod message;
pub(crate) mod session;
#[cfg(test)]
mod session_tests;
