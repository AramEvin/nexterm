//! Interactive SSH shell sessions over `russh`.
//!
//! Flow: TCP connect (with timeout) -> key exchange -> **host-key verification**
//! (known hosts / user prompt) -> authentication (password, public key, agent,
//! keyboard-interactive) -> session channel -> PTY (`xterm-256color`) -> shell.
//! After that a dedicated tokio task shuttles bytes between the channel and
//! the [`OutputSink`] until either side closes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, Disconnect, MethodKind};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::mpsc;

use super::known_hosts::{self, HostKeyStatus};
use super::prompt::{
    HostKeyDecision, PROMPT_TIMEOUT, PromptBroker, PromptField, PromptResponse, PromptSink,
    SshPrompt,
};
use crate::error::{AppError, AppResult};
use crate::session::protocol::{
    OutputSink, ProtocolKind, SessionCommand, SessionHandle, SessionStatus, TerminalProtocol,
    TerminalSize,
};
use crate::storage::Database;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_MAX_MISSED: usize = 3;
const COMMAND_QUEUE: usize = 256;
const TERM_TYPE: &str = "xterm-256color";
/// Upper bound on keyboard-interactive rounds (a hostile server can't loop us).
const MAX_KBD_ROUNDS: usize = 10;

/// How to authenticate.
pub enum SshAuth {
    /// Password; also used to answer a "Password:" keyboard-interactive prompt.
    Password(SecretString),
    /// Private key from the vault (already decoded).
    PublicKey(Arc<PrivateKey>),
    /// Keys from a running SSH agent (Windows OpenSSH agent, Pageant, or SSH_AUTH_SOCK).
    Agent,
    /// Prompt the user for everything (2FA / OTP). A stored password, if any,
    /// answers a "Password:" prompt automatically.
    KeyboardInteractive { password: Option<SecretString> },
}

impl SshAuth {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Password(_) => "password",
            Self::PublicKey(_) => "publickey",
            Self::Agent => "agent",
            Self::KeyboardInteractive { .. } => "keyboard-interactive",
        }
    }

    /// Explicit deep copy (secrets are not `Clone` on purpose).
    pub fn duplicate(&self) -> Self {
        let copy = |s: &SecretString| SecretString::from(s.expose_secret().to_owned());
        match self {
            Self::Password(p) => Self::Password(copy(p)),
            Self::PublicKey(k) => Self::PublicKey(Arc::clone(k)),
            Self::Agent => Self::Agent,
            Self::KeyboardInteractive { password } => Self::KeyboardInteractive {
                password: password.as_ref().map(copy),
            },
        }
    }

    fn password(&self) -> Option<&SecretString> {
        match self {
            Self::Password(p) => Some(p),
            Self::KeyboardInteractive { password } => password.as_ref(),
            _ => None,
        }
    }
}

/// How this connection talks to the user and where host keys are trusted.
#[derive(Clone)]
pub struct Interaction {
    pub broker: Arc<PromptBroker>,
    pub prompts: Arc<dyn PromptSink>,
    /// NexTerm's known-hosts store; `None` if the database is unavailable
    /// (then every unknown key must be accepted for this connection only).
    pub known_hosts: Option<Database>,
    /// The user's OpenSSH known_hosts file, read-only.
    pub openssh_known_hosts: Option<PathBuf>,
    /// Unix only: agent socket to use instead of `$SSH_AUTH_SOCK` (tests).
    pub agent_socket: Option<PathBuf>,
}

/// Parameters for an interactive SSH session. Already validated.
pub struct SshConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    pub interaction: Interaction,
}

impl std::fmt::Debug for SshConnectParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnectParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &self.auth.method_name())
            .finish_non_exhaustive()
    }
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
            auth,
            interaction,
        } = params;
        tracing::info!(%host, port, %username, auth_method = auth.method_name(), "SSH connecting");

        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE_INTERVAL),
            keepalive_max: KEEPALIVE_MAX_MISSED,
            nodelay: true,
            ..Default::default()
        });
        let outcome = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            host: host.clone(),
            port,
            interaction: interaction.clone(),
            outcome: Arc::clone(&outcome),
        };

        // The connect timeout covers TCP + key exchange, not the time the user
        // spends reading a host-key prompt (that has its own timeout).
        let connected = tokio::time::timeout(
            CONNECT_TIMEOUT + PROMPT_TIMEOUT,
            client::connect(config, (host.as_str(), port), handler),
        )
        .await;
        let take_outcome = || {
            outcome
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        };
        let mut handle = match connected {
            Err(_) => {
                return Err(AppError::Connection(format!(
                    "Timed out connecting to {host}:{port}."
                )));
            }
            Ok(Err(e)) => {
                // A host-key decision explains the failure better than russh's error.
                return Err(take_outcome().unwrap_or_else(|| connect_error(&e, &host, port)));
            }
            Ok(Ok(h)) => h,
        };

        let method = auth.method_name();
        match authenticate(&mut handle, &host, &username, auth, &interaction).await {
            Ok(()) => {
                tracing::info!(%host, port, %username, auth_method = method, "SSH authenticated")
            }
            Err(e) => {
                tracing::warn!(%host, port, %username, auth_method = method, code = e.code(), "SSH authentication failed");
                let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
                return Err(e);
            }
        }

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

fn auth_error(e: &russh::Error) -> AppError {
    tracing::warn!(error = %e, "SSH error during authentication");
    AppError::Connection("The connection failed during authentication.".into())
}

/// Run the configured method, then keyboard-interactive if the server asks for
/// a second factor (partial success) or only allows keyboard-interactive.
async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    host: &str,
    user: &str,
    auth: SshAuth,
    interaction: &Interaction,
) -> AppResult<()> {
    let first = match &auth {
        SshAuth::Password(pw) => handle
            .authenticate_password(user, pw.expose_secret())
            .await
            .map_err(|e| auth_error(&e))?,
        SshAuth::PublicKey(key) => {
            let hash = if key.algorithm().is_rsa() {
                handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|e| auth_error(&e))?
                    .flatten()
            } else {
                None
            };
            handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::clone(key), hash))
                .await
                .map_err(|e| auth_error(&e))?
        }
        SshAuth::Agent => agent_auth(handle, user, interaction).await?,
        SshAuth::KeyboardInteractive { .. } => {
            return keyboard_interactive(handle, host, user, auth.password(), interaction).await;
        }
    };

    match first {
        AuthResult::Success => Ok(()),
        AuthResult::Failure {
            remaining_methods,
            partial_success,
        } => {
            let kbd = remaining_methods.contains(&MethodKind::KeyboardInteractive);
            let password_disabled = matches!(auth, SshAuth::Password(_))
                && !remaining_methods.contains(&MethodKind::Password);
            if kbd && (partial_success || password_disabled) {
                tracing::info!(partial_success, "continuing with keyboard-interactive");
                keyboard_interactive(handle, host, user, auth.password(), interaction).await
            } else {
                Err(AppError::AuthFailed)
            }
        }
    }
}

/// Answer keyboard-interactive rounds. A stored password answers one hidden
/// "Password" prompt automatically; everything else (OTP, etc.) asks the user.
async fn keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    host: &str,
    user: &str,
    password: Option<&SecretString>,
    interaction: &Interaction,
) -> AppResult<()> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await
        .map_err(|e| auth_error(&e))?;
    let mut password_used = false;

    for _ in 0..MAX_KBD_ROUNDS {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Err(AppError::AuthFailed),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let answers: Vec<String> = if prompts.is_empty() {
                    Vec::new()
                } else if let (Some(pw), false, [only]) =
                    (password, password_used, prompts.as_slice())
                    && !only.echo
                    && only.prompt.to_lowercase().contains("password")
                {
                    password_used = true;
                    vec![pw.expose_secret().to_owned()]
                } else {
                    let fields: Vec<PromptField> = prompts
                        .iter()
                        .map(|p| PromptField {
                            text: p.prompt.clone(),
                            echo: p.echo,
                        })
                        .collect();
                    let count = fields.len();
                    let reply = interaction
                        .broker
                        .ask(
                            interaction.prompts.as_ref(),
                            |prompt_id| SshPrompt::KeyboardInteractive {
                                prompt_id,
                                host: host.to_owned(),
                                name,
                                instructions,
                                fields,
                            },
                            PROMPT_TIMEOUT,
                        )
                        .await?;
                    match reply {
                        PromptResponse::Answers { answers } if answers.len() == count => answers
                            .iter()
                            .map(|a| a.expose_secret().to_owned())
                            .collect(),
                        PromptResponse::Answers { .. } => {
                            return Err(AppError::Validation("Wrong number of answers.".into()));
                        }
                        _ => return Err(AppError::Cancelled),
                    }
                };
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|e| auth_error(&e))?;
            }
        }
    }
    Err(AppError::AuthFailed)
}

/// Try every identity offered by the running SSH agent.
async fn agent_auth(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    interaction: &Interaction,
) -> AppResult<AuthResult> {
    #[cfg(windows)]
    {
        let _ = interaction; // the socket override is unix-only
        const OPENSSH_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";
        if let Ok(agent) =
            russh::keys::agent::client::AgentClient::connect_named_pipe(OPENSSH_PIPE).await
        {
            tracing::info!("using the Windows OpenSSH agent");
            return agent_try_identities(handle, user, agent).await;
        }
        if let Ok(agent) = russh::keys::agent::client::AgentClient::connect_pageant().await {
            tracing::info!("using Pageant");
            return agent_try_identities(handle, user, agent).await;
        }
        Err(AppError::Connection(
            "No SSH agent found. Start the \"OpenSSH Authentication Agent\" service or Pageant \
             and add your key."
                .into(),
        ))
    }
    #[cfg(unix)]
    {
        use russh::keys::agent::client::AgentClient;
        let agent = match &interaction.agent_socket {
            Some(path) => AgentClient::connect_uds(path).await,
            None => AgentClient::connect_env().await,
        }
        .map_err(|_| AppError::Connection("No SSH agent found (SSH_AUTH_SOCK).".into()))?;
        agent_try_identities(handle, user, agent).await
    }
}

async fn agent_try_identities<S>(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    mut agent: russh::keys::agent::client::AgentClient<S>,
) -> AppResult<AuthResult>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| AppError::Connection(format!("The SSH agent did not answer: {e}")))?;
    if identities.is_empty() {
        return Err(AppError::Connection(
            "The SSH agent has no keys loaded. Add one with ssh-add.".into(),
        ));
    }
    let rsa_hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(|e| auth_error(&e))?
        .flatten();
    let mut last = AuthResult::Failure {
        remaining_methods: russh::MethodSet::empty(),
        partial_success: false,
    };
    for identity in identities {
        let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = identity else {
            continue; // certificates in the agent are not supported yet
        };
        let hash = key.algorithm().is_rsa().then_some(rsa_hash).flatten();
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
        match handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            Ok(AuthResult::Success) => {
                tracing::info!(%fingerprint, "authenticated with agent key");
                return Ok(AuthResult::Success);
            }
            Ok(failure) => last = failure,
            Err(e) => tracing::debug!(error = %e, "agent signing failed"),
        }
    }
    Ok(last)
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

/// Host-key policy (SECURITY.md §6): trusted keys pass silently, changed keys
/// are **blocked**, unknown keys are shown to the user with their fingerprint.
struct ClientHandler {
    host: String,
    port: u16,
    interaction: Interaction,
    /// Why the connection was refused, for a precise error message.
    outcome: Arc<Mutex<Option<AppError>>>,
}

impl ClientHandler {
    fn refuse(&self, err: AppError) -> Result<bool, russh::Error> {
        *self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(err);
        Ok(false)
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = known_hosts::server_public_key(server_key);
        let (key_type, fingerprint) = known_hosts::describe(&key);
        let (host, port) = (self.host.clone(), self.port);

        let status = match &self.interaction.known_hosts {
            Some(db) => db.with(|c| {
                known_hosts::check(
                    c,
                    &host,
                    port,
                    &key,
                    self.interaction.openssh_known_hosts.as_deref(),
                )
            }),
            None => Ok(HostKeyStatus::Unknown {
                other_known_types: vec![],
            }),
        };
        let status = match status {
            Ok(s) => s,
            Err(e) => {
                // Fail closed: if we can't check, we don't connect.
                tracing::error!(%host, port, error = %e, "host-key lookup failed");
                return self.refuse(AppError::Internal("host-key lookup failed".into()));
            }
        };

        match status {
            HostKeyStatus::Trusted { source } => {
                tracing::info!(%host, port, %key_type, %fingerprint, source, "host key verified");
                Ok(true)
            }
            HostKeyStatus::Changed { stored } => {
                tracing::error!(%host, port, %key_type, %fingerprint, %stored, "HOST KEY CHANGED - connection blocked");
                self.refuse(AppError::HostKeyChanged(format!(
                    "WARNING: the host key of {host}:{port} has CHANGED. Someone may be \
                     intercepting the connection (man-in-the-middle), or the server was \
                     reinstalled. NexTerm blocked the connection.\n\nExpected: {stored}\n\
                     Presented: {key_type} {fingerprint}\n\nVerify the new fingerprint with the \
                     server's administrator before removing the stored key."
                )))
            }
            HostKeyStatus::Unknown { other_known_types } => {
                let reply = self
                    .interaction
                    .broker
                    .ask(
                        self.interaction.prompts.as_ref(),
                        |prompt_id| SshPrompt::HostKey {
                            prompt_id,
                            host: host.clone(),
                            port,
                            key_type: key_type.clone(),
                            fingerprint: fingerprint.clone(),
                            other_known_types,
                        },
                        PROMPT_TIMEOUT,
                    )
                    .await;
                let decision = match reply {
                    Ok(PromptResponse::HostKey { decision }) => decision,
                    _ => HostKeyDecision::Reject,
                };
                tracing::info!(%host, port, %key_type, %fingerprint, ?decision, "host-key decision");
                match decision {
                    HostKeyDecision::AcceptOnce => Ok(true),
                    HostKeyDecision::AcceptAndSave => {
                        if let Some(db) = &self.interaction.known_hosts
                            && let Err(e) = db.with(|c| known_hosts::save(c, &host, port, &key))
                        {
                            tracing::warn!(error = %e, "could not save the host key");
                        }
                        Ok(true)
                    }
                    HostKeyDecision::Reject => self.refuse(AppError::HostKeyRejected),
                }
            }
        }
    }
}
