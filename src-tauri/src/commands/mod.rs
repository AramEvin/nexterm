//! Thin IPC layer. Commands only validate input and delegate to the core
//! modules; no business logic lives here.
//!
//! Every new command must also be:
//! 1. listed in `build.rs` (`AppManifest::commands`), and
//! 2. granted in `capabilities/main.json`,
//!
//! otherwise the frontend cannot call it (least privilege).

pub mod app;
pub mod library;
pub mod ssh;
