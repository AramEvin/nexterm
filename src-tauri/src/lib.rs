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

use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, Manager};

/// How often the auto-lock timer is checked.
const AUTO_LOCK_CHECK: Duration = Duration::from_secs(15);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> anyhow::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            app.manage(sftp::FileSessions::new());
            app.manage(Arc::new(sftp::Transfers::new()));
            app.manage(Arc::new(sftp::Edits::new()));
            // Local copies of edited files left behind by an earlier run.
            std::thread::spawn(|| {
                sftp::edit::sweep_stale(std::time::Duration::from_secs(24 * 3600));
            });
            let db_path = storage::db_path(&data_dir);
            let db = storage::Database::open(&db_path);
            if db.is_ok() {
                tracing::info!(path = %db_path.display(), "session database opened");
            }
            let vault = db.as_ref().ok().map(|db| {
                Arc::new(vault::Vault::new(
                    db.clone(),
                    Arc::new(vault::keychain::OsKeyStore),
                ))
            });
            if let Some(v) = &vault {
                if let Err(e) = v.unlock_from_device() {
                    tracing::warn!(error = %e, "could not unlock the vault from the device keychain");
                }
                // Auto-lock: check idle time periodically and tell the UI.
                let (v, handle) = (Arc::clone(v), app.handle().clone());
                tauri::async_runtime::spawn(async move {
                    let mut tick = tokio::time::interval(AUTO_LOCK_CHECK);
                    loop {
                        tick.tick().await;
                        if matches!(v.lock_if_idle(), Ok(true)) {
                            let _ = handle.emit("vault-locked", ());
                        }
                    }
                });
            }
            app.manage(commands::ssh::VaultState(vault));
            app.manage(commands::ssh::SshContext {
                broker: Arc::new(ssh::PromptBroker::new()),
                openssh_known_hosts: app
                    .path()
                    .home_dir()
                    .ok()
                    .map(|h| h.join(".ssh").join("known_hosts")),
            });
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
            commands::ssh::ssh_connect_saved,
            commands::ssh::ssh_prompt_respond,
            commands::vault::vault_status,
            commands::vault::vault_setup,
            commands::vault::vault_unlock,
            commands::vault::vault_lock,
            commands::vault::vault_forget_device,
            commands::vault::vault_change_password,
            commands::vault::vault_set_auto_lock,
            commands::vault::vault_touch,
            commands::vault::credential_list,
            commands::vault::credential_create_password,
            commands::vault::credential_import_key,
            commands::vault::credential_update,
            commands::vault::credential_delete,
            commands::vault::folder_set_default_credential,
            commands::vault::known_hosts_list,
            commands::vault::known_hosts_remove,
            commands::sftp::sftp_open_saved,
            commands::sftp::sftp_open_connection,
            commands::sftp::sftp_close,
            commands::sftp::fs_home,
            commands::sftp::fs_list,
            commands::sftp::fs_mkdir,
            commands::sftp::fs_rename,
            commands::sftp::fs_delete,
            commands::sftp::fs_chmod,
            commands::sftp::transfer_start,
            commands::sftp::transfer_cancel,
            commands::sftp::edit_start,
            commands::sftp::edit_stop,
            commands::sftp::pick_local_folder,
            commands::sftp::pick_local_files,
        ])
        .run(tauri::generate_context!())?;

    Ok(())
}
