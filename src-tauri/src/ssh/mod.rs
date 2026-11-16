//! SSH transport: connect, verify host keys, authenticate, PTY, resize.
//! Jump hosts arrive in Phase 7.

pub mod client;
pub mod connections;
pub mod known_hosts;
pub mod prompt;
pub mod validate;

pub use client::{Interaction, SshAuth, SshClient, SshConnectParams, SshProtocol, connect};
pub use connections::SshConnections;
pub use prompt::PromptBroker;
