//! Phase 8 terminals end to end: Telnet against an in-process fake server and
//! a local shell in a real PTY (both run with plain `cargo test`), plus a
//! serial line over a `socat` pseudo-terminal pair (ignored by default; needs
//! socat: `cargo test --test terminals -- --ignored`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexterm_lib::session::protocol::{
    OutputSink, SessionCommand, SessionStatus, TerminalProtocol, TerminalSize,
};
use nexterm_lib::telnet::{TelnetParams, TelnetProtocol};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };

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
    async fn wait_closed(&self) -> String {
        for _ in 0..200 {
            if let Some(SessionStatus::Closed { reason }) = self.status.lock().unwrap().first() {
                return reason.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("session never closed");
    }
}

/// Read from the client until `needle` shows up (or time out).
async fn read_until(sock: &mut tokio::net::TcpStream, seen: &mut Vec<u8>, needle: &[u8]) {
    let mut buf = [0u8; 1024];
    for _ in 0..100 {
        if seen.windows(needle.len()).any(|w| w == needle) {
            return;
        }
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(50), sock.read(&mut buf)).await
        {
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
    }
    panic!("client never sent {needle:?}; got {seen:?}");
}

#[tokio::test]
async fn telnet_negotiates_and_carries_a_session() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // DO NAWS, DO TTYPE, WILL ECHO, then ask for the terminal type.
        sock.write_all(&[255, 253, 31, 255, 253, 24, 255, 251, 1])
            .await
            .unwrap();
        sock.write_all(&[255, 250, 24, 1, 255, 240]).await.unwrap();
        sock.write_all(b"login: ").await.unwrap();
        let mut seen = Vec::new();
        // Window size 80x24 and terminal type, as the client agreed.
        read_until(
            &mut sock,
            &mut seen,
            &[255, 250, 31, 0, 80, 0, 24, 255, 240],
        )
        .await;
        read_until(&mut sock, &mut seen, b"XTERM-256COLOR").await;
        // Enter arrives as CR LF.
        read_until(&mut sock, &mut seen, b"admin\r\n").await;
        sock.write_all(b"\r\nWelcome admin\r\n").await.unwrap();
        // A resize is reported.
        read_until(
            &mut sock,
            &mut seen,
            &[255, 250, 31, 0, 100, 0, 30, 255, 240],
        )
        .await;
        sock.write_all(b"bye\r\n").await.unwrap();
    });

    let sink = Arc::new(CollectSink::default());
    let handle = TelnetProtocol::open(
        TelnetParams {
            host: "127.0.0.1".into(),
            port,
        },
        SIZE,
        sink.clone(),
    )
    .await
    .unwrap();
    sink.wait_for("login: ").await;
    handle
        .commands
        .send(SessionCommand::Write(b"admin\r".to_vec()))
        .await
        .unwrap();
    sink.wait_for("Welcome admin").await;
    handle
        .commands
        .send(SessionCommand::Resize(TerminalSize {
            cols: 100,
            rows: 30,
        }))
        .await
        .unwrap();
    sink.wait_for("bye").await;
    server.await.unwrap();
    // No negotiation bytes leak onto the screen.
    assert!(!sink.text().contains('\u{fffd}'), "{:?}", sink.text());
    assert_eq!(
        sink.wait_closed().await,
        "The server closed the connection."
    );
}

#[tokio::test]
async fn telnet_refused_connection_is_a_clear_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let err = TelnetProtocol::open(
        TelnetParams {
            host: "127.0.0.1".into(),
            port,
        },
        SIZE,
        Arc::new(CollectSink::default()),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.code(), "CONNECTION");
}

#[cfg(unix)]
#[tokio::test]
async fn local_shell_runs_in_a_pty_and_reports_exit() {
    use nexterm_lib::local::LocalProtocol;
    use nexterm_lib::session::options::{LocalOptions, LocalShell};
    let sink = Arc::new(CollectSink::default());
    let handle = LocalProtocol::open(
        LocalOptions {
            shell: LocalShell::Default,
            wsl_distro: String::new(),
        },
        SIZE,
        sink.clone(),
    )
    .await
    .unwrap();
    handle
        .commands
        .send(SessionCommand::Write(
            b"echo LOCAL-$((6*7)) $TERM; stty size\n".to_vec(),
        ))
        .await
        .unwrap();
    sink.wait_for("LOCAL-42 xterm-256color").await;
    sink.wait_for("24 80").await;
    handle
        .commands
        .send(SessionCommand::Resize(TerminalSize {
            cols: 100,
            rows: 30,
        }))
        .await
        .unwrap();
    handle
        .commands
        .send(SessionCommand::Write(b"stty size\n".to_vec()))
        .await
        .unwrap();
    sink.wait_for("30 100").await;
    handle
        .commands
        .send(SessionCommand::Write(b"exit 3\n".to_vec()))
        .await
        .unwrap();
    assert!(sink.wait_closed().await.contains("exited with code 3"));
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "needs socat (a pseudo-terminal pair stands in for a COM port)"]
async fn serial_line_over_a_pty_pair() {
    use nexterm_lib::serial::SerialProtocol;
    use nexterm_lib::session::options::SerialOptions;
    use std::io::{BufRead, Read, Write};

    // socat prints the two /dev/pts names it created.
    let mut socat = std::process::Command::new("socat")
        .args(["-d", "-d", "pty,raw,echo=0", "pty,raw,echo=0"])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("socat");
    let mut names = Vec::new();
    let mut err = std::io::BufReader::new(socat.stderr.take().unwrap());
    let mut line = String::new();
    while names.len() < 2 && err.read_line(&mut line).unwrap() > 0 {
        if let Some(pos) = line.find("/dev/pts/") {
            names.push(line[pos..].trim().to_owned());
        }
        line.clear();
    }
    let (ours, device) = (names[0].clone(), names[1].clone());

    let sink = Arc::new(CollectSink::default());
    let handle = SerialProtocol::open(
        SerialOptions {
            port: ours,
            baud_rate: 115_200,
            ..SerialOptions::default()
        },
        SIZE,
        sink.clone(),
    )
    .await
    .unwrap();

    // The "device" end answers what it receives.
    let mut dev = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&device)
        .unwrap();
    dev.write_all(b"Router>").unwrap();
    sink.wait_for("Router>").await;
    handle
        .commands
        .send(SessionCommand::Write(b"show version\r".to_vec()))
        .await
        .unwrap();
    // Blocking read off the runtime thread (the session task must keep running).
    let got = tokio::task::spawn_blocking(move || {
        let mut got = vec![0u8; 13];
        dev.read_exact(&mut got).unwrap();
        got
    })
    .await
    .unwrap();
    assert_eq!(&got, b"show version\r");

    handle.commands.send(SessionCommand::Close).await.unwrap();
    assert_eq!(sink.wait_closed().await, "Closed.");
    let _ = socat.kill();
    let _ = socat.wait();
}
