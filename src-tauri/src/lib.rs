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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app::get_app_info])
        .run(tauri::generate_context!())?;

    Ok(())
}
