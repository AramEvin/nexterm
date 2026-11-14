//! NexTerm Rust core.
//!
//! Layout (see docs/ARCHITECTURE.md):
//! - `commands` - thin IPC layer, input validation only
//! - `session`  - live-session lifecycle + protocol boundary
//! - `ssh`, `sftp`, `vault`, `storage`, `import`, `settings` - core modules
//! - `error`    - `AppError`, returned by every command
//! - `logging`  - structured, rotating logs

pub mod commands;
pub mod error;
pub mod import;
pub mod logging;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod ssh;
pub mod storage;
pub mod vault;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> anyhow::Result<()> {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().data_dir()?;
            let guard = logging::init(logging::log_dir(&data_dir))?;
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                log_dir = %guard.log_dir.display(),
                "NexTerm starting"
            );
            app.manage(guard);
            app.manage(session::SessionManager::new());
            app.manage(ssh::SshConnections::new());
            let db_path = storage::db_path(&data_dir);
            let db = storage::Database::open(&db_path);
            if db.is_ok() {
                tracing::info!(path = %db_path.display(), "session database opened");
            }
            app.manage(commands::library::LibraryDb::new(db));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::get_app_info,
            commands::library::library_list,
            commands::library::folder_create,
            commands::library::folder_rename,
            commands::library::folder_move,
            commands::library::folder_delete,
            commands::library::session_create,
            commands::library::session_update,
            commands::library::session_delete,
            commands::library::session_duplicate,
            commands::library::session_move,
            commands::library::session_set_favorite,
            commands::library::session_mark_connected,
            commands::ssh::ssh_connect,
            commands::ssh::ssh_write,
            commands::ssh::ssh_resize,
            commands::ssh::ssh_disconnect,
            commands::ssh::ssh_reconnect,
            commands::ssh::ssh_duplicate,
            commands::ssh::ssh_forget,
        ])
        .run(tauri::generate_context!())?;

    Ok(())
}
