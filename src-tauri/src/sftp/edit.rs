//! "Edit" a remote file in a local editor, WinSCP-style:
//!
//! 1. Download it to a private folder: `%TEMP%\NexTerm\edit\<random>\<name>`.
//! 2. Start the editor **directly** (no shell): `%SystemRoot%\System32\notepad.exe <file>`.
//!    The program path is fixed, so the file is never "opened" with whatever
//!    app its extension maps to (a downloaded `.exe` / `.bat` is never run).
//! 3. Every second, check whether the file changed; after a save settles,
//!    upload it back to the same remote path and report it to the UI.
//! 4. When the user stops editing (or after 12 h), delete the local copy.
//!
//! Other operating systems need `NEXTERM_EDITOR` (an absolute path) until the
//! editor setting arrives in Phase 10.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::fs::Fs;
use super::paths::{base_name, safe_local_name};
use crate::error::{AppError, AppResult};

/// Largest file opened for editing.
pub const MAX_EDIT_BYTES: u64 = 50 * 1024 * 1024;
/// Files edited at the same time.
pub const MAX_EDITS: usize = 32;
const POLL: Duration = Duration::from_secs(1);
const MAX_SESSION: Duration = Duration::from_secs(12 * 3600);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EditEvent {
    #[serde(rename_all = "camelCase")]
    Opened {
        local_path: String,
        editor: String,
    },
    #[serde(rename_all = "camelCase")]
    Uploaded {
        bytes: u64,
        at_ms: i64,
    },
    Failed {
        message: String,
    },
    Closed,
}

pub trait EditSink: Send + Sync {
    fn send(&self, event: EditEvent);
}

#[derive(Default)]
pub struct Edits {
    stops: std::sync::Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
}

impl Edits {
    pub fn new() -> Self {
        Self::default()
    }

    fn stops(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<AtomicBool>>> {
        self.stops
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Download `remote` from `fs`, open it in the editor and keep uploading
    /// saves until [`Edits::stop`]. `editor` = `None` uses the default.
    pub async fn start(
        self: &Arc<Self>,
        fs: Fs,
        remote: String,
        editor: Option<PathBuf>,
        sink: Arc<dyn EditSink>,
    ) -> AppResult<Uuid> {
        if fs.is_local() {
            return Err(AppError::Validation(
                "Local files are edited directly.".into(),
            ));
        }
        if self.stops().len() >= MAX_EDITS {
            return Err(AppError::Validation(format!(
                "Too many files open for editing (max {MAX_EDITS})."
            )));
        }
        let editor = match editor {
            Some(e) => e,
            None => default_editor()?,
        };
        let meta = fs.stat(&remote, true).await?;
        if meta.is_dir {
            return Err(AppError::Validation("Folders can't be edited.".into()));
        }
        if meta.size > MAX_EDIT_BYTES {
            return Err(AppError::FileOp(format!(
                "\"{}\" is too large to edit ({} MB max). Download it instead.",
                base_name(&remote),
                MAX_EDIT_BYTES / (1024 * 1024)
            )));
        }

        let id = Uuid::new_v4();
        let dir = edit_root().join(id.simple().to_string());
        let local = dir.join(local_name(&remote));
        create_private_dir(&dir)?;
        if let Err(e) = download(&fs, &remote, &local).await {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let baseline = stamp(&local);
        if let Err(e) = std::process::Command::new(&editor).arg(&local).spawn() {
            let _ = std::fs::remove_dir_all(&dir);
            tracing::warn!(error = %e, "editor did not start");
            return Err(AppError::FileOp(format!(
                "The editor could not be started ({}).",
                editor.display()
            )));
        }

        let stop = Arc::new(AtomicBool::new(false));
        self.stops().insert(id, Arc::clone(&stop));
        tracing::info!("remote file opened for editing");
        sink.send(EditEvent::Opened {
            local_path: local.display().to_string(),
            editor: editor
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });

        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            watch(&fs, &remote, &local, baseline, &stop, sink.as_ref()).await;
            this.stops().remove(&id);
            // The editor may still hold the file open (Windows); stale folders
            // are also swept at start-up.
            let _ = tokio::fs::remove_dir_all(&dir).await;
            tracing::info!("remote file editing stopped");
            sink.send(EditEvent::Closed);
        });
        Ok(id)
    }

    pub fn stop(&self, id: Uuid) -> bool {
        match self.stops().get(&id) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// `(modified, len)` of the local copy.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| Some((m.modified().ok()?, m.len())))
}

async fn watch(
    fs: &Fs,
    remote: &str,
    local: &Path,
    mut uploaded: Stamp,
    stop: &AtomicBool,
    sink: &dyn EditSink,
) {
    let started = Instant::now();
    let mut pending: Stamp = None;
    while !stop.load(Ordering::Relaxed) && started.elapsed() < MAX_SESSION {
        tokio::time::sleep(POLL).await;
        let now = stamp(local);
        if now.is_none() || now == uploaded {
            pending = None;
            continue;
        }
        // Wait until the save has settled (same stamp twice in a row).
        if pending != now {
            pending = now;
            continue;
        }
        pending = None;
        match upload(fs, local, remote).await {
            Ok(bytes) => {
                uploaded = now;
                tracing::info!(bytes, "edited file uploaded");
                sink.send(EditEvent::Uploaded {
                    bytes,
                    at_ms: crate::storage::now_ms(),
                });
            }
            Err(e) => {
                tracing::warn!(code = e.code(), "edited file upload failed");
                uploaded = now; // don't retry the same version in a loop
                sink.send(EditEvent::Failed {
                    message: e.user_message(),
                });
            }
        }
    }
}

async fn download(fs: &Fs, remote: &str, local: &Path) -> AppResult<()> {
    let mut reader = fs.open_read(remote).await?;
    let mut file = tokio::fs::File::create(local)
        .await
        .map_err(|e| super::local::io_error(&e, local))?;
    tokio::io::copy(&mut reader, &mut file)
        .await
        .map_err(|e| AppError::FileOp(format!("Download failed: {e}")))?;
    file.flush()
        .await
        .map_err(|e| super::local::io_error(&e, local))
}

async fn upload(fs: &Fs, local: &Path, remote: &str) -> AppResult<u64> {
    let mut file = tokio::fs::File::open(local)
        .await
        .map_err(|e| super::local::io_error(&e, local))?;
    let mut writer = fs.open_write(remote).await?;
    let bytes = tokio::io::copy(&mut file, &mut writer)
        .await
        .map_err(|e| AppError::FileOp(format!("Upload failed: {e}")))?;
    writer
        .shutdown()
        .await
        .map_err(|e| AppError::FileOp(format!("Upload failed: {e}")))?;
    Ok(bytes)
}

/// `%TEMP%\NexTerm\edit`
pub fn edit_root() -> PathBuf {
    std::env::temp_dir().join("NexTerm").join("edit")
}

/// Delete edit folders left over from earlier runs (best effort).
pub fn sweep_stale(max_age: Duration) {
    let Ok(dirs) = std::fs::read_dir(edit_root()) else {
        return;
    };
    for d in dirs.flatten() {
        let old = d
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_dir_all(d.path());
        }
    }
}

/// The remote name if it is a safe local name, else a neutral one.
fn local_name(remote: &str) -> String {
    let name = base_name(remote);
    if safe_local_name(name, cfg!(windows)) {
        name.to_owned()
    } else {
        "remote-file.txt".to_owned()
    }
}

fn create_private_dir(dir: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dir).map_err(|e| super::local::io_error(&e, dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| super::local::io_error(&e, dir))?;
    }
    Ok(())
}

/// Notepad on Windows (absolute path, not looked up in PATH);
/// `NEXTERM_EDITOR` (absolute) elsewhere.
pub fn default_editor() -> AppResult<PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let notepad = PathBuf::from(root).join("System32").join("notepad.exe");
        if notepad.is_file() {
            return Ok(notepad);
        }
        return Err(AppError::FileOp("Notepad was not found.".into()));
    }
    match std::env::var_os("NEXTERM_EDITOR").map(PathBuf::from) {
        Some(p) if p.is_absolute() && p.is_file() => Ok(p),
        _ => Err(AppError::FileOp(
            "Editing remote files needs Windows for now (or NEXTERM_EDITOR set to an editor's \
             full path)."
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_are_safe() {
        assert_eq!(local_name("/etc/nginx/nginx.conf"), "nginx.conf");
        assert_eq!(local_name("/x/.."), "remote-file.txt");
        assert_eq!(local_name("/x/a\\b"), "remote-file.txt");
    }
}
