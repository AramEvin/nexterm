//! Integration tests against a real SSH server.
//!
//! Ignored by default. To run them, start a disposable server, e.g.:
//!
//! ```text
//! docker run -d --name nexterm-sshd -p 2222:2222 \
//!   -e USER_NAME=ntest -e USER_PASSWORD=Test-Pass-123 -e PASSWORD_ACCESS=true \
//!   lscr.io/linuxserver/openssh-server:latest
//! ```
//!
//! (add `-e PUBLIC_KEY="$(cat tests/fixtures/test_ed25519.pub)"` for the
//! public-key test), then:
//!
//! ```text
//! NEXTERM_TEST_SSH_HOST=127.0.0.1 NEXTERM_TEST_SSH_PORT=2222 \
//! NEXTERM_TEST_SSH_USER=ntest NEXTERM_TEST_SSH_PASSWORD=Test-Pass-123 \
//!   cargo test --test ssh_integration -- --ignored --test-threads=1
//! ```
//!
//! Optional: `NEXTERM_TEST_SSH_KBD_PORT` = a second sshd with password auth
//! disabled and keyboard-interactive (PAM) enabled, for the 2FA-style tests.

// Test code: panicking on unexpected state is the point.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexterm_lib::error::AppError;
use nexterm_lib::session::monitor::{MonitorSink, ResourceEvent};
use nexterm_lib::session::protocol::{
    OutputSink, SessionCommand, SessionStatus, TerminalProtocol, TerminalSize,
};
use nexterm_lib::ssh::prompt::{HostKeyDecision, PromptResponse, PromptSink, SshPrompt};
use nexterm_lib::ssh::{Interaction, PromptBroker, SshAuth, SshConnectParams, SshProtocol};
use nexterm_lib::storage::Database;
use secrecy::SecretString;

/// Answers prompts automatically and records them.
struct AutoPrompts {
    broker: Arc<PromptBroker>,
    decision: HostKeyDecision,
    answers: Vec<String>,
    seen: Mutex<Vec<SshPrompt>>,
}

impl PromptSink for AutoPrompts {
    fn send(&self, prompt: SshPrompt) -> bool {
        // The broker registers the prompt before calling `send`, so answering
        // synchronously here is fine.
        let response = match &prompt {
            SshPrompt::HostKey { prompt_id, .. } => (
                prompt_id.clone(),
                PromptResponse::HostKey {
                    decision: self.decision,
                },
            ),
            SshPrompt::KeyboardInteractive { prompt_id, .. } => (
                prompt_id.clone(),
                PromptResponse::Answers {
                    answers: self
                        .answers
                        .iter()
                        .map(|a| SecretString::from(a.clone()))
                        .collect(),
                },
            ),
        };
        self.seen.lock().unwrap().push(prompt);
        self.broker.respond(&response.0, response.1).unwrap();
        true
    }
}

struct Ctx {
    prompts: Arc<AutoPrompts>,
    interaction: Interaction,
}

fn ctx(db: Option<Database>, decision: HostKeyDecision, answers: &[&str]) -> Ctx {
    let broker = Arc::new(PromptBroker::new());
    let prompts = Arc::new(AutoPrompts {
        broker: broker.clone(),
        decision,
        answers: answers.iter().map(|s| (*s).to_string()).collect(),
        seen: Mutex::new(Vec::new()),
    });
    let interaction = Interaction {
        broker,
        prompts: prompts.clone(),
        known_hosts: db,
        openssh_known_hosts: None,
        agent_socket: None,
    };
    Ctx {
        prompts,
        interaction,
    }
}

fn host_key_prompts(c: &Ctx) -> usize {
    c.prompts
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|p| matches!(p, SshPrompt::HostKey { .. }))
        .count()
}

fn params_with(auth: SshAuth, interaction: Interaction) -> SshConnectParams {
    params_on(
        env("NEXTERM_TEST_SSH_PORT").parse().unwrap(),
        auth,
        interaction,
    )
}

fn params_on(port: u16, auth: SshAuth, interaction: Interaction) -> SshConnectParams {
    SshConnectParams {
        host: env("NEXTERM_TEST_SSH_HOST"),
        port,
        username: env("NEXTERM_TEST_SSH_USER"),
        auth,
        interaction,
        jump: None,
    }
}

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

const TERM: TerminalSize = TerminalSize { cols: 80, rows: 24 };

/// Connect, check the shell answers, close.
async fn connect_and_echo(params: SshConnectParams) -> Result<(), AppError> {
    let sink = Arc::new(CollectSink::default());
    let handle = SshProtocol::open(params, TERM, sink.clone()).await?;
    handle
        .commands
        .send(SessionCommand::Write(b"echo NX-$((40+2))\n".to_vec()))
        .await
        .unwrap();
    sink.wait_for("NX-42").await;
    handle.commands.send(SessionCommand::Close).await.unwrap();
    let _ = handle.task.await;
    Ok(())
}

#[derive(Default)]
struct CollectSink {
    out: Mutex<Vec<u8>>,
    status: Mutex<Vec<SessionStatus>>,
}

impl OutputSink for CollectSink {
    fn data(&self, bytes: Vec<u8>) -> bool {
        self.out.lock().unwrap().extend_from_slice(&bytes);
        true
    }
    fn status(&self, status: SessionStatus) {
        self.status.lock().unwrap().push(status);
    }
}

impl CollectSink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }
    async fn wait_for(&self, needle: &str) -> String {
        for _ in 0..200 {
            let t = self.text();
            if t.contains(needle) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "timed out waiting for {needle:?}; output so far:\n{}",
            self.text()
        );
    }
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set for integration tests"))
}

/// Password auth; unknown host keys are accepted for this connection only.
fn params(password: &str) -> SshConnectParams {
    params_with(
        SshAuth::Password(pw(password)),
        ctx(None, HostKeyDecision::AcceptOnce, &[]).interaction,
    )
}

#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn interactive_shell_pty_resize_utf8_and_exit() {
    let sink = Arc::new(CollectSink::default());
    let handle = SshProtocol::open(
        params(&env("NEXTERM_TEST_SSH_PASSWORD")),
        TerminalSize {
            cols: 100,
            rows: 30,
        },
        sink.clone(),
    )
    .await
    .expect("connect");

    let send = |s: &str| {
        let tx = handle.commands.clone();
        let bytes = s.as_bytes().to_vec();
        async move { tx.send(SessionCommand::Write(bytes)).await.unwrap() }
    };

    // PTY has the requested size and TERM.
    send("stty size; echo TERM=$TERM; echo M1-$((40+2))\n").await;
    let out = sink.wait_for("M1-42").await;
    assert!(out.contains("30 100"), "stty size wrong:\n{out}");
    assert!(out.contains("TERM=xterm-256color"), "TERM wrong:\n{out}");

    // Resize reaches the remote PTY.
    handle
        .commands
        .send(SessionCommand::Resize(TerminalSize {
            cols: 132,
            rows: 43,
        }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    send("stty size; echo M2-$((40+2))\n").await;
    let out = sink.wait_for("M2-42").await;
    assert!(out.contains("43 132"), "resize not applied:\n{out}");

    // UTF-8 survives the round trip byte-exact.
    send("printf '\\xe2\\x9c\\x93 \\xd5\\x80\\xd5\\xa1\\xd5\\xb5 \\xf0\\x9f\\x9a\\x80\\n'; echo M3-$((40+2))\n").await;
    let out = sink.wait_for("M3-42").await;
    assert!(out.contains("✓ Հայ 🚀"), "UTF-8 mangled:\n{out}");

    // `exit` ends the session with a clear status.
    send("exit 0\n").await;
    handle.task.await.unwrap();
    let status = sink.status.lock().unwrap().clone();
    assert_eq!(
        status,
        vec![SessionStatus::Closed {
            reason: "Session ended (exit status 0).".into()
        }]
    );
}

#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn user_close_reports_disconnected() {
    let sink = Arc::new(CollectSink::default());
    let handle = SshProtocol::open(
        params(&env("NEXTERM_TEST_SSH_PASSWORD")),
        TerminalSize { cols: 80, rows: 24 },
        sink.clone(),
    )
    .await
    .expect("connect");
    handle.commands.send(SessionCommand::Close).await.unwrap();
    handle.task.await.unwrap();
    assert_eq!(
        sink.status.lock().unwrap().first(),
        Some(&SessionStatus::Closed {
            reason: "Disconnected.".into()
        })
    );
}

#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn wrong_password_is_auth_failed() {
    let result = SshProtocol::open(
        params("definitely-wrong-password"),
        TerminalSize { cols: 80, rows: 24 },
        Arc::new(CollectSink::default()),
    )
    .await;
    assert!(matches!(result, Err(AppError::AuthFailed)));
}

#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn closed_port_is_connection_refused() {
    let mut p = params("x");
    p.port = 1; // nothing listens on tcpmux
    let Err(err) = SshProtocol::open(
        p,
        TerminalSize { cols: 80, rows: 24 },
        Arc::new(CollectSink::default()),
    )
    .await
    else {
        panic!("expected an error");
    };
    assert_eq!(err.code(), "CONNECTION");
    assert!(
        err.user_message().contains("refused"),
        "{}",
        err.user_message()
    );
}

/// Phase 2 acceptance: 20 concurrent sessions through the `SessionManager`;
/// closing one leaves the others working; repeated open/close leaves nothing
/// behind in the registry.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a real SSH server (see module docs)"]
async fn twenty_sessions_are_independent_and_cleaned_up() {
    use nexterm_lib::session::SessionManager;

    const N: usize = 20;
    let manager = SessionManager::new();
    let size = TerminalSize { cols: 80, rows: 24 };
    let password = env("NEXTERM_TEST_SSH_PASSWORD");

    // Open in batches of 5: OpenSSH's default `MaxStartups 10:30:100` starts
    // dropping connections when more than 10 logins are in flight at once.
    let mut opened = Vec::new();
    for _ in 0..N / 5 {
        let batch = (0..5).map(|_| {
            let sink = Arc::new(CollectSink::default());
            let params = params(&password);
            async move {
                let handle = SshProtocol::open(params, size, sink.clone())
                    .await
                    .expect("connect");
                (handle, sink)
            }
        });
        opened.extend(futures_join_all(batch).await);
    }
    let mut sessions = Vec::new();
    for (handle, sink) in opened {
        let sink_dyn: Arc<dyn OutputSink> = sink.clone();
        let id = manager.register(handle, sink_dyn).await;
        sessions.push((id, sink));
    }
    assert_eq!(manager.len().await, N);

    // Close one in the middle.
    let (closed_id, closed_sink) = &sessions[N / 2];
    manager
        .send(*closed_id, SessionCommand::Close)
        .await
        .unwrap();
    for _ in 0..100 {
        if manager.len().await == N - 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(manager.len().await, N - 1);
    assert!(!closed_sink.status.lock().unwrap().is_empty());

    // Every other session still answers.
    for (i, (id, sink)) in sessions.iter().enumerate() {
        if i == N / 2 {
            continue;
        }
        let cmd = format!("echo ALIVE-$((1000+{i}))\n");
        manager
            .send(*id, SessionCommand::Write(cmd.into_bytes()))
            .await
            .unwrap();
        sink.wait_for(&format!("ALIVE-{}", 1000 + i)).await;
    }

    // Close the rest; the registry must end up empty.
    for (i, (id, _)) in sessions.iter().enumerate() {
        if i != N / 2 {
            manager.send(*id, SessionCommand::Close).await.unwrap();
        }
    }
    for _ in 0..200 {
        if manager.is_empty().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(manager.is_empty().await, "sessions leaked in the registry");

    // Repeated open/close: still nothing left behind.
    for _ in 0..10 {
        let sink = Arc::new(CollectSink::default());
        let handle = SshProtocol::open(params(&password), size, sink.clone())
            .await
            .expect("connect");
        let id = manager.register(handle, sink).await;
        manager.send(id, SessionCommand::Close).await.unwrap();
    }
    for _ in 0..200 {
        if manager.is_empty().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        manager.is_empty().await,
        "sessions leaked after open/close cycles"
    );
}

/// Minimal join_all so the test needs no extra dependency.
async fn futures_join_all<F: std::future::Future + Send + 'static>(
    futs: impl Iterator<Item = F>,
) -> Vec<F::Output>
where
    F::Output: Send + 'static,
{
    let handles: Vec<_> = futs.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.unwrap());
    }
    out
}

// ---- Phase 4: host keys, key / agent / keyboard-interactive auth ------------

/// Phase 4 acceptance: unknown key -> prompt -> saved; next time trusted
/// silently; a changed key BLOCKS the connection; rejecting refuses.
#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn host_key_tofu_trust_and_changed_key_is_blocked() {
    let db = Database::open_in_memory().unwrap();
    let password = env("NEXTERM_TEST_SSH_PASSWORD");

    // 1. Unknown: the user is asked once and saves the key.
    let c1 = ctx(Some(db.clone()), HostKeyDecision::AcceptAndSave, &[]);
    connect_and_echo(params_with(
        SshAuth::Password(pw(&password)),
        c1.interaction.clone(),
    ))
    .await
    .expect("first connect");
    assert_eq!(host_key_prompts(&c1), 1);

    // 2. Known: no prompt.
    let c2 = ctx(Some(db.clone()), HostKeyDecision::Reject, &[]);
    connect_and_echo(params_with(
        SshAuth::Password(pw(&password)),
        c2.interaction.clone(),
    ))
    .await
    .expect("trusted connect");
    assert_eq!(host_key_prompts(&c2), 0, "a trusted key must not prompt");

    // 3. Changed: swap the stored key for a different one of the same type.
    let other = "AAAAC3NzaC1lZDI1NTE5AAAAIHo0b/OsK3MI4yXM+vWWWf3Kxr8hAe8wDdrfVXCmUWYG";
    db.with(|c| {
        let n = c.execute(
            "UPDATE known_hosts SET public_key = key_type || ' ' || ?1, fingerprint = 'SHA256:old'",
            [other],
        )?;
        assert_eq!(n, 1);
        Ok(())
    })
    .unwrap();
    let c3 = ctx(Some(db.clone()), HostKeyDecision::AcceptAndSave, &[]);
    let err = connect_and_echo(params_with(
        SshAuth::Password(pw(&password)),
        c3.interaction.clone(),
    ))
    .await
    .expect_err("a changed host key must block the connection");
    assert_eq!(err.code(), "HOST_KEY_CHANGED", "{err:?}");
    assert!(err.user_message().contains("SHA256:old"));
    assert_eq!(
        host_key_prompts(&c3),
        0,
        "no 'accept anyway' prompt for a changed key"
    );

    // 4. Rejecting an unknown key refuses the connection.
    db.with(|c| Ok(c.execute("DELETE FROM known_hosts", [])?))
        .unwrap();
    let c4 = ctx(Some(db), HostKeyDecision::Reject, &[]);
    let err = connect_and_echo(params_with(
        SshAuth::Password(pw(&password)),
        c4.interaction.clone(),
    ))
    .await
    .expect_err("rejected key");
    assert_eq!(err.code(), "HOST_KEY_REJECTED");
}

const TEST_KEY: &str = include_str!("fixtures/test_ed25519");
const TEST_KEY_ENC: &str = include_str!("fixtures/test_ed25519_enc");

/// Needs the fixture public key in the server's authorized_keys
/// (CI: `PUBLIC_KEY` env of the openssh-server container).
#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn public_key_auth_plain_and_passphrase() {
    for (key, passphrase) in [(TEST_KEY, None), (TEST_KEY_ENC, Some("test-passphrase"))] {
        let parsed = russh::keys::decode_secret_key(key, passphrase).unwrap();
        let c = ctx(None, HostKeyDecision::AcceptOnce, &[]);
        connect_and_echo(params_with(
            SshAuth::PublicKey(Arc::new(parsed)),
            c.interaction,
        ))
        .await
        .expect("public-key login");
    }
}

#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn wrong_key_is_rejected() {
    let other =
        russh::keys::decode_secret_key(include_str!("fixtures/test_ed25519_other"), None).unwrap();
    let c = ctx(None, HostKeyDecision::AcceptOnce, &[]);
    let err = connect_and_echo(params_with(
        SshAuth::PublicKey(Arc::new(other)),
        c.interaction,
    ))
    .await
    .expect_err("unknown key must fail");
    assert_eq!(err.code(), "AUTH_FAILED");
}

/// Agent login with a throwaway `ssh-agent` holding the fixture key.
#[cfg(unix)]
#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn agent_auth() {
    let dir = std::env::temp_dir().join(format!("nexterm-agent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("agent.sock");
    let _ = std::fs::remove_file(&sock);
    let Ok(mut agent) = std::process::Command::new("ssh-agent")
        .args(["-D", "-a"])
        .arg(&sock)
        .stdout(std::process::Stdio::null())
        .spawn()
    else {
        eprintln!("ssh-agent not installed; skipping");
        return;
    };
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let key = russh::keys::decode_secret_key(TEST_KEY, None).unwrap();
    let mut client = russh::keys::agent::client::AgentClient::connect_uds(&sock)
        .await
        .unwrap();
    client.add_identity(&key, &[]).await.unwrap();

    let mut c = ctx(None, HostKeyDecision::AcceptOnce, &[]);
    c.interaction.agent_socket = Some(sock.clone());
    let result = connect_and_echo(params_with(SshAuth::Agent, c.interaction)).await;
    let _ = agent.kill();
    std::fs::remove_dir_all(&dir).ok();
    result.expect("agent login");
}

/// Keyboard-interactive (PAM): the stored password answers "Password:"
/// automatically; with no stored password the user is asked.
#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn keyboard_interactive_autofill_and_prompt() {
    let Ok(port) = std::env::var("NEXTERM_TEST_SSH_KBD_PORT") else {
        eprintln!("NEXTERM_TEST_SSH_KBD_PORT not set; skipping");
        return;
    };
    let port: u16 = port.parse().unwrap();
    let password = env("NEXTERM_TEST_SSH_PASSWORD");

    // Password method on a server that only offers keyboard-interactive:
    // falls back and auto-fills the password; no prompt shown.
    let c = ctx(None, HostKeyDecision::AcceptOnce, &[]);
    connect_and_echo(params_on(
        port,
        SshAuth::Password(pw(&password)),
        c.interaction.clone(),
    ))
    .await
    .expect("kbd-interactive with auto-filled password");
    assert!(
        !c.prompts
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|p| matches!(p, SshPrompt::KeyboardInteractive { .. }))
    );

    // No stored password: the prompt goes to the user (hidden field).
    let c = ctx(None, HostKeyDecision::AcceptOnce, &[&password]);
    connect_and_echo(params_on(
        port,
        SshAuth::KeyboardInteractive { password: None },
        c.interaction.clone(),
    ))
    .await
    .expect("kbd-interactive with user answer");
    let seen = c.prompts.seen.lock().unwrap();
    let kbd = seen
        .iter()
        .find_map(|p| match p {
            SshPrompt::KeyboardInteractive { fields, .. } => Some(fields.clone()),
            _ => None,
        })
        .expect("user was prompted");
    assert!(!kbd[0].echo, "password prompt must hide input");
}

/// Captures log output in memory.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Phase 4 acceptance: logs record auth events but never secrets.
#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn logs_contain_no_secrets() {
    let buffer = LogBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // Current-thread runtime: spawned tasks log through this subscriber too.
    let _guard = tracing::subscriber::set_default(subscriber);

    let password = env("NEXTERM_TEST_SSH_PASSWORD");
    let db = Database::open_in_memory().unwrap();
    let c = ctx(Some(db), HostKeyDecision::AcceptAndSave, &[]);
    connect_and_echo(params_with(
        SshAuth::Password(pw(&password)),
        c.interaction.clone(),
    ))
    .await
    .unwrap();
    let key = russh::keys::decode_secret_key(TEST_KEY_ENC, Some("test-passphrase")).unwrap();
    connect_and_echo(params_with(
        SshAuth::PublicKey(Arc::new(key)),
        c.interaction.clone(),
    ))
    .await
    .unwrap();
    let _ = connect_and_echo(params_with(
        SshAuth::Password(pw("Wrong-Pass-Secret-981")),
        c.interaction.clone(),
    ))
    .await;
    if let Ok(port) = std::env::var("NEXTERM_TEST_SSH_KBD_PORT") {
        let kc = ctx(None, HostKeyDecision::AcceptOnce, &[]);
        connect_and_echo(params_on(
            port.parse().unwrap(),
            SshAuth::Password(pw(&password)),
            kc.interaction,
        ))
        .await
        .unwrap();
    }

    let logs = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    for secret in [
        password.as_str(),
        "test-passphrase",
        "Wrong-Pass-Secret-981",
    ] {
        assert!(!logs.contains(secret), "secret {secret:?} found in logs");
    }
    for line in TEST_KEY_ENC.lines().skip(1).take(3) {
        assert!(
            !logs.contains(line.trim()),
            "private key material found in logs"
        );
    }
    // ...while the audit events are there.
    assert!(logs.contains("SSH authenticated"));
    assert!(logs.contains("auth_method=\"password\"") || logs.contains("auth_method=password"));
    assert!(logs.contains("publickey"));
    assert!(logs.contains("host-key decision") || logs.contains("host key verified"));
}

#[derive(Default)]
struct CollectMonitor(Mutex<Vec<ResourceEvent>>);

impl MonitorSink for CollectMonitor {
    fn event(&self, event: ResourceEvent) -> bool {
        self.0.lock().unwrap().push(event);
        true
    }
}

#[tokio::test]
#[ignore = "needs a real SSH server (see module docs)"]
async fn resource_monitor_reports_linux_readings_without_touching_the_shell() {
    let sink = Arc::new(CollectSink::default());
    let handle = SshProtocol::open(
        params(&env("NEXTERM_TEST_SSH_PASSWORD")),
        TERM,
        sink.clone(),
    )
    .await
    .expect("connect");
    let monitor = Arc::new(CollectMonitor::default());
    handle
        .commands
        .send(SessionCommand::Monitor(Some(monitor.clone())))
        .await
        .unwrap();

    // Two readings (3 s apart) give a CPU percentage.
    let mut samples = Vec::new();
    for _ in 0..200 {
        samples = monitor
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                ResourceEvent::Sample(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        if samples.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        samples.len() >= 2,
        "events: {:?}",
        monitor.0.lock().unwrap()
    );
    let s = &samples[1];
    assert!(s.mem_total_kib > 0 && s.mem_used_kib <= s.mem_total_kib);
    assert!(s.cores >= 1);
    assert!(s.cpu_percent.is_some_and(|c| (0.0..=100.0).contains(&c)));
    assert!(s.disk_total_kib.is_some_and(|t| t > 0));

    // The shell is still interactive and saw none of the monitor's output.
    handle
        .commands
        .send(SessionCommand::Write(b"echo NX-$((40+2))\n".to_vec()))
        .await
        .unwrap();
    let text = sink.wait_for("NX-42").await;
    assert!(
        !text.contains("NXT:"),
        "monitor output leaked into the terminal"
    );

    // Stop: no new readings after a while.
    handle
        .commands
        .send(SessionCommand::Monitor(None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let count = monitor.0.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(monitor.0.lock().unwrap().len(), count);

    handle.commands.send(SessionCommand::Close).await.unwrap();
    let _ = handle.task.await;
}

/// Where the test server can reach itself (it's also its own jump host).
fn jump_target() -> (String, u16) {
    let host =
        std::env::var("NEXTERM_TEST_JUMP_TARGET_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("NEXTERM_TEST_JUMP_TARGET_PORT")
        .unwrap_or_else(|_| env("NEXTERM_TEST_SSH_PORT"))
        .parse()
        .unwrap();
    (host, port)
}

#[tokio::test]
#[ignore = "needs a real SSH server with TCP forwarding (see module docs)"]
async fn chained_jump_hosts_tunnel_to_the_target() {
    let password = env("NEXTERM_TEST_SSH_PASSWORD");
    let (target_host, target_port) = jump_target();
    // You → hop2 → hop1 → target, all the same test server.
    let mut hop1 = params(&password);
    hop1.host.clone_from(&target_host);
    hop1.port = target_port;
    hop1.jump = Some(Box::new(params(&password)));
    let mut target = params(&password);
    target.host = target_host;
    target.port = target_port;
    target.jump = Some(Box::new(hop1));

    let sink = Arc::new(CollectSink::default());
    let handle = SshProtocol::open(target, TERM, sink.clone())
        .await
        .expect("connect through two jump hosts");
    handle
        .commands
        .send(SessionCommand::Write(b"echo JUMP-$((6*7))\n".to_vec()))
        .await
        .unwrap();
    sink.wait_for("JUMP-42").await;
    handle.commands.send(SessionCommand::Close).await.unwrap();
    let _ = handle.task.await;
}

#[tokio::test]
#[ignore = "needs a real SSH server with TCP forwarding (see module docs)"]
async fn jump_host_that_cannot_reach_the_target_gives_a_clear_error() {
    let password = env("NEXTERM_TEST_SSH_PASSWORD");
    let mut target = params(&password);
    target.host = "127.0.0.1".into();
    target.port = 1; // nothing listens there
    target.jump = Some(Box::new(params(&password)));
    let sink = Arc::new(CollectSink::default());
    let err = SshProtocol::open(target, TERM, sink)
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.code(), "CONNECTION", "{err}");
    assert!(
        err.to_string().contains("couldn't reach 127.0.0.1:1"),
        "{err}"
    );
}
