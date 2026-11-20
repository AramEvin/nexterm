//! Live-session lifecycle (`SessionManager`) and the protocol boundary.

pub mod blocking;
pub mod manager;
pub mod monitor;
pub mod options;
pub mod protocol;

pub use manager::SessionManager;
