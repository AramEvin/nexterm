//! File manager back end: local and remote (SFTP) file systems, and the
//! transfer queue.
//!
//! - [`remote`]: one SFTP session per file-manager pane, on its own SSH
//!   connection (same host-key checks and credentials as terminals).
//! - [`local`]: the user's own disks.
//! - [`fs`]: one interface over both, so a transfer is just "read from one
//!   side, write to the other" - local↔remote and remote↔remote (relay
//!   through this PC) use the same code.
//! - [`transfer`]: queued, streamed (>4 GB fine), cancellable copies with
//!   progress events.
//! - [`paths`]: validation, including names that come from the server.

pub mod fs;
pub mod local;
pub mod paths;
pub mod registry;
pub mod remote;
pub mod transfer;

use serde::Serialize;

pub use registry::FileSessions;
pub use remote::RemoteFs;
pub use transfer::Transfers;

/// What a directory entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One row in a file-manager pane.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    /// Full path, ready to send back in later commands.
    pub path: String,
    pub kind: EntryKind,
    /// For symlinks: whether the target is a folder (double-click opens it).
    pub link_to_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<i64>,
    /// POSIX mode bits (`0o755`); `None` on Windows disks.
    pub permissions: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub hidden: bool,
}

/// A folder's contents.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// Canonical path of the folder ("" = the list of drives on Windows).
    pub path: String,
    /// Where "Up" goes; `None` at the top.
    pub parent: Option<String>,
    pub entries: Vec<FileEntry>,
    /// The listing was cut at [`MAX_LISTING`] entries.
    pub truncated: bool,
}

/// Most entries returned for one folder (the UI stays responsive).
pub const MAX_LISTING: usize = 20_000;

/// Folders first, then case-insensitive by name.
pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        let a_dir = a.kind == EntryKind::Dir || a.link_to_dir;
        let b_dir = b.kind == EntryKind::Dir || b.link_to_dir;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}
