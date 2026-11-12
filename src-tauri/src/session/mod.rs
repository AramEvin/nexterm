//! Live-session lifecycle (`SessionManager`) and the protocol boundary.
//!
//! Each live session will run in its own tokio task so a crash in one session
//! cannot affect the others. The `SessionManager` itself arrives in Phase 1/2.

pub mod protocol;
