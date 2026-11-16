//! File-manager IPC commands: open/close SFTP sessions, browse and change
//! files on either side, and run transfers.
//!
//! Every path and name is validated here before it reaches `sftp/`. A pane
//! is addressed by an [`Endpoint`]: the local disk, or an open SFTP session
//! (by opaque `fsId`; the connection and its credentials stay in Rust).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::commands::library::LibraryDb;
use crate::commands::ssh::{ChannelPrompts, SshContext, VaultState, parse_connection_id, resolve};
use crate::error::{AppError, AppResult};
use crate::sftp::fs::{Fs, blocking};
use crate::sftp::paths::{self, base_name, remote_join, remote_parent};
use crate::sftp::transfer::{Conflict, EventSink, Job, TransferEvent};
use crate::sftp::{FileSessions, Listing, RemoteFs, Transfers, local};
use crate::ssh::connections::ConnectSpec;
use crate::ssh::prompt::SshPrompt;
use crate::ssh::{Interaction, SshConnectParams, SshConnections, validate};
use crate::storage::sessions;

/// Most items one delete / chmod / transfer request may name.
const MAX_PATHS: usize = 10_000;

/// Which side of the file manager a command is about.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Endpoint {
    Local,
    #[serde(rename_all = "camelCase")]
    Remote {
        fs_id: String,
    },
}

/// Returned when a remote pane connects.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsInfo {
    pub fs_id: String,
    /// The login folder.
    pub home: String,
    /// "user@host:port".
    pub label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SftpOpenSavedRequest {
    pub session_id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferRequest {
    pub source: Endpoint,
    pub sources: Vec<String>,
    pub target: Endpoint,
    pub target_dir: String,
    pub conflict: Conflict,
}

/// Resolved side: the file system plus whether its paths are local.
enum Side {
    Local,
    Remote(Arc<RemoteFs>),
}

async fn side(endpoint: &Endpoint, sessions: &FileSessions) -> AppResult<Side> {
    match endpoint {
        Endpoint::Local => Ok(Side::Local),
        Endpoint::Remote { fs_id } => Ok(Side::Remote(sessions.get(parse_fs_id(fs_id)?).await?)),
    }
}

fn parse_fs_id(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw).map_err(|_| AppError::Validation("Invalid file session id.".into()))
}

fn check_count(paths: &[String]) -> AppResult<()> {
    if paths.is_empty() {
        return Err(AppError::Validation("Nothing selected.".into()));
    }
    if paths.len() > MAX_PATHS {
        return Err(AppError::Validation(format!(
            "Too many items at once (max {MAX_PATHS})."
        )));
    }
    Ok(())
}

/// Connect a pane to a remote server and start SFTP.
async fn open_remote(
    spec: ConnectSpec,
    on_prompt: Channel<SshPrompt>,
    lib: &LibraryDb,
    vault: &VaultState,
    ctx: &SshContext,
    files: &FileSessions,
) -> AppResult<FsInfo> {
    let resolved = resolve(spec, lib, vault).await?;
    let saved_id = resolved.saved_session_id.clone();
    let params = SshConnectParams {
        host: resolved.host,
        port: resolved.port,
        username: resolved.username,
        auth: resolved.auth,
        interaction: Interaction {
            broker: Arc::clone(&ctx.broker),
            prompts: Arc::new(ChannelPrompts(on_prompt)),
            known_hosts: lib.db().ok(),
            openssh_known_hosts: ctx.openssh_known_hosts.clone(),
            agent_socket: None,
        },
    };
    let fs = RemoteFs::open(params).await?;
    let home = match fs.home().await {
        Ok(h) => h,
        Err(_) => "/".to_owned(),
    };
    let label = fs.label.clone();
    let id = files.insert(fs).await?;
    if let (Some(sid), Ok(db)) = (saved_id, lib.db()) {
        let _ = db.run(move |c| sessions::mark_connected(c, &sid)).await;
    }
    Ok(FsInfo {
        fs_id: id.to_string(),
        home,
        label,
    })
}

/// Open a saved session in a file-manager pane.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sftp_open_saved(
    request: SftpOpenSavedRequest,
    on_prompt: Channel<SshPrompt>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
    files: State<'_, FileSessions>,
) -> AppResult<FsInfo> {
    if let Some(pw) = &request.password {
        validate::password_len(pw.expose_secret().len())?;
    }
    let spec = ConnectSpec::Saved {
        session_id: crate::storage::validate_id(&request.session_id)?,
        username: request.username.filter(|u| !u.trim().is_empty()),
        password: request.password.filter(|p| !p.expose_secret().is_empty()),
    };
    open_remote(spec, on_prompt, &lib, &vault, &ctx, &files).await
}

/// Open the server of a terminal tab (reuses that tab's remembered login,
/// so a quick-connect password is not asked again).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sftp_open_connection(
    connection_id: String,
    on_prompt: Channel<SshPrompt>,
    connections: State<'_, SshConnections>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
    files: State<'_, FileSessions>,
) -> AppResult<FsInfo> {
    let spec = connections
        .get_copy(parse_connection_id(&connection_id)?)
        .await?;
    open_remote(spec, on_prompt, &lib, &vault, &ctx, &files).await
}

#[tauri::command]
pub async fn sftp_close(fs_id: String, files: State<'_, FileSessions>) -> AppResult<()> {
    files.remove(parse_fs_id(&fs_id)?).await;
    Ok(())
}

/// Starting folder of a pane.
#[tauri::command]
pub async fn fs_home(endpoint: Endpoint, files: State<'_, FileSessions>) -> AppResult<String> {
    match side(&endpoint, &files).await? {
        Side::Local => Ok(local::home()),
        Side::Remote(r) => r.home().await,
    }
}

#[tauri::command]
pub async fn fs_list(
    endpoint: Endpoint,
    path: String,
    files: State<'_, FileSessions>,
) -> AppResult<Listing> {
    match side(&endpoint, &files).await? {
        Side::Local => blocking(move || local::list(&path)).await,
        Side::Remote(r) => r.list(&paths::remote_path(&path)?).await,
    }
}

/// Create folder `name` inside `dir`; returns its path.
#[tauri::command]
pub async fn fs_mkdir(
    endpoint: Endpoint,
    dir: String,
    name: String,
    files: State<'_, FileSessions>,
) -> AppResult<String> {
    match side(&endpoint, &files).await? {
        Side::Local => {
            let target = paths::local_path(&dir)?.join(paths::new_name(&name, true)?);
            let out = target.display().to_string();
            blocking(move || local::mkdir(&target)).await?;
            Ok(out)
        }
        Side::Remote(r) => {
            let target = remote_join(&paths::remote_path(&dir)?, &paths::new_name(&name, false)?);
            r.mkdir(&target).await?;
            Ok(target)
        }
    }
}

/// Rename an item in place; returns the new path.
#[tauri::command]
pub async fn fs_rename(
    endpoint: Endpoint,
    path: String,
    new_name: String,
    files: State<'_, FileSessions>,
) -> AppResult<String> {
    match side(&endpoint, &files).await? {
        Side::Local => {
            let from = paths::local_path(&path)?;
            let parent = from
                .parent()
                .ok_or_else(|| AppError::Validation("A drive can't be renamed.".into()))?;
            let to = parent.join(paths::new_name(&new_name, true)?);
            let out = to.display().to_string();
            blocking(move || local::rename(&from, &to)).await?;
            Ok(out)
        }
        Side::Remote(r) => {
            let from = paths::remote_path(&path)?;
            let parent = remote_parent(&from)
                .ok_or_else(|| AppError::Validation("The root folder can't be renamed.".into()))?;
            let to = remote_join(&parent, &paths::new_name(&new_name, false)?);
            r.rename(&from, &to).await?;
            Ok(to)
        }
    }
}

/// Delete items (folders recursively). Returns how many top-level items were deleted.
#[tauri::command]
pub async fn fs_delete(
    endpoint: Endpoint,
    paths: Vec<String>,
    files: State<'_, FileSessions>,
) -> AppResult<u64> {
    check_count(&paths)?;
    let side = side(&endpoint, &files).await?;
    let mut deleted = 0;
    for raw in &paths {
        match &side {
            Side::Local => {
                let p = paths::local_path(raw)?;
                if p.parent().is_none() {
                    return Err(AppError::Validation(
                        "A whole drive can't be deleted.".into(),
                    ));
                }
                blocking(move || local::remove(&p)).await?;
            }
            Side::Remote(r) => {
                let p = paths::remote_path(raw)?;
                if remote_parent(&p).is_none() {
                    return Err(AppError::Validation(
                        "The root folder can't be deleted.".into(),
                    ));
                }
                r.remove(&p).await?;
            }
        }
        deleted += 1;
    }
    tracing::info!(
        items = deleted,
        local = matches!(side, Side::Local),
        "items deleted"
    );
    Ok(deleted)
}

/// Set permission bits (e.g. `0o644`) on each item.
#[tauri::command]
pub async fn fs_chmod(
    endpoint: Endpoint,
    paths: Vec<String>,
    mode: u32,
    files: State<'_, FileSessions>,
) -> AppResult<()> {
    check_count(&paths)?;
    if mode > 0o7777 {
        return Err(AppError::Validation("Invalid permissions.".into()));
    }
    let side = side(&endpoint, &files).await?;
    for raw in &paths {
        match &side {
            Side::Local => {
                let p = paths::local_path(raw)?;
                blocking(move || local::chmod(&p, mode)).await?;
            }
            Side::Remote(r) => r.chmod(&paths::remote_path(raw)?, mode).await?,
        }
    }
    Ok(())
}

struct ChannelEvents(Channel<TransferEvent>);

impl EventSink for ChannelEvents {
    fn send(&self, event: TransferEvent) {
        if let Err(e) = self.0.send(event) {
            tracing::debug!(error = %e, "transfer event channel closed");
        }
    }
}

fn validate_dir(fs: &Fs, raw: &str) -> AppResult<String> {
    match fs {
        Fs::Local => {
            let p: PathBuf = paths::local_path(raw)?;
            Ok(p.display().to_string())
        }
        Fs::Remote(_) => paths::remote_path(raw),
    }
}

/// Queue a copy. Returns the transfer id; progress arrives on `on_event`.
#[tauri::command]
pub async fn transfer_start(
    request: TransferRequest,
    on_event: Channel<TransferEvent>,
    files: State<'_, FileSessions>,
    transfers: State<'_, Arc<Transfers>>,
) -> AppResult<String> {
    check_count(&request.sources)?;
    let to_fs = |s: Side| match s {
        Side::Local => Fs::Local,
        Side::Remote(r) => Fs::Remote(r),
    };
    let source = to_fs(side(&request.source, &files).await?);
    let target = to_fs(side(&request.target, &files).await?);
    let sources = request
        .sources
        .iter()
        .map(|p| {
            let p = validate_dir(&source, p)?;
            let top = match &source {
                Fs::Local => Path::new(&p).parent().is_none(),
                Fs::Remote(_) => remote_parent(&p).is_none(),
            };
            if top || base_name(&p).is_empty() {
                return Err(AppError::Validation(
                    "A whole drive or the root folder can't be copied; open it and select items."
                        .into(),
                ));
            }
            Ok(p)
        })
        .collect::<AppResult<Vec<_>>>()?;
    let target_dir = validate_dir(&target, &request.target_dir)?;
    let job = Job {
        source,
        sources,
        target,
        target_dir,
        conflict: request.conflict,
    };
    let id = transfers.start(job, Arc::new(ChannelEvents(on_event)))?;
    Ok(id.to_string())
}

#[tauri::command]
pub async fn transfer_cancel(id: String, transfers: State<'_, Arc<Transfers>>) -> AppResult<bool> {
    let id =
        Uuid::parse_str(&id).map_err(|_| AppError::Validation("Invalid transfer id.".into()))?;
    Ok(transfers.cancel(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_shape() {
        let e: Endpoint = serde_json::from_value(serde_json::json!({"kind": "local"})).unwrap();
        assert!(matches!(e, Endpoint::Local));
        let e: Endpoint =
            serde_json::from_value(serde_json::json!({"kind": "remote", "fsId": "x"})).unwrap();
        assert!(matches!(e, Endpoint::Remote { .. }));
        assert!(serde_json::from_value::<Endpoint>(serde_json::json!({"kind": "ftp"})).is_err());
    }

    #[test]
    fn counts() {
        assert!(check_count(&[]).is_err());
        assert!(check_count(&vec![String::new(); MAX_PATHS + 1]).is_err());
        assert!(check_count(&["a".into()]).is_ok());
    }
}
