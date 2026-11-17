//! Protocol boundary.
//!
//! Every terminal-like protocol (SSH today; serial / local shell later) plugs
//! in by implementing [`TerminalProtocol`]. The rest of the app only sees:
//!
//! - a [`SessionHandle`] (send [`SessionCommand`]s, await the task), and
//! - an [`OutputSink`] the protocol pushes bytes and status changes into.
//!
//! Nothing in here depends on Tauri, so protocols can be tested headless.
//! See `docs/adr/0002-protocol-boundary.md`.

use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::AppResult;

/// Kinds of session NexTerm can open. Serialized as lowercase strings
/// (`"ssh"`), which is also how they will be stored in SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolKind {
    Ssh,
    /// Plain FTP (unencrypted; the UI shows a warning badge). File manager only.
    Ftp,
    /// FTP with explicit TLS (AUTH TLS). File manager only.
    Ftps,
}

impl ProtocolKind {
    /// Opens in the file manager instead of a terminal.
    pub fn is_file_only(self) -> bool {
        matches!(self, Self::Ftp | Self::Ftps)
    }
}

/// Terminal dimensions in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

/// Commands the UI sends to a running session.
#[derive(Debug)]
pub enum SessionCommand {
    /// Keystrokes / pasted text, already encoded as bytes.
    Write(Vec<u8>),
    /// The terminal was resized.
    Resize(TerminalSize),
    /// The user closed the session.
    Close,
}

/// Lifecycle notifications sent to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionStatus {
    /// The session ended. `reason` is user-facing (no secrets, no internals).
    Closed { reason: String },
}

/// Where a protocol delivers terminal output and status changes.
pub trait OutputSink: Send + Sync + 'static {
    /// Deliver raw terminal bytes. Returns `false` once the receiver is gone.
    fn data(&self, bytes: Vec<u8>) -> bool;
    /// Deliver a status change.
    fn status(&self, status: SessionStatus);
}

/// A running session: a command queue plus the task that owns the connection.
pub struct SessionHandle {
    pub kind: ProtocolKind,
    pub commands: mpsc::Sender<SessionCommand>,
    pub task: JoinHandle<()>,
}

/// Implemented once per terminal-like protocol.
pub trait TerminalProtocol {
    /// Protocol-specific connection parameters (host, credentials, ...).
    type Params: Send;

    /// Connect, authenticate and start an interactive shell. On success the
    /// connection runs in its own task; output flows into `sink`.
    fn open(
        params: Self::Params,
        size: TerminalSize,
        sink: Arc<dyn OutputSink>,
    ) -> impl Future<Output = AppResult<SessionHandle>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ProtocolKind::Ssh).unwrap(),
            "\"ssh\""
        );
    }

    #[test]
    fn status_serializes_with_kind_tag() {
        let json = serde_json::to_value(SessionStatus::Closed {
            reason: "bye".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "closed");
        assert_eq!(json["reason"], "bye");
    }
}
