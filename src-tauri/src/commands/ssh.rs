//! SSH IPC commands. Validation happens here; logic lives in `ssh/` and `session/`.
//!
//! Output streaming: each session gets two `tauri::ipc::Channel`s supplied by
//! the frontend - one for raw terminal bytes (sent as an `ArrayBuffer`, not a
//! JSON number array) and one for status changes (JSON).

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::{Channel, Response};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::session::SessionManager;
use crate::session::protocol::{OutputSink, SessionCommand, SessionStatus, TerminalProtocol};
use crate::ssh::{SshConnectParams, SshConnections, SshProtocol, validate};

/// Largest single `ssh_write` payload (a big paste). 1 MiB.
const MAX_WRITE_BYTES: usize = 1024 * 1024;

/// Quick-connect request from the frontend.
///
/// `password` is deserialized straight into a `SecretString`: it is zeroized
/// on drop and prints as `[REDACTED]` if ever debug-formatted.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectRequest {
    pub host: String,
    pub port: u32,
    pub username: String,
    pub password: SecretString,
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

fn parse_connection_id(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw).map_err(|_| AppError::Validation("Invalid connection id.".into()))
}

/// Shared by connect / reconnect / duplicate: open the SSH session and
/// register it with the manager.
async fn open_session(
    params: SshConnectParams,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    manager: &Arc<SessionManager>,
) -> AppResult<Uuid> {
    let size = validate::terminal_size(cols, rows)?;
    let sink: Arc<dyn OutputSink> = Arc::new(ChannelSink {
        output: on_output,
        status: on_status,
    });
    let handle = SshProtocol::open(params, size, Arc::clone(&sink)).await?;
    let id = manager.register(handle, sink).await;
    tracing::info!(session_id = %id, "session registered");
    Ok(id)
}

/// Connect, authenticate and start a shell in a new tab.
#[tauri::command]
pub async fn ssh_connect(
    request: SshConnectRequest,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
) -> AppResult<SshSessionInfo> {
    let params = SshConnectParams {
        host: validate::host(&request.host)?,
        port: validate::port(request.port)?,
        username: validate::username(&request.username)?,
        password: {
            validate::password_len(request.password.expose_secret().len())?;
            request.password
        },
    };
    let remembered = params.duplicate();
    let session_id = open_session(
        params,
        request.cols,
        request.rows,
        on_output,
        on_status,
        &manager,
    )
    .await?;
    // Remember the parameters only after a successful login, so a typo'd
    // password is never kept.
    let connection_id = match connections.insert(remembered).await {
        Ok(id) => id,
        Err(e) => {
            let _ = manager.send(session_id, SessionCommand::Close).await;
            return Err(e);
        }
    };
    Ok(SshSessionInfo {
        session_id: session_id.to_string(),
        connection_id: connection_id.to_string(),
    })
}

/// Open a new session for an existing tab, reusing its remembered parameters.
#[tauri::command]
pub async fn ssh_reconnect(
    connection_id: String,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
) -> AppResult<SshSessionInfo> {
    let cid = parse_connection_id(&connection_id)?;
    let params = connections.get_copy(cid).await?;
    tracing::info!(connection_id = %cid, "reconnect requested");
    let session_id = open_session(params, cols, rows, on_output, on_status, &manager).await?;
    Ok(SshSessionInfo {
        session_id: session_id.to_string(),
        connection_id,
    })
}

/// Open the same connection in a new tab (gets its own connection id).
#[tauri::command]
pub async fn ssh_duplicate(
    connection_id: String,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
    on_status: Channel<SessionStatus>,
    manager: State<'_, Arc<SessionManager>>,
    connections: State<'_, SshConnections>,
) -> AppResult<SshSessionInfo> {
    let cid = parse_connection_id(&connection_id)?;
    let params = connections.get_copy(cid).await?;
    let remembered = params.duplicate();
    tracing::info!(source_connection_id = %cid, "duplicate requested");
    let session_id = open_session(params, cols, rows, on_output, on_status, &manager).await?;
    let new_id = match connections.insert(remembered).await {
        Ok(id) => id,
        Err(e) => {
            let _ = manager.send(session_id, SessionCommand::Close).await;
            return Err(e);
        }
    };
    Ok(SshSessionInfo {
        session_id: session_id.to_string(),
        connection_id: new_id.to_string(),
    })
}

/// The tab was closed: wipe its remembered parameters (password included).
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
