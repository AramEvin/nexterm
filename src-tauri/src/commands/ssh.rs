//! SSH IPC commands. Validation happens here; logic lives in `ssh/` and `session/`.
//!
//! Each connection gets three `tauri::ipc::Channel`s from the frontend:
//! raw terminal bytes (an `ArrayBuffer`, not a JSON number array), status
//! changes, and prompts (host-key decisions, 2FA codes) that the UI answers
//! with `ssh_prompt_respond`.

use crate::session::monitor::{MonitorSink, ResourceEvent};
use std::path::PathBuf;
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::{Channel, Response};
use uuid::Uuid;

use crate::commands::library::LibraryDb;
use crate::error::{AppError, AppResult};
use crate::session::SessionManager;
use crate::session::protocol::{OutputSink, SessionCommand, SessionStatus, TerminalProtocol};
use crate::ssh::connections::ConnectSpec;
use crate::ssh::prompt::{PromptResponse, PromptSink, SshPrompt};
use crate::ssh::{
    Interaction, PromptBroker, SshAuth, SshConnectParams, SshConnections, SshProtocol, validate,
};
use crate::storage::sessions;
use crate::vault::{Vault, credentials};

/// Largest single `ssh_write` payload (a big paste). 1 MiB.
const MAX_WRITE_BYTES: usize = 1024 * 1024;

/// Shared SSH state: the prompt broker and the user's OpenSSH known_hosts path.
pub struct SshContext {
    pub broker: Arc<PromptBroker>,
    pub openssh_known_hosts: Option<PathBuf>,
}

/// The vault, or `None` if the database could not be opened.
pub struct VaultState(pub Option<Arc<Vault>>);

impl VaultState {
    pub fn get(&self) -> AppResult<Arc<Vault>> {
        self.0.clone().ok_or_else(|| {
            AppError::Validation("The vault is unavailable (database error).".into())
        })
    }
}

/// Quick-connect request from the frontend.
///
/// `password` is deserialized straight into a `SecretString`: it is zeroized
/// on drop and prints as `[REDACTED]` if ever debug-formatted.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SshConnectRequest {
    pub host: String,
    pub port: u32,
    pub username: String,
    pub password: SecretString,
    pub cols: u16,
    pub rows: u16,
}

/// Connect to a saved session. `username` / `password` are only sent when a
/// previous attempt returned `CREDENTIALS_REQUIRED`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SshConnectSavedRequest {
    pub session_id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<SecretString>,
    pub cols: u16,
    pub rows: u16,
}

/// Bridges the protocol-agnostic `OutputSink` to Tauri IPC channels.
struct ChannelSink {
    output: Channel<Response>,
    status: Channel<SessionStatus>,
}

impl OutputSink for ChannelSink {
    fn data(&self, bytes: Vec<u8>) -> bool {
        self.output.send(Response::new(bytes)).is_ok()
    }

    fn status(&self, status: SessionStatus) {
        if let Err(e) = self.status.send(status) {
            tracing::debug!(error = %e, "status channel closed");
        }
    }
}

pub(crate) struct ChannelPrompts(pub(crate) Channel<SshPrompt>);

impl PromptSink for ChannelPrompts {
    fn send(&self, prompt: SshPrompt) -> bool {
        self.0.send(prompt).is_ok()
    }
}

/// The three per-connection channels supplied by the frontend.
pub struct Channels {
    output: Channel<Response>,
    status: Channel<SessionStatus>,
    prompts: Channel<SshPrompt>,
}

fn parse_session_id(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw).map_err(|_| AppError::Validation("Invalid session id.".into()))
}

/// Returned by every command that opens a session.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshSessionInfo {
    /// The live session (changes on every reconnect).
    pub session_id: String,
    /// Opaque handle to the remembered connection parameters; owned by one tab.
    pub connection_id: String,
}

pub(crate) fn parse_connection_id(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw).map_err(|_| AppError::Validation("Invalid connection id.".into()))
}

pub(crate) struct Resolved {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    pub saved_session_id: Option<String>,
}

/// Turn a spec into concrete parameters. For saved sessions this reads the
/// session, finds its credential (own or inherited from a folder) and
/// decrypts it with the vault - all inside Rust.
pub(crate) async fn resolve(
    spec: ConnectSpec,
    lib: &LibraryDb,
    vault: &VaultState,
) -> AppResult<Resolved> {
    let (session_id, typed_user, typed_password) = match spec {
        ConnectSpec::Direct {
            host,
            port,
            username,
            auth,
        } => {
            return Ok(Resolved {
                host,
                port,
                username,
                auth,
                saved_session_id: None,
            });
        }
        ConnectSpec::Saved {
            session_id,
            username,
            password,
        } => (session_id, username, password),
    };
    let db = lib.db()?;
    let vault = vault.0.clone();
    db.run(move |c| {
        let session = sessions::get(c, &session_id)?;
        if session.protocol.is_file_only() {
            return Err(AppError::Validation(
                "This is an FTP session: open it in the file manager.".into(),
            ));
        }
        let credential = match credentials::resolve_for_session(c, &session.id)? {
            Some(cid) if session.auth_method != "agent" => {
                let vault = vault.as_ref().ok_or(AppError::VaultLocked)?;
                Some(credentials::load_secret(c, vault, &cid)?)
            }
            _ => None,
        };
        let cred_username = credential
            .as_ref()
            .map(|(meta, _)| meta.username.clone())
            .filter(|u| !u.is_empty());
        let username = typed_user
            .filter(|u| !u.trim().is_empty())
            .or_else(|| Some(session.username.clone()).filter(|u| !u.is_empty()))
            .or(cred_username)
            .ok_or_else(|| {
                AppError::CredentialsRequired("Enter a username for this session.".into())
            })?;
        let username = validate::username(&username)?;

        let stored_password = credential.as_ref().and_then(|(_, s)| {
            s.password
                .as_ref()
                .map(|p| SecretString::from(p.expose_secret().to_owned()))
        });
        let auth = match session.auth_method.as_str() {
            "publickey" => {
                let Some((_, secret)) = &credential else {
                    return Err(AppError::CredentialsRequired(
                        "This session uses a private key, but no key credential is linked. \
                         Edit the session and choose one."
                            .into(),
                    ));
                };
                SshAuth::PublicKey(Arc::new(secret.decode_key()?))
            }
            "agent" => SshAuth::Agent,
            "keyboard-interactive" => SshAuth::KeyboardInteractive {
                password: stored_password.or(typed_password),
            },
            _ => match stored_password.or(typed_password) {
                Some(pw) => SshAuth::Password(pw),
                None => {
                    return Err(AppError::CredentialsRequired(format!(
                        "Enter the password for {}.",
                        session.name
                    )));
                }
            },
        };
        Ok(Resolved {
            host: session.host,
            port: session.port,
            username,
            auth,
            saved_session_id: Some(session.id),
        })
    })
    .await
}

/// Open the session described by `spec` and register it with the manager.
async fn open_spec(
    spec: ConnectSpec,
    cols: u16,
    rows: u16,
    channels: Channels,
    deps: &Deps<'_>,
) -> AppResult<Uuid> {
    let size = validate::terminal_size(cols, rows)?;
    let resolved = resolve(spec, deps.lib, deps.vault).await?;
    let interaction = Interaction {
        broker: Arc::clone(&deps.ctx.broker),
        prompts: Arc::new(ChannelPrompts(channels.prompts)),
        known_hosts: deps.lib.db().ok(),
        openssh_known_hosts: deps.ctx.openssh_known_hosts.clone(),
        agent_socket: None,
    };
    let saved_id = resolved.saved_session_id.clone();
    let params = SshConnectParams {
        host: resolved.host,
        port: resolved.port,
        username: resolved.username,
        auth: resolved.auth,
        interaction,
    };
    let sink: Arc<dyn OutputSink> = Arc::new(ChannelSink {
        output: channels.output,
        status: channels.status,
    });
    let handle = SshProtocol::open(params, size, Arc::clone(&sink)).await?;
    let id = deps.manager.register(handle, sink).await;
    tracing::info!(session_id = %id, "session registered");

    if let (Some(sid), Ok(db)) = (saved_id, deps.lib.db()) {
        let _ = db.run(move |c| sessions::mark_connected(c, &sid)).await;
    }
    Ok(id)
}

/// Managed state every connect command needs.
struct Deps<'a> {
    manager: &'a Arc<SessionManager>,
    lib: &'a LibraryDb,
    vault: &'a VaultState,
    ctx: &'a SshContext,
}

/// Open, then remember the spec for reconnect/duplicate (only after success,
/// so a mistyped password is never kept).
async fn open_and_remember(
    spec: ConnectSpec,
    cols: u16,
    rows: u16,
    channels: Channels,
    deps: &Deps<'_>,
    connections: &SshConnections,
) -> AppResult<SshSessionInfo> {
    let remembered = spec.duplicate();
    let session_id = open_spec(spec, cols, rows, channels, deps).await?;
    let connection_id = match connections.insert(remembered).await {
        Ok(id) => id,
        Err(e) => {
            let _ = deps.manager.send(session_id, SessionCommand::Close).await;
            return Err(e);
        }
    };
    Ok(SshSessionInfo {
        session_id: session_id.to_string(),
        connection_id: connection_id.to_string(),
    })
}

/// Quick connect: host, user and password typed by the user.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each piece of state separately.
pub async fn ssh_connect(
    request: SshConnectRequest,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    on_prompt: Channel<SshPrompt>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
) -> AppResult<SshSessionInfo> {
    validate::password_len(request.password.expose_secret().len())?;
    let spec = ConnectSpec::Direct {
        host: validate::host(&request.host)?,
        port: validate::port(request.port)?,
        username: validate::username(&request.username)?,
        auth: SshAuth::Password(request.password),
    };
    let channels = Channels {
        output: on_output,
        status: on_status,
        prompts: on_prompt,
    };
    let deps = Deps {
        manager: &manager,
        lib: &lib,
        vault: &vault,
        ctx: &ctx,
    };
    open_and_remember(
        spec,
        request.cols,
        request.rows,
        channels,
        &deps,
        &connections,
    )
    .await
}

/// Connect to a saved session (credentials resolved from the vault in Rust).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ssh_connect_saved(
    request: SshConnectSavedRequest,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    on_prompt: Channel<SshPrompt>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
) -> AppResult<SshSessionInfo> {
    if let Some(pw) = &request.password {
        validate::password_len(pw.expose_secret().len())?;
    }
    let spec = ConnectSpec::Saved {
        session_id: crate::storage::validate_id(&request.session_id)?,
        username: request.username.filter(|u| !u.trim().is_empty()),
        password: request.password.filter(|p| !p.expose_secret().is_empty()),
    };
    let channels = Channels {
        output: on_output,
        status: on_status,
        prompts: on_prompt,
    };
    let deps = Deps {
        manager: &manager,
        lib: &lib,
        vault: &vault,
        ctx: &ctx,
    };
    open_and_remember(
        spec,
        request.cols,
        request.rows,
        channels,
        &deps,
        &connections,
    )
    .await
}

/// Open a new session for an existing tab, reusing its remembered spec.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ssh_reconnect(
    connection_id: String,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    on_prompt: Channel<SshPrompt>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
) -> AppResult<SshSessionInfo> {
    let cid = parse_connection_id(&connection_id)?;
    let spec = connections.get_copy(cid).await?;
    tracing::info!(connection_id = %cid, "reconnect requested");
    let channels = Channels {
        output: on_output,
        status: on_status,
        prompts: on_prompt,
    };
    let deps = Deps {
        manager: &manager,
        lib: &lib,
        vault: &vault,
        ctx: &ctx,
    };
    let session_id = open_spec(spec, cols, rows, channels, &deps).await?;
    Ok(SshSessionInfo {
        session_id: session_id.to_string(),
        connection_id,
    })
}

/// Open the same connection in a new tab (gets its own connection id).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ssh_duplicate(
    connection_id: String,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    on_prompt: Channel<SshPrompt>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
) -> AppResult<SshSessionInfo> {
    let cid = parse_connection_id(&connection_id)?;
    let spec = connections.get_copy(cid).await?;
    tracing::info!(source_connection_id = %cid, "duplicate requested");
    let channels = Channels {
        output: on_output,
        status: on_status,
        prompts: on_prompt,
    };
    let deps = Deps {
        manager: &manager,
        lib: &lib,
        vault: &vault,
        ctx: &ctx,
    };
    open_and_remember(spec, cols, rows, channels, &deps, &connections).await
}

/// The UI's answer to a prompt (host-key decision or 2FA answers).
#[tauri::command]
pub async fn ssh_prompt_respond(
    prompt_id: String,
    response: PromptResponse,
    ctx: State<'_, SshContext>,
) -> AppResult<()> {
    ctx.broker.respond(&prompt_id, response)
}

/// The tab was closed: wipe its remembered spec (typed passwords included).
#[tauri::command]
pub async fn ssh_forget(
    connection_id: String,
    connections: State<'_, SshConnections>,
) -> AppResult<()> {
    let cid = parse_connection_id(&connection_id)?;
    if connections.remove(cid).await {
        tracing::debug!(connection_id = %cid, "connection parameters forgotten");
    }
    Ok(())
}

/// Send keystrokes / pasted text to a session.
#[tauri::command]
pub async fn ssh_write(
    session_id: String,
    data: String,
    manager: State<'_, Arc<SessionManager>>,
) -> AppResult<()> {
    let id = parse_session_id(&session_id)?;
    if data.len() > MAX_WRITE_BYTES {
        return Err(AppError::Validation(
            "Input is too large (max 1 MiB at once).".into(),
        ));
    }
    manager
        .send(id, SessionCommand::Write(data.into_bytes()))
        .await
}

/// Tell the remote PTY the terminal size changed.
#[tauri::command]
pub async fn ssh_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    manager: State<'_, Arc<SessionManager>>,
) -> AppResult<()> {
    let id = parse_session_id(&session_id)?;
    let size = validate::terminal_size(cols, rows)?;
    manager.send(id, SessionCommand::Resize(size)).await
}

struct ChannelMonitor(Channel<ResourceEvent>);

impl MonitorSink for ChannelMonitor {
    fn event(&self, event: ResourceEvent) -> bool {
        self.0.send(event).is_ok()
    }
}

/// Start the live resource bar of a session (replaces a running one). The
/// readings run on a separate exec channel of the same connection
/// (see `ssh::monitor`).
#[tauri::command]
pub async fn ssh_monitor_start(
    session_id: String,
    on_event: Channel<ResourceEvent>,
    manager: State<'_, Arc<SessionManager>>,
) -> AppResult<()> {
    let id = parse_session_id(&session_id)?;
    let sink: Arc<dyn MonitorSink> = Arc::new(ChannelMonitor(on_event));
    manager.send(id, SessionCommand::Monitor(Some(sink))).await
}

/// Stop the resource bar readings of a session.
#[tauri::command]
pub async fn ssh_monitor_stop(
    session_id: String,
    manager: State<'_, Arc<SessionManager>>,
) -> AppResult<()> {
    let id = parse_session_id(&session_id)?;
    match manager.send(id, SessionCommand::Monitor(None)).await {
        Ok(()) | Err(AppError::NotFound(_)) => Ok(()), // session already gone
        Err(e) => Err(e),
    }
}

/// Close a session.
#[tauri::command]
pub async fn ssh_disconnect(
    session_id: String,
    manager: State<'_, Arc<SessionManager>>,
) -> AppResult<()> {
    let id = parse_session_id(&session_id)?;
    tracing::info!(session_id = %id, "disconnect requested");
    match manager.send(id, SessionCommand::Close).await {
        // Already gone is fine: the goal (session closed) is met.
        Ok(()) | Err(AppError::NotFound(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_request_debug_redacts_password() {
        let req: SshConnectRequest = serde_json::from_value(serde_json::json!({
            "host": "h", "port": 22, "username": "u",
            "password": "hunter2-super-secret", "cols": 80, "rows": 24
        }))
        .unwrap();
        let dbg = format!("{req:?}");
        assert!(!dbg.contains("hunter2"), "password leaked in Debug: {dbg}");
    }

    #[test]
    fn rejects_bad_session_ids() {
        assert!(parse_session_id("not-a-uuid").is_err());
        assert!(parse_session_id(&Uuid::new_v4().to_string()).is_ok());
    }
}
