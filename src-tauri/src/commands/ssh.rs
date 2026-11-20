//! SSH IPC commands. Validation happens here; logic lives in `ssh/` and `session/`.
//!
//! Each connection gets three `tauri::ipc::Channel`s from the frontend:
//! raw terminal bytes (an `ArrayBuffer`, not a JSON number array), status
//! changes, and prompts (host-key decisions, 2FA codes) that the UI answers
//! with `ssh_prompt_respond`.

use crate::local::LocalProtocol;
use crate::serial::SerialProtocol;
use crate::session::monitor::{MonitorSink, ResourceEvent};
use crate::session::options::{LocalOptions, SerialOptions};
use crate::telnet::{TelnetParams, TelnetProtocol};
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
use crate::session::protocol::{
    OutputSink, ProtocolKind, SessionCommand, SessionStatus, TerminalProtocol,
};
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

/// Open an unsaved local terminal.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalOpenRequest {
    pub options: LocalOptions,
    pub cols: u16,
    pub rows: u16,
}

/// A shell the local-terminal menu can offer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalShellInfo {
    pub shell: crate::session::options::LocalShell,
    pub label: &'static str,
    pub available: bool,
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
    /// Connect through this host first (ProxyJump); itself possibly chained.
    pub jump: Option<Box<Resolved>>,
}

impl Resolved {
    /// Connection parameters for this hop and every jump host before it.
    pub(crate) fn into_params(self, interaction: Interaction) -> SshConnectParams {
        SshConnectParams {
            host: self.host,
            port: self.port,
            username: self.username,
            auth: self.auth,
            jump: self
                .jump
                .map(|j| Box::new(j.into_params(interaction.clone()))),
            interaction,
        }
    }
}

/// Turn a spec into concrete parameters. For saved sessions this reads the
/// session, finds its credential (own or inherited from a folder) and
/// decrypts it with the vault - all inside Rust. Jump hosts are resolved the
/// same way, from their own saved sessions.
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
                jump: None,
            });
        }
        ConnectSpec::Saved {
            session_id,
            username,
            password,
        } => (session_id, username, password),
        ConnectSpec::Local { .. } => {
            return Err(AppError::Validation("Not an SSH session.".into()));
        }
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
        if session.protocol != ProtocolKind::Ssh {
            return Err(AppError::Validation("Not an SSH session.".into()));
        }
        let mut next_jump = session.jump_host_id.clone();
        let mut target = resolve_one(c, vault.as_deref(), session, typed_user, typed_password)?;

        // Walk the chain outwards: target ← hop1 ← hop2 … (hop N is dialled first).
        let mut hops: Vec<Resolved> = Vec::new();
        let mut seen = vec![session_id];
        while let Some(id) = next_jump {
            if seen.contains(&id) || hops.len() >= sessions::MAX_JUMP_HOPS {
                return Err(AppError::Validation(
                    "The jump hosts of this session form a loop or a chain that is too long."
                        .into(),
                ));
            }
            let hop = sessions::get(c, &id).map_err(|e| match e {
                AppError::NotFound(_) => {
                    AppError::Validation("The jump host of this session no longer exists.".into())
                }
                other => other,
            })?;
            if hop.protocol != ProtocolKind::Ssh {
                return Err(AppError::Validation(
                    "A jump host must be an SSH session.".into(),
                ));
            }
            next_jump = hop.jump_host_id.clone();
            seen.push(id);
            let name = hop.name.clone();
            let resolved =
                resolve_one(c, vault.as_deref(), hop, None, None).map_err(|e| match e {
                    // The login dialog asks for the target's credentials; a jump
                    // host must have its own saved ones.
                    AppError::CredentialsRequired(_) => AppError::Validation(format!(
                        "The jump host \"{name}\" needs a saved username and password or key. \
                     Edit it and link a credential (or use the SSH agent)."
                    )),
                    other => other,
                })?;
            hops.push(resolved);
        }
        // Nest: target.jump = hop1, hop1.jump = hop2, …
        let mut chain: Option<Box<Resolved>> = None;
        while let Some(mut hop) = hops.pop() {
            hop.jump = chain;
            chain = Some(Box::new(hop));
        }
        target.jump = chain;
        Ok(target)
    })
    .await
}

/// One saved session → host, user and decrypted authentication.
fn resolve_one(
    c: &rusqlite::Connection,
    vault: Option<&Vault>,
    session: crate::storage::models::SavedSession,
    typed_user: Option<String>,
    typed_password: Option<SecretString>,
) -> AppResult<Resolved> {
    let credential = match credentials::resolve_for_session(c, &session.id)? {
        Some(cid) if session.auth_method != "agent" => {
            let vault = vault.ok_or(AppError::VaultLocked)?;
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
        jump: None,
    })
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
    let sink: Arc<dyn OutputSink> = Arc::new(ChannelSink {
        output: channels.output.clone(),
        status: channels.status.clone(),
    });
    // Telnet, serial and local terminals: no login or host keys in Rust.
    if let Some((other, saved_id)) = other_protocol(&spec, deps.lib).await? {
        let handle = match other {
            Other::Telnet(p) => TelnetProtocol::open(p, size, Arc::clone(&sink)).await?,
            Other::Serial(o) => SerialProtocol::open(o, size, Arc::clone(&sink)).await?,
            Other::Local(o) => LocalProtocol::open(o, size, Arc::clone(&sink)).await?,
        };
        let id = deps.manager.register(handle, sink).await;
        tracing::info!(session_id = %id, "session registered");
        if let (Some(sid), Ok(db)) = (saved_id, deps.lib.db()) {
            let _ = db.run(move |c| sessions::mark_connected(c, &sid)).await;
        }
        return Ok(id);
    }
    let resolved = resolve(spec, deps.lib, deps.vault).await?;
    let interaction = Interaction {
        broker: Arc::clone(&deps.ctx.broker),
        prompts: Arc::new(ChannelPrompts(channels.prompts)),
        known_hosts: deps.lib.db().ok(),
        openssh_known_hosts: deps.ctx.openssh_known_hosts.clone(),
        agent_socket: None,
    };
    let saved_id = resolved.saved_session_id.clone();
    let params = resolved.into_params(interaction);
    let handle = SshProtocol::open(params, size, Arc::clone(&sink)).await?;
    let id = deps.manager.register(handle, sink).await;
    tracing::info!(session_id = %id, "session registered");

    if let (Some(sid), Ok(db)) = (saved_id, deps.lib.db()) {
        let _ = db.run(move |c| sessions::mark_connected(c, &sid)).await;
    }
    Ok(id)
}

/// A non-SSH terminal target.
enum Other {
    Telnet(TelnetParams),
    Serial(SerialOptions),
    Local(LocalOptions),
}

/// `Some` when the spec opens a Telnet, serial or local terminal (with the
/// saved session id, if any); `None` for SSH.
async fn other_protocol(
    spec: &ConnectSpec,
    lib: &LibraryDb,
) -> AppResult<Option<(Other, Option<String>)>> {
    let session_id = match spec {
        ConnectSpec::Direct { .. } => return Ok(None),
        ConnectSpec::Local { options } => {
            return Ok(Some((Other::Local(options.validated()?), None)));
        }
        ConnectSpec::Saved { session_id, .. } => session_id.clone(),
    };
    let db = lib.db()?;
    db.run(move |c| {
        let s = sessions::get(c, &session_id)?;
        let other =
            match s.protocol {
                ProtocolKind::Telnet => Other::Telnet(TelnetParams {
                    host: s.host,
                    port: s.port,
                }),
                ProtocolKind::Serial => Other::Serial(s.options.serial.ok_or_else(|| {
                    AppError::Validation("This session has no serial port set.".into())
                })?),
                ProtocolKind::Local => Other::Local(s.options.local.ok_or_else(|| {
                    AppError::Validation("This session has no shell set.".into())
                })?),
                ProtocolKind::Ssh | ProtocolKind::Ftp | ProtocolKind::Ftps => return Ok(None),
            };
        Ok(Some((other, Some(s.id))))
    })
    .await
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

/// Open a local terminal (PowerShell, CMD, WSL, or the login shell) that
/// isn't a saved session. Reconnect/duplicate work like for SSH tabs.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn local_open(
    request: LocalOpenRequest,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    on_prompt: Channel<SshPrompt>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
    lib: State<'_, LibraryDb>,
    vault: State<'_, VaultState>,
    ctx: State<'_, SshContext>,
) -> AppResult<SshSessionInfo> {
    let spec = ConnectSpec::Local {
        options: request.options.validated()?,
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

/// Shells for the local-terminal menu, the platform default first.
#[tauri::command]
pub fn local_shells() -> Vec<LocalShellInfo> {
    use crate::session::options::LocalShell;
    let candidates: &[LocalShell] = if cfg!(windows) {
        &[
            LocalShell::Powershell,
            LocalShell::Pwsh,
            LocalShell::Cmd,
            LocalShell::Wsl,
        ]
    } else {
        &[LocalShell::Default]
    };
    candidates
        .iter()
        .map(|&shell| LocalShellInfo {
            shell,
            label: shell.label(),
            available: crate::local::command(&LocalOptions {
                shell,
                wsl_distro: String::new(),
            })
            .is_ok(),
        })
        .collect()
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
