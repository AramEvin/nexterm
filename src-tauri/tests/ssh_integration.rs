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
        panic!("timed out waiting for {needle:?}; output so far:\n{}", self.text());
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
        TerminalSize { cols: 100, rows: 30 },
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
        .send(SessionCommand::Resize(TerminalSize { cols: 132, rows: 43 }))
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
    assert!(err.user_message().contains("refused"), "{}", err.user_message());
}
