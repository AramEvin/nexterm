//! Session import commands (Phase 7). The file is picked by a native dialog
//! opened from Rust and read here; the UI only sees the parsed preview.

use std::io::Read;

use serde::Deserialize;
use tauri::State;

use crate::commands::library::LibraryDb;
use crate::error::{AppError, AppResult};
use crate::import::apply::{self, ImportResult};
use crate::import::{self, ImportPreview, ImportSource, ImportedSession, MAX_FILE_BYTES, putty};
use crate::sftp::fs::blocking;

/// Where to import from.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImportFrom {
    /// A `.mxtsessions` export or `MobaXterm.ini`.
    MobaxtermFile,
    /// A `.reg` export of PuTTY's sessions.
    PuttyFile,
    /// PuTTY's sessions in this PC's registry (Windows).
    PuttyRegistry,
}

fn read_capped(path: &std::path::Path) -> AppResult<Vec<u8>> {
    let file = std::fs::File::open(path)
        .map_err(|_| AppError::Validation("The file could not be opened.".into()))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::Validation("The file could not be read.".into()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AppError::Validation(
            "The file is too large to be a session export.".into(),
        ));
    }
    Ok(bytes)
}

/// Pick the file (if any) and return what would be imported. `None` when the
/// user closed the file dialog.
#[tauri::command]
pub async fn import_preview(
    from: ImportFrom,
    app: tauri::AppHandle,
) -> AppResult<Option<ImportPreview>> {
    use tauri_plugin_dialog::DialogExt;
    let (source, title, name, exts): (ImportSource, &str, &str, &[&str]) = match from {
        ImportFrom::PuttyRegistry => return blocking(putty::read_registry).await.map(Some),
        ImportFrom::MobaxtermFile => (
            ImportSource::Mobaxterm,
            "Import MobaXterm sessions",
            "MobaXterm sessions",
            &["mxtsessions", "ini"],
        ),
        ImportFrom::PuttyFile => (
            ImportSource::Putty,
            "Import PuTTY sessions",
            "Registry export",
            &["reg"],
        ),
    };
    let picked = blocking(move || {
        Ok(app
            .dialog()
            .file()
            .set_title(title)
            .add_filter(name, exts)
            .blocking_pick_file())
    })
    .await?;
    let Some(file) = picked else { return Ok(None) };
    let path = file
        .into_path()
        .map_err(|_| AppError::Validation("Unsupported file location.".into()))?;
    let preview = blocking(move || {
        let bytes = read_capped(&path)?;
        Ok(import::parse(source, &bytes))
    })
    .await?;
    tracing::info!(
        source = source.label(),
        sessions = preview.sessions.len(),
        skipped = preview.skipped.len(),
        "import preview"
    );
    Ok(Some(preview))
}

/// Create the chosen sessions.
#[tauri::command]
pub async fn import_apply(
    source: ImportSource,
    sessions: Vec<ImportedSession>,
    skip_existing: bool,
    lib: State<'_, LibraryDb>,
) -> AppResult<ImportResult> {
    let db = lib.db()?;
    let result = db
        .run(move |c| apply::apply(c, source, &sessions, skip_existing))
        .await?;
    tracing::info!(
        source = source.label(),
        created = result.created,
        jump_hosts = result.jump_hosts,
        existing = result.existing,
        failed = result.failed,
        "import done"
    );
    Ok(result)
}
