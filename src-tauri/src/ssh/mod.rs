//! SSH transport: connect, authenticate, PTY, resize.
//! Jump hosts (Phase 6) and host-key verification (Phase 4) arrive later.

pub mod client;
pub mod validate;

pub use client::{SshConnectParams, SshProtocol};
