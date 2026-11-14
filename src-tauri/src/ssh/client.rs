//! Interactive SSH shell sessions over `russh`.
//!
//! Flow: TCP connect (with timeout) -> key exchange -> host-key check ->
//! password auth -> session channel -> PTY (`xterm-256color`) -> shell.
//! After that a dedicated tokio task shuttles bytes between the channel and
//! the [`OutputSink`] until either side closes.

use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, Disconnect};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::session::protocol::{
    OutputSink, ProtocolKind, SessionCommand, SessionHandle, SessionStatus, TerminalProtocol,
    TerminalSize,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_MAX_MISSED: usize = 3;
const COMMAND_QUEUE: usize = 256;
const TERM_TYPE: &str = "xterm-256color";

/// Parameters for an interactive SSH session. Already validated.
/// `Debug` is safe: `SecretString` prints as `[REDACTED]`.
#[derive(Debug)]
pub struct SshConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: SecretString,
}

/// The SSH implementation of [`TerminalProtocol`].
pub struct SshProtocol;

impl TerminalProtocol for SshProtocol {
    type Params = SshConnectParams;

    async fn open(
        params: SshConnectParams,
        size: TerminalSize,
        sink: Arc<dyn OutputSink>,
    ) -> AppResult<SessionHandle> {
        let SshConnectParams {
            host,
            port,
            username,
            password,
        } = params;
        tracing::info!(%host, port, %username, "SSH connecting");

        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE_INTERVAL),
            keepalive_max: KEEPALIVE_MAX_MISSED,
            nodelay: true,
            ..Default::default()
        });
        let handler = ClientHandler {
            host: host.clone(),
            port,
        };

        let mut handle = tokio::time::timeout(
            CONNECT_TIMEOUT,
            client::connect(config, (host.as_str(), port), handler),
        )
        .await
        .map_err(|_| {
            AppError::Connection(format!(
                "Timed out connecting to {host}:{port} after {}s.",
                CONNECT_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| connect_error(&e, &host, port))?;

        let auth = handle
            .authenticate_password(username.as_str(), password.expose_secret())
            .await
            .map_err(|e| connect_error(&e, &host, port))?;
        drop(password); // zeroized on drop
        if !auth.success() {
            tracing::warn!(%host, port, %username, auth_method = "password", "SSH authentication failed");
            return Err(AppError::AuthFailed);
        }
        tracing::info!(%host, port, %username, auth_method = "password", "SSH authenticated");

        let channel = open_shell(&handle, size)
            .await
            .map_err(|e| connect_error(&e, &host, port))?;

        let (commands, rx) = mpsc::channel(COMMAND_QUEUE);
        let task = tokio::spawn(async move {
            let reason = run_session(&handle, channel, rx, sink.as_ref()).await;
            let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
            tracing::info!(%host, port, %reason, "SSH session closed");
            sink.status(SessionStatus::Closed { reason });
        });

        Ok(SessionHandle {
            kind: ProtocolKind::Ssh,
            commands,
            task,
        })
    }
}

/// Open a session channel with a PTY and start the login shell.
async fn open_shell(
    handle: &Handle<ClientHandler>,
    size: TerminalSize,
) -> Result<Channel<client::Msg>, russh::Error> {
    let channel = handle.channel_open_session().await?;
    channel
        .request_pty(
            false,
            TERM_TYPE,
            u32::from(size.cols),
            u32::from(size.rows),
            0,
            0,
            &[],
        )
        .await?;
    channel.request_shell(false).await?;
    Ok(channel)
}

/// Pump data until the shell exits, the connection drops or the user closes.
/// Returns the user-facing reason the session ended.
async fn run_session(
    handle: &Handle<ClientHandler>,
    mut channel: Channel<client::Msg>,
    mut commands: mpsc::Receiver<SessionCommand>,
    sink: &dyn OutputSink,
) -> String {
    let mut exit_status: Option<u32> = None;

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(SessionCommand::Write(bytes)) => {
                    if channel.data_bytes(bytes).await.is_err() {
                        return "Connection lost while sending input.".into();
                    }
                }
                Some(SessionCommand::Resize(size)) => {
                    if let Err(e) = channel
                        .window_change(u32::from(size.cols), u32::from(size.rows), 0, 0)
                        .await
                    {
                        tracing::debug!(error = %e, "window_change failed");
                    }
                }
                Some(SessionCommand::Close) | None => {
                    let _ = channel.eof().await;
                    let _ = channel.close().await;
                    return "Disconnected.".into();
                }
            },
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    if !sink.data(data.to_vec()) {
                        // UI is gone (window closed); stop quietly.
                        return "Terminal closed.".into();
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status: code }) => exit_status = Some(code),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    return format!("Remote shell was terminated by signal {signal_name:?}.");
                }
                Some(ChannelMsg::Close) | None => {
                    return match exit_status {
                        Some(code) => format!("Session ended (exit status {code})."),
                        None if handle.is_closed() => "Connection closed by the remote host.".into(),
                        None => "Session ended.".into(),
                    };
                }
                Some(_) => {}
            },
        }
    }
}

/// Map a russh error to a user-safe `AppError`.
fn connect_error(err: &russh::Error, host: &str, port: u16) -> AppError {
    tracing::warn!(%host, port, error = %err, "SSH connection error");
    let msg = match err {
        russh::Error::IO(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                format!("Connection refused by {host}:{port}. Is SSH running on that port?")
            }
            std::io::ErrorKind::TimedOut => format!("Timed out connecting to {host}:{port}."),
            _ => format!("Could not connect to {host}:{port}: {io}"),
        },
        russh::Error::Disconnect | russh::Error::HUP => {
            "The server closed the connection.".to_owned()
        }
        russh::Error::NoCommonAlgo { .. } => {
            "No common encryption algorithms with the server (it may be very old).".to_owned()
        }
        russh::Error::ConnectionTimeout | russh::Error::Elapsed(_) => {
            format!("Timed out connecting to {host}:{port}.")
        }
        other => format!("SSH error: {other}"),
    };
    AppError::Connection(msg)
}

/// Host-key policy. **Phase 1 placeholder**: accepts every key but logs its
/// fingerprint. Real verification (TOFU dialog, known_hosts, blocking changed
/// keys) is implemented in Phase 4.
struct ClientHandler {
    host: String,
    port: u16,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let (key_type, fingerprint) = describe_host_key(server_key);
        tracing::warn!(
            host = %self.host,
            port = self.port,
            %key_type,
            %fingerprint,
            "host key accepted WITHOUT verification (temporary until Phase 4)"
        );
        Ok(true)
    }
}

/// Key algorithm and SHA256 fingerprint, as shown by `ssh-keygen -lf`.
fn describe_host_key(key: &PublicKeyOrCertificate) -> (String, String) {
    match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => (
            key.algorithm().to_string(),
            key.fingerprint(HashAlg::Sha256).to_string(),
        ),
        PublicKeyOrCertificate::Certificate(cert) => (
            cert.algorithm().to_string(),
            cert.public_key().fingerprint(HashAlg::Sha256).to_string(),
        ),
    }
}
