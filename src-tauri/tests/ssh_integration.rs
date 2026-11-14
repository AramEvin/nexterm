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
//! then:
//!
//! ```text
//! NEXTERM_TEST_SSH_HOST=127.0.0.1 NEXTERM_TEST_SSH_PORT=2222 \
//! NEXTERM_TEST_SSH_USER=ntest NEXTERM_TEST_SSH_PASSWORD=Test-Pass-123 \
//!   cargo test --test ssh_integration -- --ignored --test-threads=1
//! ```

// Test code: panicking on unexpected state is the point.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexterm_lib::error::AppError;
use nexterm_lib::session::protocol::{
    OutputSink, SessionCommand, SessionStatus, TerminalProtocol, TerminalSize,
};
use nexterm_lib::ssh::{SshConnectParams, SshProtocol};
use secrecy::SecretString;

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

fn params(password: &str) -> SshConnectParams {
    SshConnectParams {
        host: env("NEXTERM_TEST_SSH_HOST"),
        port: env("NEXTERM_TEST_SSH_PORT").parse().unwrap(),
        username: env("NEXTERM_TEST_SSH_USER"),
        password: SecretString::from(password.to_owned()),
    }
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
