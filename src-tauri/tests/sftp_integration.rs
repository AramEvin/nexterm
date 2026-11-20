//! File-manager integration tests against a real OpenSSH server (SFTP).
//! Same server and environment variables as `ssh_integration.rs`:
//!
//! ```text
//! NEXTERM_TEST_SSH_HOST=127.0.0.1 NEXTERM_TEST_SSH_PORT=2222 \
//! NEXTERM_TEST_SSH_USER=ntest NEXTERM_TEST_SSH_PASSWORD=Test-Pass-123 \
//!   cargo test --test sftp_integration -- --ignored --test-threads=1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexterm_lib::sftp::fs::Fs;
use nexterm_lib::sftp::paths::remote_join;
use nexterm_lib::sftp::transfer::{Conflict, EventSink, Job, PART_SUFFIX, TransferEvent};
use nexterm_lib::sftp::{EntryKind, RemoteFs, Transfers};
use nexterm_lib::ssh::prompt::{HostKeyDecision, PromptResponse, PromptSink, SshPrompt};
use nexterm_lib::ssh::{Interaction, PromptBroker, SshAuth, SshConnectParams};
use secrecy::SecretString;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

struct AcceptOnce(Arc<PromptBroker>);

impl PromptSink for AcceptOnce {
    fn send(&self, prompt: SshPrompt) -> bool {
        if let SshPrompt::HostKey { prompt_id, .. } = prompt {
            self.0
                .respond(
                    &prompt_id,
                    PromptResponse::HostKey {
                        decision: HostKeyDecision::AcceptOnce,
                    },
                )
                .unwrap();
        }
        true
    }
}

async fn open() -> Arc<RemoteFs> {
    let broker = Arc::new(PromptBroker::new());
    let params = SshConnectParams {
        host: env("NEXTERM_TEST_SSH_HOST"),
        port: env("NEXTERM_TEST_SSH_PORT").parse().unwrap(),
        username: env("NEXTERM_TEST_SSH_USER"),
        auth: SshAuth::Password(SecretString::from(env("NEXTERM_TEST_SSH_PASSWORD"))),
        interaction: Interaction {
            broker: broker.clone(),
            prompts: Arc::new(AcceptOnce(broker)),
            known_hosts: None,
            openssh_known_hosts: None,
            agent_socket: None,
        },
        jump: None,
    };
    Arc::new(RemoteFs::open(params).await.unwrap())
}

/// A fresh folder in the remote home, removed by the test at the end.
async fn scratch(fs: &RemoteFs) -> String {
    let dir = remote_join(
        &fs.home().await.unwrap(),
        &format!("nexterm-test-{}", uuid::Uuid::new_v4().simple()),
    );
    fs.mkdir(&dir).await.unwrap();
    dir
}

#[derive(Default)]
struct Events(Mutex<Vec<TransferEvent>>);

impl EventSink for Events {
    fn send(&self, event: TransferEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn finished(&self) -> Option<TransferEvent> {
        self.0.lock().unwrap().last().cloned().filter(|e| {
            matches!(
                e,
                TransferEvent::Done { .. }
                    | TransferEvent::Failed { .. }
                    | TransferEvent::Cancelled
            )
        })
    }
}

async fn transfer(transfers: &Arc<Transfers>, job: Job) -> TransferEvent {
    let events = Arc::new(Events::default());
    transfers.start(job, events.clone()).unwrap();
    for _ in 0..1200 {
        if let Some(e) = events.finished() {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("transfer did not finish");
}

#[tokio::test]
#[ignore = "needs a real SSH server; see the module docs"]
async fn browse_and_file_operations() {
    let fs = open().await;
    let dir = scratch(&fs).await;

    fs.mkdir(&remote_join(&dir, "sub")).await.unwrap();
    put(&fs, remote_join(&dir, "a.txt"), b"hello").await;
    let listing = fs.list(&dir).await.unwrap();
    assert_eq!(listing.path, dir);
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["sub", "a.txt"]);
    assert_eq!(listing.entries[0].kind, EntryKind::Dir);
    assert_eq!(listing.entries[1].size, 5);

    // Rename refuses to overwrite.
    let err = fs
        .rename(&remote_join(&dir, "a.txt"), &remote_join(&dir, "sub"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "FILE");
    fs.rename(&remote_join(&dir, "a.txt"), &remote_join(&dir, "b.txt"))
        .await
        .unwrap();

    fs.chmod(&remote_join(&dir, "b.txt"), 0o600).await.unwrap();
    let listing = fs.list(&dir).await.unwrap();
    let b = listing.entries.iter().find(|e| e.name == "b.txt").unwrap();
    assert_eq!(b.permissions, Some(0o600));

    let missing = fs.list(&remote_join(&dir, "nope")).await.unwrap_err();
    assert_eq!(missing.code(), "FILE");
    assert!(missing.user_message().contains("does not exist"));

    // Recursive delete.
    put(&fs, remote_join(&dir, "sub/inner.txt"), b"x").await;
    assert_eq!(fs.remove(&dir).await.unwrap(), 4);
    assert!(!fs.exists(&dir).await);
    fs.close().await;
}

#[tokio::test]
#[ignore = "needs a real SSH server; see the module docs"]
async fn upload_download_and_relay_round_trip() {
    let fs = open().await;
    let dir = scratch(&fs).await;
    let transfers = Arc::new(Transfers::new());

    // Local tree: a 20 MB file (many chunks), an empty folder, a nested file.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("payload");
    std::fs::create_dir_all(src.join("nested/empty")).unwrap();
    let big: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(src.join("big.bin"), &big).unwrap();
    std::fs::write(src.join("nested/n.txt"), b"nested").unwrap();
    let old = std::time::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    std::fs::File::options()
        .write(true)
        .open(src.join("nested/n.txt"))
        .unwrap()
        .set_modified(old)
        .unwrap();

    // Upload.
    let ev = transfer(
        &transfers,
        Job {
            source: Fs::Local,
            sources: vec![src.display().to_string()],
            target: Fs::Remote(fs.clone()),
            target_dir: dir.clone(),
            conflict: Conflict::Overwrite,
        },
    )
    .await;
    assert!(
        matches!(ev, TransferEvent::Done { files_done: 2, .. }),
        "{ev:?}"
    );
    let remote_big = fs
        .sftp
        .read(remote_join(&dir, "payload/big.bin"))
        .await
        .unwrap();
    assert_eq!(remote_big, big);
    let attrs = fs
        .sftp
        .metadata(remote_join(&dir, "payload/nested/n.txt"))
        .await
        .unwrap();
    assert_eq!(attrs.mtime, Some(1_500_000_000));
    assert!(fs.exists(&remote_join(&dir, "payload/nested/empty")).await);

    // Relay to a second connection (remote -> remote through this PC).
    let other = open().await;
    let relay_dir = remote_join(&dir, "relay");
    other.mkdir(&relay_dir).await.unwrap();
    let ev = transfer(
        &transfers,
        Job {
            source: Fs::Remote(fs.clone()),
            sources: vec![remote_join(&dir, "payload")],
            target: Fs::Remote(other.clone()),
            target_dir: relay_dir.clone(),
            conflict: Conflict::Overwrite,
        },
    )
    .await;
    assert!(
        matches!(ev, TransferEvent::Done { files_done: 2, .. }),
        "{ev:?}"
    );

    // Download the relayed copy and compare.
    let down = tmp.path().join("down");
    std::fs::create_dir(&down).unwrap();
    let ev = transfer(
        &transfers,
        Job {
            source: Fs::Remote(other.clone()),
            sources: vec![remote_join(&relay_dir, "payload")],
            target: Fs::Local,
            target_dir: down.display().to_string(),
            conflict: Conflict::Overwrite,
        },
    )
    .await;
    assert!(
        matches!(ev, TransferEvent::Done { files_done: 2, .. }),
        "{ev:?}"
    );
    assert_eq!(std::fs::read(down.join("payload/big.bin")).unwrap(), big);
    assert_eq!(
        std::fs::read(down.join("payload/nested/n.txt")).unwrap(),
        b"nested"
    );
    assert_eq!(
        std::fs::metadata(down.join("payload/nested/n.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        old
    );

    // Same download again with "keep both": a renamed copy appears.
    let ev = transfer(
        &transfers,
        Job {
            source: Fs::Remote(other.clone()),
            sources: vec![remote_join(&relay_dir, "payload/nested/n.txt")],
            target: Fs::Local,
            target_dir: down.join("payload/nested").display().to_string(),
            conflict: Conflict::Rename,
        },
    )
    .await;
    assert!(matches!(ev, TransferEvent::Done { .. }), "{ev:?}");
    assert!(down.join("payload/nested/n (1).txt").exists());

    fs.remove(&dir).await.unwrap();
    fs.close().await;
    other.close().await;
}

#[tokio::test]
#[ignore = "needs a real SSH server; see the module docs"]
async fn hostile_names_from_the_server_never_escape_the_target() {
    let fs = open().await;
    let dir = scratch(&fs).await;
    let transfers = Arc::new(Transfers::new());

    // Legal POSIX names that would be path traversal on Windows / unsafe locally.
    let tree = remote_join(&dir, "tree");
    fs.mkdir(&tree).await.unwrap();
    for name in [r"..\..\escaped.txt", r"C:\evil.txt", "ok.txt"] {
        put(&fs, remote_join(&tree, name), b"data").await;
    }
    let tmp = tempfile::tempdir().unwrap();
    let down = tmp.path().join("down");
    std::fs::create_dir(&down).unwrap();
    let ev = transfer(
        &transfers,
        Job {
            source: Fs::Remote(fs.clone()),
            sources: vec![tree.clone()],
            target: Fs::Local,
            target_dir: down.display().to_string(),
            conflict: Conflict::Overwrite,
        },
    )
    .await;
    match ev {
        TransferEvent::Done {
            files_done,
            skipped,
            ..
        } => {
            assert_eq!(files_done, 1);
            assert_eq!(skipped, 2);
        }
        other => panic!("{other:?}"),
    }
    assert!(down.join("tree/ok.txt").exists());
    // Nothing was written outside the chosen folder.
    let mut all = Vec::new();
    for e in walk(tmp.path()) {
        all.push(e);
    }
    assert_eq!(all.len(), 3, "{all:?}"); // down, down/tree, down/tree/ok.txt
    fs.remove(&dir).await.unwrap();
    fs.close().await;
}

#[tokio::test]
#[ignore = "needs a real SSH server; see the module docs"]
async fn cancelled_upload_leaves_no_partial_file() {
    let fs = open().await;
    let dir = scratch(&fs).await;
    let transfers = Arc::new(Transfers::new());
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("huge.bin");
    std::fs::write(&src, vec![0u8; 200 * 1024 * 1024]).unwrap();

    let events = Arc::new(Events::default());
    let id = transfers
        .start(
            Job {
                source: Fs::Local,
                sources: vec![src.display().to_string()],
                target: Fs::Remote(fs.clone()),
                target_dir: dir.clone(),
                conflict: Conflict::Overwrite,
            },
            events.clone(),
        )
        .unwrap();
    // Wait for the first progress event, then cancel.
    for _ in 0..400 {
        if events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, TransferEvent::Progress { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(transfers.cancel(id));
    for _ in 0..400 {
        if events.finished().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(matches!(events.finished(), Some(TransferEvent::Cancelled)));
    assert!(!fs.exists(&remote_join(&dir, "huge.bin")).await);
    assert!(
        !fs.exists(&remote_join(&dir, &format!("huge.bin{PART_SUFFIX}")))
            .await
    );
    fs.remove(&dir).await.unwrap();
    fs.close().await;
}

async fn put(fs: &RemoteFs, path: String, data: &[u8]) {
    use tokio::io::AsyncWriteExt;
    let mut f = fs.sftp.create(path).await.unwrap();
    f.write_all(data).await.unwrap();
    f.shutdown().await.unwrap();
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        out.push(p.clone());
        if p.is_dir() {
            out.extend(walk(&p));
        }
    }
    out
}

#[derive(Default)]
struct EditEvents(Mutex<Vec<nexterm_lib::sftp::edit::EditEvent>>);

impl nexterm_lib::sftp::edit::EditSink for EditEvents {
    fn send(&self, event: nexterm_lib::sftp::edit::EditEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[tokio::test]
#[ignore = "needs a real SSH server; see the module docs"]
async fn edit_remote_file_uploads_saves_and_cleans_up() {
    use nexterm_lib::sftp::Edits;
    use nexterm_lib::sftp::edit::EditEvent;

    let fs = open().await;
    let dir = scratch(&fs).await;
    let remote = remote_join(&dir, "app.conf");
    put(&fs, remote.clone(), b"port=80\n").await;

    let edits = Arc::new(Edits::new());
    let events = Arc::new(EditEvents::default());
    // `/bin/true` stands in for Notepad: it exits at once; the test "saves".
    let id = edits
        .start(
            Fs::Remote(fs.clone()),
            remote.clone(),
            Some("/bin/true".into()),
            events.clone(),
        )
        .await
        .unwrap();
    let local = match events.0.lock().unwrap().first().cloned() {
        Some(EditEvent::Opened { local_path, .. }) => std::path::PathBuf::from(local_path),
        other => panic!("{other:?}"),
    };
    assert_eq!(std::fs::read(&local).unwrap(), b"port=80\n");

    std::fs::write(&local, b"port=8080\n").unwrap();
    for _ in 0..100 {
        if events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, EditEvent::Uploaded { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(fs.sftp.read(remote.clone()).await.unwrap(), b"port=8080\n");

    assert!(edits.stop(id));
    for _ in 0..50 {
        if events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, EditEvent::Closed))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!local.exists(), "local copy must be deleted");
    fs.remove(&dir).await.unwrap();
    fs.close().await;
}
