//! Vault, credential and known-hosts commands.
//!
//! Secrets only ever travel *into* Rust (master password, a typed credential
//! password). Nothing decrypted is returned to the frontend.

use secrecy::SecretString;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::library::LibraryDb;
use super::ssh::VaultState;
use crate::error::{AppError, AppResult};
use crate::ssh::known_hosts::{self, KnownHost};
use crate::storage::folders;
use crate::storage::models::Folder;
use crate::vault::VaultStatus;
use crate::vault::credentials::{self, Credential, MAX_KEY_FILE_BYTES};

/// Run slow vault work (Argon2id) off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("vault task failed: {e}")))?
}

#[tauri::command]
pub async fn vault_status(vault: State<'_, VaultState>) -> AppResult<VaultStatus> {
    let v = vault.get()?;
    blocking(move || v.status()).await
}

#[tauri::command]
pub async fn vault_setup(
    master_password: SecretString,
    remember: bool,
    vault: State<'_, VaultState>,
) -> AppResult<VaultStatus> {
    let v = vault.get()?;
    blocking(move || {
        v.setup(&master_password, remember)?;
        v.status()
    })
    .await
}

#[tauri::command]
pub async fn vault_unlock(
    master_password: SecretString,
    remember: bool,
    vault: State<'_, VaultState>,
) -> AppResult<VaultStatus> {
    let v = vault.get()?;
    blocking(move || {
        v.unlock(&master_password, remember)?;
        v.status()
    })
    .await
}

#[tauri::command]
pub async fn vault_lock(vault: State<'_, VaultState>) -> AppResult<VaultStatus> {
    let v = vault.get()?;
    v.lock();
    blocking(move || v.status()).await
}

#[tauri::command]
pub async fn vault_forget_device(vault: State<'_, VaultState>) -> AppResult<VaultStatus> {
    let v = vault.get()?;
    blocking(move || {
        v.forget_device()?;
        v.status()
    })
    .await
}

#[tauri::command]
pub async fn vault_change_password(
    old_password: SecretString,
    new_password: SecretString,
    vault: State<'_, VaultState>,
) -> AppResult<()> {
    let v = vault.get()?;
    blocking(move || v.change_master_password(&old_password, &new_password)).await
}

#[tauri::command]
pub async fn vault_set_auto_lock(
    minutes: u32,
    vault: State<'_, VaultState>,
) -> AppResult<VaultStatus> {
    let v = vault.get()?;
    blocking(move || {
        v.set_auto_lock(minutes)?;
        v.status()
    })
    .await
}

/// User activity in the UI (throttled by the frontend): resets auto-lock.
#[tauri::command]
pub async fn vault_touch(vault: State<'_, VaultState>) -> AppResult<()> {
    vault.get()?.touch();
    Ok(())
}

// ---- credentials -----------------------------------------------------------

#[tauri::command]
pub async fn credential_list(lib: State<'_, LibraryDb>) -> AppResult<Vec<Credential>> {
    lib.db()?.run(|c| credentials::list(c)).await
}

#[tauri::command]
pub async fn credential_create_password(
    name: String,
    username: String,
    password: SecretString,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
) -> AppResult<Credential> {
    let v = vault.get()?;
    lib.db()?
        .run(move |c| credentials::create_password(c, &v, &name, &username, password))
        .await
}

/// Pick a private key file with a native dialog (opened by Rust) and store it
/// in the vault. Returns `None` if the user cancels the dialog.
#[tauri::command]
pub async fn credential_import_key(
    app: AppHandle,
    name: String,
    username: String,
    passphrase: Option<SecretString>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
) -> AppResult<Option<Credential>> {
    let v = vault.get()?;
    // Fail fast (before the dialog) if the vault is locked.
    v.with_key(|_| Ok(()))?;
    let db = lib.db()?;
    let picked = blocking(move || {
        Ok(app
            .dialog()
            .file()
            .set_title("Choose a private key (OpenSSH, PEM or PuTTY .ppk)")
            .blocking_pick_file())
    })
    .await?;
    let Some(file) = picked else { return Ok(None) };
    let path = file
        .into_path()
        .map_err(|_| AppError::Validation("Unsupported file location.".into()))?;

    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_KEY_FILE_BYTES as u64 {
        return Err(AppError::Validation(
            "That file is too large to be an SSH private key.".into(),
        ));
    }
    let bytes = zeroize::Zeroizing::new(std::fs::read(&path)?);
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| AppError::Validation("That file is not a text key file.".into()))?;
    let key = SecretString::from(text.to_owned());
    let name = if name.trim().is_empty() {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Private key".into())
    } else {
        name
    };
    tracing::info!(file = %path.display(), "importing private key");
    let cred = db
        .run(move |c| credentials::create_key(c, &v, &name, &username, key, passphrase))
        .await?;
    Ok(Some(cred))
}

#[tauri::command]
pub async fn credential_update(
    id: String,
    name: String,
    username: String,
    password: Option<SecretString>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
) -> AppResult<Credential> {
    let v = vault.get()?;
    lib.db()?
        .run(move |c| credentials::update(c, &v, &id, &name, &username, password))
        .await
}

#[tauri::command]
pub async fn credential_delete(id: String, lib: State<'_, LibraryDb>) -> AppResult<()> {
    lib.db()?.run(move |c| credentials::delete(c, &id)).await
}

#[tauri::command]
pub async fn folder_set_default_credential(
    id: String,
    credential_id: Option<String>,
    lib: State<'_, LibraryDb>,
) -> AppResult<Folder> {
    lib.db()?
        .run(move |c| folders::set_default_credential(c, &id, credential_id.as_deref()))
        .await
}

// ---- known hosts -----------------------------------------------------------

#[tauri::command]
pub async fn known_hosts_list(lib: State<'_, LibraryDb>) -> AppResult<Vec<KnownHost>> {
    lib.db()?.run(|c| known_hosts::list(c)).await
}

/// Forget stored keys for `host:port` (after a verified key change).
#[tauri::command]
pub async fn known_hosts_remove(
    host: String,
    port: u32,
    lib: State<'_, LibraryDb>,
) -> AppResult<usize> {
    let host = crate::ssh::validate::host(&host)?;
    let port = crate::ssh::validate::port(port)?;
    let removed = lib
        .db()?
        .run(move |c| known_hosts::remove(c, &host, port))
        .await?;
    tracing::warn!(removed, "stored host keys removed by the user");
    Ok(removed)
}
