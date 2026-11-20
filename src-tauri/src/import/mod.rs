//! Import saved sessions from MobaXterm and PuTTY (Phase 7).
//!
//! Parsing is pure (bytes in, preview out) and treats the file as untrusted:
//! every value goes through the same validators as the session editor, and
//! anything that doesn't fit is listed as skipped with a reason. Passwords
//! are never imported (MobaXterm keeps them encrypted elsewhere, PuTTY has
//! none); private-key paths are noted so the user can link a key credential.

pub mod apply;
pub mod mobaxterm;
pub mod putty;

use serde::{Deserialize, Serialize};

use crate::session::protocol::ProtocolKind;
use crate::ssh::validate;

/// Largest file accepted (a MobaXterm.ini with thousands of sessions is ~2 MB).
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Most sessions one import may create.
pub const MAX_SESSIONS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportSource {
    Mobaxterm,
    Putty,
}

impl ImportSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mobaxterm => "MobaXterm",
            Self::Putty => "PuTTY",
        }
    }
}

/// A jump host (gateway / SSH proxy) in front of an imported session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedHop {
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedSession {
    pub name: String,
    /// Folder path inside the import folder (`["Prod", "DB"]`).
    pub folder: Vec<String>,
    pub protocol: ProtocolKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// `password`, `publickey` or `agent`.
    pub auth_method: String,
    /// Key file the other program used (for the notes; never read).
    pub key_path: Option<String>,
    /// Jump hosts, dialled first to last (the last one reaches the target).
    pub jump: Vec<ImportedHop>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub source: ImportSource,
    pub sessions: Vec<ImportedSession>,
    pub skipped: Vec<Skipped>,
}

impl ImportPreview {
    fn new(source: ImportSource) -> Self {
        Self {
            source,
            sessions: Vec::new(),
            skipped: Vec::new(),
        }
    }

    fn skip(&mut self, name: &str, reason: impl Into<String>) {
        self.skipped.push(Skipped {
            name: clip(name, 128),
            reason: reason.into(),
        });
    }

    /// Validate host/port/user and add, or record why not.
    fn push(&mut self, mut s: ImportedSession) {
        if self.sessions.len() >= MAX_SESSIONS {
            self.skip(
                &s.name,
                format!("More than {MAX_SESSIONS} sessions in one import."),
            );
            return;
        }
        s.name = clip(s.name.trim(), crate::storage::validate::MAX_NAME_LEN);
        if s.name.is_empty() {
            s.name = s.host.clone();
        }
        match validate::host(&s.host) {
            Ok(h) => s.host = h,
            Err(_) => return self.skip(&s.name, "The host name isn't valid."),
        }
        if s.port == 0 {
            return self.skip(&s.name, "The port isn't valid.");
        }
        if !s.username.is_empty() && validate::username(&s.username).is_err() {
            s.username.clear(); // asked at connect time instead
        }
        let mut hops = Vec::new();
        for mut hop in std::mem::take(&mut s.jump) {
            let Ok(h) = validate::host(&hop.host) else {
                return self.skip(&s.name, "A jump host name isn't valid.");
            };
            hop.host = h;
            if hop.port == 0 {
                hop.port = 22;
            }
            if !hop.username.is_empty() && validate::username(&hop.username).is_err() {
                hop.username.clear();
            }
            hops.push(hop);
        }
        if hops.len() > crate::storage::sessions::MAX_JUMP_HOPS - 1 {
            return self.skip(&s.name, "Too many jump hosts.");
        }
        s.jump = hops;
        s.folder = s
            .folder
            .iter()
            .map(|f| clip(f.trim(), crate::storage::validate::MAX_FOLDER_NAME_LEN))
            .filter(|f| !f.is_empty())
            .take(8)
            .collect();
        self.sessions.push(s);
    }
}

/// Truncate to `max` characters.
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Parse the file of a source.
pub fn parse(source: ImportSource, bytes: &[u8]) -> ImportPreview {
    match source {
        ImportSource::Mobaxterm => mobaxterm::parse(bytes),
        ImportSource::Putty => putty::parse_reg(bytes),
    }
}
