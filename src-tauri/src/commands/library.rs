//! Saved sessions and folders ("library") IPC commands.
//!
//! Validation happens in `storage::validate`; these commands only route
//! requests to the repositories on the blocking pool.

use serde::Deserialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::storage::models::{Folder, Library, SavedSession, SessionInput};
use crate::storage::{Database, folders, sessions};

/// The database, or why it could not be opened. The app keeps running without
/// it (quick connect still works) and the sidebar shows the error.
pub struct LibraryDb(Result<Database, String>);

impl LibraryDb {
    pub fn new(result: AppResult<Database>) -> Self {
        Self(result.map_err(|e| {
            tracing::error!(error = %e, "session database unavailable");
            e.user_message()
        }))
    }

    pub(crate) fn db(&self) -> AppResult<Database> {
        self.0
            .as_ref()
            .cloned()
            .map_err(|msg| AppError::Validation(format!("Saved sessions are unavailable: {msg}")))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoveRequest {
    pub id: String,
    /// Target folder; `None` = top level.
    pub parent_id: Option<String>,
    /// Sibling to insert before; `None` = at the end.
    pub before_id: Option<String>,
}

#[tauri::command]
pub async fn library_list(lib: State<'_, LibraryDb>) -> AppResult<Library> {
    lib.db()?
        .run(|c| {
            Ok(Library {
                folders: folders::list(c)?,
                sessions: sessions::list(c)?,
            })
        })
        .await
}

// ---- folders ---------------------------------------------------------------

#[tauri::command]
pub async fn folder_create(
    lib: State<'_, LibraryDb>,
    parent_id: Option<String>,
    name: String,
) -> AppResult<Folder> {
    lib.db()?
        .run(move |c| folders::create(c, parent_id.as_deref(), &name))
        .await
}

#[tauri::command]
pub async fn folder_rename(
    lib: State<'_, LibraryDb>,
    id: String,
    name: String,
) -> AppResult<Folder> {
    lib.db()?.run(move |c| folders::rename(c, &id, &name)).await
}

#[tauri::command]
pub async fn folder_move(lib: State<'_, LibraryDb>, request: MoveRequest) -> AppResult<()> {
    lib.db()?
        .run(move |c| {
            folders::move_to(
                c,
                &request.id,
                request.parent_id.as_deref(),
                request.before_id.as_deref(),
            )
        })
        .await
}

/// Returns how many sessions were deleted along with the folder.
#[tauri::command]
pub async fn folder_delete(lib: State<'_, LibraryDb>, id: String) -> AppResult<usize> {
    let removed = lib.db()?.run(move |c| folders::delete(c, &id)).await?;
    tracing::info!(sessions_removed = removed, "folder deleted");
    Ok(removed)
}

// ---- sessions --------------------------------------------------------------

#[tauri::command]
pub async fn session_create(
    lib: State<'_, LibraryDb>,
    input: SessionInput,
) -> AppResult<SavedSession> {
    lib.db()?.run(move |c| sessions::create(c, &input)).await
}

#[tauri::command]
pub async fn session_update(
    lib: State<'_, LibraryDb>,
    id: String,
    input: SessionInput,
) -> AppResult<SavedSession> {
    lib.db()?
        .run(move |c| sessions::update(c, &id, &input))
        .await
}

#[tauri::command]
pub async fn session_delete(lib: State<'_, LibraryDb>, id: String) -> AppResult<()> {
    lib.db()?.run(move |c| sessions::delete(c, &id)).await
}

#[tauri::command]
pub async fn session_duplicate(lib: State<'_, LibraryDb>, id: String) -> AppResult<SavedSession> {
    lib.db()?.run(move |c| sessions::duplicate(c, &id)).await
}

#[tauri::command]
pub async fn session_move(lib: State<'_, LibraryDb>, request: MoveRequest) -> AppResult<()> {
    lib.db()?
        .run(move |c| {
            sessions::move_to(
                c,
                &request.id,
                request.parent_id.as_deref(),
                request.before_id.as_deref(),
            )
        })
        .await
}

#[tauri::command]
pub async fn session_set_favorite(
    lib: State<'_, LibraryDb>,
    id: String,
    favorite: bool,
) -> AppResult<()> {
    lib.db()?
        .run(move |c| sessions::set_favorite(c, &id, favorite))
        .await
}

#[tauri::command]
pub async fn session_mark_connected(lib: State<'_, LibraryDb>, id: String) -> AppResult<()> {
    lib.db()?
        .run(move |c| sessions::mark_connected(c, &id))
        .await
}
