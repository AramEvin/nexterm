//! Protocol boundary.
//!
//! v1 only ships SSH, but RDP, VNC, serial and local shells are planned. Every
//! protocol plugs in behind this boundary so adding one is a new module, not a
//! rewrite. The full `Protocol` trait (connect / write / resize / close) is
//! defined in Phase 1, once the SSH implementation shows the real shape - see
//! `docs/adr/0002-protocol-boundary.md`.

use serde::{Deserialize, Serialize};

/// Kinds of session NexTerm can open. Serialized as lowercase strings
/// (`"ssh"`), which is also how they will be stored in SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolKind {
    Ssh,
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
}
