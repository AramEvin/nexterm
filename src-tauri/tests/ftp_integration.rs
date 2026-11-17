//! FTP / FTPS integration tests against real servers (pyftpdlib in CI and
//! locally; see `scripts/test-ftp-server.py`):
//!
//! ```text
//! NEXTERM_TEST_FTP_PORT=2121 NEXTERM_TEST_FTPS_PORT=2990 \
//! NEXTERM_TEST_FTP_USER=ftest NEXTERM_TEST_FTP_PASSWORD=Ftp-Pass-123 \
//!   cargo test --test ftp_integration -- --ignored --test-threads=1
//! ```
//! The FTPS server uses a self-signed certificate; set
//! `NEXTERM_TEST_FTPS_FINGERPRINT` to its `SHA256:<base64>` to check the prompt.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexterm_lib::sftp::fs::Fs;
use nexterm_lib::sftp::ftp::{FtpFs, FtpParams, FtpTrust};
use nexterm_lib::sftp::paths::remote_join;
use nexterm_lib::sftp::transfer::{Conflict, EventSink, Job, TransferEvent};
use nexterm_lib::sftp::{EntryKind, Transfers};
use nexterm_lib::ssh::PromptBroker;
use nexterm_lib::ssh::known_hosts;
use nexterm_lib::ssh::prompt::{HostKeyDecision, PromptResponse, PromptSink, SshPrompt};
use nexterm_lib::storage::Database;
use secrecy::SecretString;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

/// Answers certificate prompts with a fixed decision and records them.
struct Decide {
    broker: Arc<PromptBroker>,
    decision: HostKeyDecision,
    seen: Mutex<Vec<(String, String)>>,
}

impl PromptSink for Decide {
    fn send(&self, prompt: SshPrompt) -> bool {
        if let SshPrompt::HostKey {
            prompt_id,
            kind,
            fingerprint,
            ..
        } = prompt
        {
            self.seen
                .lock()
                .unwrap()
                .push((kind.to_owned(), fingerprint));
            self.broker
                .respond(
                    &prompt_id,
                    PromptResponse::HostKey {
                        decision: self.decision,
                    },
                )
                .unwrap();
        }
        true
    }
}

fn trust(decision: HostKeyDecision, db: Option<Database>) -> (FtpTrust, Arc<Decide>) {
    let broker = Arc::new(PromptBroker::new());
    let sink = Arc::new(Decide {
        broker: broker.clone(),
        decision,
        seen: Mutex::new(Vec::new()),
    });
    (
        FtpTrust {
            broker,
            prompts: sink.clone(),
            known_hosts: db,
        },
        sink,
    )
}

fn params(secure: bool, password: &str) -> FtpParams {
    FtpParams {
        host: "127.0.0.1".into(),
        port: env(if secure {
            "NEXTERM_TEST_FTPS_PORT"
        } else {
            "NEXTERM_TEST_FTP_PORT"
        })
        .parse()
        .unwrap(),
        username: env("NEXTERM_TEST_FTP_USER"),
        password: SecretString::from(password.to_owned()),
        secure,
    }
}

async fn open(secure: bool) -> Arc<FtpFs> {
    let (t, _) = trust(HostKeyDecision::AcceptOnce, None);
    FtpFs::open(params(secure, &env("NEXTERM_TEST_FTP_PASSWORD")), t)
        .await
        .unwrap()
}

#[derive(Default)]
struct Events(Mutex<Vec<TransferEvent>>);

impl EventSink for Events {
    fn send(&self, event: TransferEvent) {
        self.0.lock().unwrap().push(event);
    }
}

async fn transfer(job: Job) -> TransferEvent {
    let transfers = Arc::new(Transfers::new());
    let events = Arc::new(Events::default());
    transfers.start(job, events.clone()).unwrap();
    for _ in 0..1200 {
        let last = events.0.lock().unwrap().last().cloned();
        if let Some(
            e @ (TransferEvent::Done { .. }
            | TransferEvent::Failed { .. }
            | TransferEvent::Cancelled),
        ) = last
        {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("transfer did not finish");
}

/// Upload a local tree, browse it, download it back, then clean up.
async fn round_trip(secure: bool) {
    let fs = open(secure).await;
    let home = fs.home().await.unwrap();
    let dir = remote_join(&home, &format!("nx-{}", uuid::Uuid::new_v4().simple()));
    fs.mkdir(&dir).await.unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("tree");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    let big: Vec<u8> = (0..5 * 1024 * 1024u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(src.join("big.bin"), &big).unwrap();
    std::fs::write(src.join("sub/small.txt"), b"hello ftp").unwrap();

    let ev = transfer(Job {
        source: Fs::Local,
        sources: vec![src.display().to_string()],
        target: Fs::Ftp(fs.clone()),
        target_dir: dir.clone(),
        conflict: Conflict::Overwrite,
    })
    .await;
    assert!(
        matches!(ev, TransferEvent::Done { files_done: 2, .. }),
        "{ev:?}"
    );

    let listing = fs.list(&remote_join(&dir, "tree")).await.unwrap();
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["sub", "big.bin"]);
    assert_eq!(listing.entries[0].kind, EntryKind::Dir);
    assert_eq!(listing.entries[1].size, big.len() as u64);

    // Rename (refuses to overwrite), chmod, stat.
    let a = remote_join(&dir, "tree/big.bin");
    let b = remote_join(&dir, "tree/renamed.bin");
    assert_eq!(
        fs.rename(&a, &remote_join(&dir, "tree/sub"))
            .await
            .unwrap_err()
            .code(),
        "FILE"
    );
    fs.rename(&a, &b).await.unwrap();
    fs.chmod(&b, 0o600).await.unwrap();
    let listing = fs.list(&remote_join(&dir, "tree")).await.unwrap();
    let renamed = listing
        .entries
        .iter()
        .find(|e| e.name == "renamed.bin")
        .unwrap();
    assert_eq!(renamed.permissions, Some(0o600));
    let meta = fs.stat(&b).await.unwrap();
    assert!(!meta.is_dir);
    assert_eq!(meta.size, big.len() as u64);

    // Download and compare.
    let down = tmp.path().join("down");
    std::fs::create_dir(&down).unwrap();
    let ev = transfer(Job {
        source: Fs::Ftp(fs.clone()),
        sources: vec![remote_join(&dir, "tree")],
        target: Fs::Local,
        target_dir: down.display().to_string(),
        conflict: Conflict::Overwrite,
    })
    .await;
    assert!(
        matches!(ev, TransferEvent::Done { files_done: 2, .. }),
        "{ev:?}"
    );
    assert_eq!(std::fs::read(down.join("tree/renamed.bin")).unwrap(), big);
    assert_eq!(
        std::fs::read(down.join("tree/sub/small.txt")).unwrap(),
        b"hello ftp"
    );

    // Command injection through names is refused before reaching the server.
    let err = fs.mkdir(&format!("{dir}/x\r\nDELE {b}")).await.unwrap_err();
    assert_eq!(err.code(), "VALIDATION");
    assert!(fs.exists(&b).await);

    assert_eq!(fs.remove(&dir).await.unwrap(), 5);
    assert!(!fs.exists(&dir).await);
    fs.close().await;
}

#[tokio::test]
#[ignore = "needs FTP servers; see the module docs"]
async fn ftp_round_trip() {
    round_trip(false).await;
}

#[tokio::test]
#[ignore = "needs FTP servers; see the module docs"]
async fn ftps_round_trip() {
    round_trip(true).await;
}

#[tokio::test]
#[ignore = "needs FTP servers; see the module docs"]
async fn wrong_password_is_auth_failed() {
    let (t, _) = trust(HostKeyDecision::AcceptOnce, None);
    let err = FtpFs::open(params(false, "wrong"), t).await.unwrap_err();
    assert_eq!(err.code(), "AUTH_FAILED");
}

#[tokio::test]
#[ignore = "needs FTP servers; see the module docs"]
async fn self_signed_certificate_tofu_and_change_detection() {
    let db = Database::open_in_memory().unwrap();
    let port: u16 = env("NEXTERM_TEST_FTPS_PORT").parse().unwrap();
    let password = env("NEXTERM_TEST_FTP_PASSWORD");

    // Reject -> no connection.
    let (t, _) = trust(HostKeyDecision::Reject, Some(db.clone()));
    let err = FtpFs::open(params(true, &password), t).await.unwrap_err();
    assert_eq!(err.code(), "HOST_KEY_REJECTED");

    // Accept & save -> prompt shows the certificate fingerprint; then pinned.
    let (t, sink) = trust(HostKeyDecision::AcceptAndSave, Some(db.clone()));
    let fs = FtpFs::open(params(true, &password), t).await.unwrap();
    let seen = sink.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "tls");
    if let Ok(expected) = std::env::var("NEXTERM_TEST_FTPS_FINGERPRINT") {
        assert_eq!(seen[0].1, expected);
    }
    // Data connections (listing) work with the pinned certificate.
    fs.list(&fs.home().await.unwrap()).await.unwrap();
    fs.close().await;

    // Saved -> next time no prompt.
    let (t, sink) = trust(HostKeyDecision::Reject, Some(db.clone()));
    let fs = FtpFs::open(params(true, &password), t).await.unwrap();
    assert!(sink.seen.lock().unwrap().is_empty());
    fs.close().await;

    // A different saved fingerprint -> blocked, no prompt.
    db.with(|c| known_hosts::save_cert(c, "127.0.0.1", port, "SHA256:somethingelse"))
        .unwrap();
    let (t, sink) = trust(HostKeyDecision::AcceptAndSave, Some(db.clone()));
    let err = FtpFs::open(params(true, &password), t).await.unwrap_err();
    assert_eq!(err.code(), "HOST_KEY_CHANGED");
    assert!(err.user_message().contains("CHANGED"));
    assert!(sink.seen.lock().unwrap().is_empty());
}
