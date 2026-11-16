//! The transfer queue: streamed, cancellable copies between any two file
//! systems (local→remote, remote→local, remote→remote relayed through this PC).
//!
//! A job runs in two steps:
//! 1. **Plan**: expand folders (symlinks are skipped, never followed), check
//!    every name coming from the source against the target's rules (a server
//!    can't make us write `..\..\x`), apply the conflict policy, sum the size.
//! 2. **Copy**: stream each file in 256 KiB chunks (u64 counters, so files
//!    larger than 4 GB are fine), report progress every ~150 ms, keep the
//!    modification time. Local targets are written to `name.nexterm-part`
//!    first and renamed at the end, so a cancelled download never leaves a
//!    half file under the real name.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::fs::{Fs, MAX_DEPTH};
use super::paths::base_name;
use crate::error::{AppError, AppResult};

/// Jobs copying at the same time; the rest wait in the queue.
pub const MAX_ACTIVE: usize = 3;
/// Jobs tracked at once (queued + running).
pub const MAX_TRACKED: usize = 1000;
/// Files + folders one job may contain.
pub const MAX_ITEMS: usize = 200_000;
/// Copy buffer size.
const CHUNK: usize = 256 * 1024;
/// Minimum time between progress events.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);
/// Suffix of in-progress local files.
pub const PART_SUFFIX: &str = ".nexterm-part";
/// Names listed in a "skipped" report.
const MAX_SKIPPED_NAMES: usize = 20;

/// What to do when the target already has an item with the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    /// Replace files; merge into existing folders.
    Overwrite,
    /// Leave the existing item alone.
    Skip,
    /// Keep both: "name (1).ext".
    Rename,
}

/// Progress reported to the UI over the job's channel.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TransferEvent {
    /// Waiting for a free slot.
    Queued,
    /// Planning finished; totals are known.
    #[serde(rename_all = "camelCase")]
    Started {
        files_total: u64,
        bytes_total: u64,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
        bytes_per_sec: u64,
        current: String,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        files_done: u64,
        bytes_done: u64,
        skipped: u64,
        skipped_names: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        message: String,
        files_done: u64,
        bytes_done: u64,
    },
    Cancelled,
}

pub trait EventSink: Send + Sync {
    fn send(&self, event: TransferEvent);
}

/// One copy request.
pub struct Job {
    pub source: Fs,
    pub sources: Vec<String>,
    pub target: Fs,
    pub target_dir: String,
    pub conflict: Conflict,
}

pub struct Transfers {
    cancels: std::sync::Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    permits: Arc<Semaphore>,
}

impl Default for Transfers {
    fn default() -> Self {
        Self {
            cancels: std::sync::Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(MAX_ACTIVE)),
        }
    }
}

impl Transfers {
    pub fn new() -> Self {
        Self::default()
    }

    fn cancels(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<AtomicBool>>> {
        self.cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queue a job. Returns its id at once; progress arrives on `sink`.
    pub fn start(self: &Arc<Self>, job: Job, sink: Arc<dyn EventSink>) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut map = self.cancels();
            if map.len() >= MAX_TRACKED {
                return Err(AppError::Validation(
                    "Too many transfers queued. Wait for some to finish.".into(),
                ));
            }
            map.insert(id, Arc::clone(&cancel));
        }
        let this = Arc::clone(self);
        sink.send(TransferEvent::Queued);
        tauri::async_runtime::spawn(async move {
            let permit = this.permits.clone().acquire_owned().await;
            let event = if cancel.load(Ordering::Relaxed) {
                TransferEvent::Cancelled
            } else {
                let mut run = Run::new(job, Arc::clone(&sink), Arc::clone(&cancel));
                let result = run.execute().await;
                run.finish(result)
            };
            drop(permit);
            this.cancels().remove(&id);
            sink.send(event);
        });
        Ok(id)
    }

    /// Ask a job to stop. Returns false if it is unknown or already finished.
    pub fn cancel(&self, id: Uuid) -> bool {
        match self.cancels().get(&id) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn active_count(&self) -> usize {
        self.cancels().len()
    }
}

/// One planned step.
#[derive(Debug)]
struct Item {
    src: String,
    dst: String,
    is_dir: bool,
    size: u64,
    mtime: Option<u32>,
}

struct Run {
    job: Job,
    sink: Arc<dyn EventSink>,
    cancel: Arc<AtomicBool>,
    files_total: u64,
    bytes_total: u64,
    files_done: u64,
    bytes_done: u64,
    skipped: u64,
    skipped_names: Vec<String>,
    last_emit: Instant,
    last_bytes: u64,
    speed: f64,
}

impl Run {
    fn new(job: Job, sink: Arc<dyn EventSink>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            job,
            sink,
            cancel,
            files_total: 0,
            bytes_total: 0,
            files_done: 0,
            bytes_done: 0,
            skipped: 0,
            skipped_names: Vec::new(),
            last_emit: Instant::now(),
            last_bytes: 0,
            speed: 0.0,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn skip(&mut self, name: &str) {
        self.skipped += 1;
        if self.skipped_names.len() < MAX_SKIPPED_NAMES {
            self.skipped_names.push(name.to_owned());
        }
    }

    fn finish(&self, result: AppResult<()>) -> TransferEvent {
        match result {
            Ok(()) => {
                tracing::info!(
                    files = self.files_done,
                    bytes = self.bytes_done,
                    skipped = self.skipped,
                    "transfer finished"
                );
                TransferEvent::Done {
                    files_done: self.files_done,
                    bytes_done: self.bytes_done,
                    skipped: self.skipped,
                    skipped_names: self.skipped_names.clone(),
                }
            }
            Err(AppError::Cancelled) => {
                tracing::info!(files = self.files_done, "transfer cancelled");
                TransferEvent::Cancelled
            }
            Err(e) => {
                tracing::warn!(code = e.code(), files = self.files_done, "transfer failed");
                TransferEvent::Failed {
                    message: e.user_message(),
                    files_done: self.files_done,
                    bytes_done: self.bytes_done,
                }
            }
        }
    }

    async fn execute(&mut self) -> AppResult<()> {
        let plan = self.plan().await?;
        self.files_total = plan.iter().filter(|i| !i.is_dir).count() as u64;
        self.bytes_total = plan.iter().filter(|i| !i.is_dir).map(|i| i.size).sum();
        tracing::info!(
            files = self.files_total,
            bytes = self.bytes_total,
            local_source = self.job.source.is_local(),
            local_target = self.job.target.is_local(),
            "transfer started"
        );
        self.sink.send(TransferEvent::Started {
            files_total: self.files_total,
            bytes_total: self.bytes_total,
        });
        for item in &plan {
            if self.cancelled() {
                return Err(AppError::Cancelled);
            }
            if item.is_dir {
                self.ensure_dir(&item.dst).await?;
            } else {
                self.copy_file(item).await?;
                self.files_done += 1;
            }
        }
        self.emit(true, "");
        Ok(())
    }

    /// Two references to the same file system (so a path can clash with itself)?
    fn same_fs(&self) -> bool {
        match (&self.job.source, &self.job.target) {
            (Fs::Local, Fs::Local) => true,
            (Fs::Remote(a), Fs::Remote(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    async fn plan(&mut self) -> AppResult<Vec<Item>> {
        let mut plan = Vec::new();
        let sources = std::mem::take(&mut self.job.sources);
        for src in &sources {
            if self.cancelled() {
                return Err(AppError::Cancelled);
            }
            let name = base_name(src).to_owned();
            if !self.job.target.accepts_name(&name) {
                self.skip(&name);
                continue;
            }
            let meta = self.job.source.stat(src, true).await?;
            let mut dst = self.job.target.join(&self.job.target_dir, &name);
            let same_fs = self.same_fs();
            if same_fs && meta.is_dir && is_within(&dst, src) {
                return Err(AppError::FileOp(format!(
                    "Cannot copy the folder \"{name}\" into itself."
                )));
            }
            let clashes_with_self = same_fs && paths_equal(&dst, src);
            if clashes_with_self || self.job.target.exists(&dst).await {
                match self.job.conflict {
                    Conflict::Skip => {
                        self.skip(&name);
                        continue;
                    }
                    Conflict::Overwrite if clashes_with_self => {
                        // Overwriting a file with itself would destroy it.
                        self.skip(&name);
                        continue;
                    }
                    Conflict::Overwrite => {}
                    Conflict::Rename => dst = self.free_name(&name).await,
                }
            }
            if meta.is_dir {
                plan.push(Item {
                    src: src.clone(),
                    dst: dst.clone(),
                    is_dir: true,
                    size: 0,
                    mtime: None,
                });
                Box::pin(self.walk(src.clone(), dst, 1, &mut plan)).await?;
            } else {
                plan.push(Item {
                    src: src.clone(),
                    dst,
                    is_dir: false,
                    size: meta.size,
                    mtime: meta.mtime,
                });
            }
            if plan.len() > MAX_ITEMS {
                return Err(too_many());
            }
        }
        self.job.sources = sources;
        Ok(plan)
    }

    async fn walk(
        &mut self,
        src_dir: String,
        dst_dir: String,
        depth: usize,
        plan: &mut Vec<Item>,
    ) -> AppResult<()> {
        if depth > MAX_DEPTH {
            return Err(AppError::FileOp(
                "Folder nesting is too deep (more than 64 levels).".into(),
            ));
        }
        let mut children = self.job.source.read_dir(&src_dir).await?;
        children.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, meta) in children {
            if self.cancelled() {
                return Err(AppError::Cancelled);
            }
            // Never follow links inside a tree (loops, escaping the folder),
            // and never create a name the target can't hold safely.
            if meta.is_symlink || !self.job.target.accepts_name(&name) {
                self.skip(&name);
                continue;
            }
            let src = self.job.source.join(&src_dir, &name);
            let dst = self.job.target.join(&dst_dir, &name);
            if meta.is_dir {
                plan.push(Item {
                    src: src.clone(),
                    dst: dst.clone(),
                    is_dir: true,
                    size: 0,
                    mtime: None,
                });
                Box::pin(self.walk(src, dst, depth + 1, plan)).await?;
            } else {
                plan.push(Item {
                    src,
                    dst,
                    is_dir: false,
                    size: meta.size,
                    mtime: meta.mtime,
                });
            }
            if plan.len() > MAX_ITEMS {
                return Err(too_many());
            }
        }
        Ok(())
    }

    /// "name (1).ext", "name (2).ext", ... the first that does not exist.
    async fn free_name(&self, name: &str) -> String {
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        for n in 1..10_000u32 {
            let candidate = self
                .job
                .target
                .join(&self.job.target_dir, &format!("{stem} ({n}){ext}"));
            if !self.job.target.exists(&candidate).await {
                return candidate;
            }
        }
        self.job.target.join(
            &self.job.target_dir,
            &format!("{stem} ({}){ext}", Uuid::new_v4().simple()),
        )
    }

    async fn ensure_dir(&self, path: &str) -> AppResult<()> {
        match self.job.target.stat(path, true).await {
            Ok(m) if m.is_dir => Ok(()),
            Ok(_) => Err(AppError::FileOp(format!(
                "\"{}\" exists and is not a folder.",
                base_name(path)
            ))),
            Err(_) => self.job.target.mkdir(path).await,
        }
    }

    async fn copy_file(&mut self, item: &Item) -> AppResult<()> {
        let name = base_name(&item.src).to_owned();
        if let Ok(existing) = self.job.target.stat(&item.dst, false).await
            && existing.is_dir
        {
            return Err(AppError::FileOp(format!(
                "Cannot replace the folder \"{name}\" with a file."
            )));
        }
        let temp = if self.job.target.is_local() {
            format!("{}{PART_SUFFIX}", item.dst)
        } else {
            item.dst.clone()
        };
        let result = self.stream(item, &temp, &name).await;
        match result {
            Ok(()) => {
                if self.job.target.is_local()
                    && let Err(e) = self.job.target.replace(&temp, &item.dst).await
                {
                    self.job.target.discard(&temp).await;
                    return Err(e);
                }
                if let Some(mtime) = item.mtime {
                    self.job.target.set_mtime(&item.dst, mtime).await;
                }
                Ok(())
            }
            Err(e) => {
                self.job.target.discard(&temp).await;
                Err(e)
            }
        }
    }

    async fn stream(&mut self, item: &Item, temp: &str, name: &str) -> AppResult<()> {
        let mut reader = self.job.source.open_read(&item.src).await?;
        let mut writer = self.job.target.open_write(temp).await?;
        let mut buf = vec![0u8; CHUNK];
        let read_err = |e: std::io::Error| {
            tracing::debug!(error = %e, "transfer read failed");
            AppError::FileOp(format!("Reading \"{name}\" failed: {e}"))
        };
        let write_err = |e: std::io::Error| {
            tracing::debug!(error = %e, "transfer write failed");
            AppError::FileOp(format!("Writing \"{name}\" failed: {e}"))
        };
        loop {
            if self.cancelled() {
                let _ = writer.shutdown().await;
                return Err(AppError::Cancelled);
            }
            let n = reader.read(&mut buf).await.map_err(read_err)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).await.map_err(write_err)?;
            self.bytes_done += n as u64;
            self.emit(false, name);
        }
        writer.shutdown().await.map_err(write_err)?;
        Ok(())
    }

    fn emit(&mut self, force: bool, current: &str) {
        let elapsed = self.last_emit.elapsed();
        if !force && elapsed < PROGRESS_EVERY {
            return;
        }
        let secs = elapsed.as_secs_f64().max(0.001);
        let instant = (self.bytes_done - self.last_bytes) as f64 / secs;
        // Smooth the speed so the ETA doesn't jump around.
        self.speed = if self.speed == 0.0 {
            instant
        } else {
            self.speed * 0.7 + instant * 0.3
        };
        self.last_emit = Instant::now();
        self.last_bytes = self.bytes_done;
        self.sink.send(TransferEvent::Progress {
            files_done: self.files_done,
            files_total: self.files_total,
            bytes_done: self.bytes_done,
            bytes_total: self.bytes_total,
            bytes_per_sec: self.speed as u64,
            current: current.to_owned(),
        });
    }
}

fn too_many() -> AppError {
    AppError::FileOp(format!(
        "Too many files in one transfer (max {MAX_ITEMS}). Copy the folders separately."
    ))
}

fn normalize(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']).replace('\\', "/");
    if cfg!(windows) { p.to_lowercase() } else { p }
}

fn paths_equal(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

/// Is `path` equal to `dir` or somewhere below it?
fn is_within(path: &str, dir: &str) -> bool {
    let (p, d) = (normalize(path), normalize(dir));
    p == d || p.starts_with(&format!("{d}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collect(Mutex<Vec<TransferEvent>>);
    impl EventSink for Collect {
        fn send(&self, event: TransferEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    async fn run_local(
        sources: Vec<String>,
        target_dir: &std::path::Path,
        conflict: Conflict,
    ) -> TransferEvent {
        let sink = Arc::new(Collect::default());
        let mut run = Run::new(
            Job {
                source: Fs::Local,
                sources,
                target: Fs::Local,
                target_dir: target_dir.display().to_string(),
                conflict,
            },
            sink.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let result = run.execute().await;
        run.finish(result)
    }

    fn s(p: &std::path::Path) -> String {
        p.display().to_string()
    }

    #[tokio::test]
    async fn copies_trees_and_keeps_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("a/b")).unwrap();
        std::fs::write(src.join("top.txt"), b"top").unwrap();
        std::fs::write(src.join("a/b/deep.bin"), vec![7u8; 600_000]).unwrap();
        let old = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        std::fs::File::options()
            .write(true)
            .open(src.join("top.txt"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let dst = tmp.path().join("dst");
        std::fs::create_dir(&dst).unwrap();

        match run_local(vec![s(&src)], &dst, Conflict::Overwrite).await {
            TransferEvent::Done {
                files_done,
                bytes_done,
                skipped,
                ..
            } => {
                assert_eq!(files_done, 2);
                assert_eq!(bytes_done, 600_003);
                assert_eq!(skipped, 0);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            std::fs::read(dst.join("src/a/b/deep.bin")).unwrap().len(),
            600_000
        );
        let mtime = std::fs::metadata(dst.join("src/top.txt"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(mtime, old);
        // No temporary files left behind.
        assert!(!dst.join(format!("src/top.txt{PART_SUFFIX}")).exists());
    }

    #[tokio::test]
    async fn conflict_policies() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("f.txt");
        std::fs::write(&src, b"new").unwrap();
        let dst = tmp.path().join("dst");
        std::fs::create_dir(&dst).unwrap();
        std::fs::write(dst.join("f.txt"), b"old").unwrap();

        let ev = run_local(vec![s(&src)], &dst, Conflict::Skip).await;
        assert!(matches!(ev, TransferEvent::Done { skipped: 1, .. }));
        assert_eq!(std::fs::read(dst.join("f.txt")).unwrap(), b"old");

        run_local(vec![s(&src)], &dst, Conflict::Rename).await;
        assert_eq!(std::fs::read(dst.join("f (1).txt")).unwrap(), b"new");
        assert_eq!(std::fs::read(dst.join("f.txt")).unwrap(), b"old");

        run_local(vec![s(&src)], &dst, Conflict::Overwrite).await;
        assert_eq!(std::fs::read(dst.join("f.txt")).unwrap(), b"new");
    }

    #[tokio::test]
    async fn never_overwrites_a_file_with_itself_or_copies_into_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("same.txt");
        std::fs::write(&f, b"precious").unwrap();
        let ev = run_local(vec![s(&f)], tmp.path(), Conflict::Overwrite).await;
        assert!(matches!(ev, TransferEvent::Done { skipped: 1, .. }));
        assert_eq!(std::fs::read(&f).unwrap(), b"precious");

        let dir = tmp.path().join("dir");
        std::fs::create_dir(&dir).unwrap();
        let ev = run_local(vec![s(&dir)], &dir, Conflict::Overwrite).await;
        assert!(matches!(ev, TransferEvent::Failed { .. }), "{ev:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_inside_trees_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let secret = tmp.path().join("secret");
        std::fs::create_dir(&secret).unwrap();
        std::fs::write(secret.join("key"), b"k").unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::os::unix::fs::symlink(&secret, src.join("link")).unwrap();
        std::fs::write(src.join("ok.txt"), b"ok").unwrap();
        let dst = tmp.path().join("dst");
        std::fs::create_dir(&dst).unwrap();
        let ev = run_local(vec![s(&src)], &dst, Conflict::Overwrite).await;
        assert!(
            matches!(
                ev,
                TransferEvent::Done {
                    files_done: 1,
                    skipped: 1,
                    ..
                }
            ),
            "{ev:?}"
        );
        assert!(!dst.join("src/link").exists());
    }

    #[tokio::test]
    async fn cancel_removes_partial_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("big.bin");
        std::fs::write(&src, vec![1u8; 4 * CHUNK]).unwrap();
        let dst = tmp.path().join("dst");
        std::fs::create_dir(&dst).unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let mut run = Run::new(
            Job {
                source: Fs::Local,
                sources: vec![s(&src)],
                target: Fs::Local,
                target_dir: s(&dst),
                conflict: Conflict::Overwrite,
            },
            Arc::new(Collect::default()),
            cancel,
        );
        let r = run.execute().await;
        assert!(matches!(run.finish(r), TransferEvent::Cancelled));
        assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn queue_reports_events_and_cancel_is_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("x.txt");
        std::fs::write(&src, b"x").unwrap();
        let transfers = Arc::new(Transfers::new());
        let sink = Arc::new(Collect::default());
        let id = transfers
            .start(
                Job {
                    source: Fs::Local,
                    sources: vec![s(&src)],
                    target: Fs::Local,
                    target_dir: s(&tmp.path().join("missing-dir")),
                    conflict: Conflict::Overwrite,
                },
                sink.clone(),
            )
            .unwrap();
        for _ in 0..100 {
            if transfers.active_count() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!transfers.cancel(id));
        let events = sink.0.lock().unwrap();
        assert!(matches!(events.first(), Some(TransferEvent::Queued)));
        assert!(matches!(events.last(), Some(TransferEvent::Failed { .. })));
    }

    #[test]
    fn path_helpers() {
        assert!(is_within("/a/b/c", "/a/b"));
        assert!(is_within("/a/b", "/a/b/"));
        assert!(!is_within("/a/bc", "/a/b"));
        assert!(paths_equal("/x/y/", "/x/y"));
    }
}
