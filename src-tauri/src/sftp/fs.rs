//! One interface over the local disk and an SFTP server, used by transfers.
//! Paths are plain strings: native paths locally, POSIX paths remotely.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use russh_sftp::protocol::{FileAttributes, FileType, OpenFlags};
use tokio::io::{AsyncRead, AsyncWrite};

use super::ftp::FtpFs;
use super::local::io_error;
use super::paths::{remote_join, safe_local_name, safe_remote_name};
use super::remote::{RemoteFs, sftp_error};
use crate::error::AppResult;

/// Deepest folder nesting a recursive operation follows.
pub const MAX_DEPTH: usize = 64;

pub type Reader = Pin<Box<dyn AsyncRead + Send>>;
pub type Writer = Pin<Box<dyn AsyncWrite + Send>>;

#[derive(Clone)]
pub enum Fs {
    Local,
    Remote(Arc<RemoteFs>),
    Ftp(Arc<FtpFs>),
}

/// What a transfer needs to know about a path.
#[derive(Debug, Clone, Copy)]
pub struct Meta {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime: Option<u32>,
}

impl Fs {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    /// Is `name` (which came from the *other* side) safe to create here?
    pub fn accepts_name(&self, name: &str) -> bool {
        match self {
            Self::Local => safe_local_name(name, cfg!(windows)),
            Self::Remote(_) => safe_remote_name(name),
            // FTP commands are text lines: no CR/LF in names.
            Self::Ftp(_) => safe_remote_name(name) && !name.contains(['\r', '\n']),
        }
    }

    pub fn join(&self, dir: &str, name: &str) -> String {
        match self {
            Self::Local => Path::new(dir).join(name).display().to_string(),
            Self::Remote(_) | Self::Ftp(_) => remote_join(dir, name),
        }
    }

    /// Metadata, following symlinks (`follow`) or not.
    pub async fn stat(&self, path: &str, follow: bool) -> AppResult<Meta> {
        match self {
            Self::Local => {
                let p = PathBuf::from(path);
                blocking(move || {
                    let m = if follow {
                        std::fs::metadata(&p)
                    } else {
                        std::fs::symlink_metadata(&p)
                    }
                    .map_err(|e| io_error(&e, &p))?;
                    Ok(Meta {
                        is_dir: m.is_dir(),
                        is_symlink: m.file_type().is_symlink(),
                        size: m.len(),
                        mtime: m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .and_then(|d| u32::try_from(d.as_secs()).ok()),
                    })
                })
                .await
            }
            Self::Remote(r) => {
                let attrs = if follow {
                    r.sftp.metadata(path).await
                } else {
                    r.sftp.symlink_metadata(path).await
                }
                .map_err(|e| sftp_error(&e, path))?;
                Ok(remote_meta(&attrs))
            }
            Self::Ftp(f) => f.stat(path).await,
        }
    }

    pub async fn exists(&self, path: &str) -> bool {
        self.stat(path, false).await.is_ok()
    }

    /// Children of a folder: `(name, meta)` without following symlinks.
    pub async fn read_dir(&self, path: &str) -> AppResult<Vec<(String, Meta)>> {
        match self {
            Self::Local => {
                let p = PathBuf::from(path);
                blocking(move || {
                    let mut out = Vec::new();
                    for item in std::fs::read_dir(&p).map_err(|e| io_error(&e, &p))? {
                        let item = item.map_err(|e| io_error(&e, &p))?;
                        let m = std::fs::symlink_metadata(item.path())
                            .map_err(|e| io_error(&e, &item.path()))?;
                        out.push((
                            item.file_name().to_string_lossy().into_owned(),
                            Meta {
                                is_dir: m.is_dir(),
                                is_symlink: m.file_type().is_symlink(),
                                size: m.len(),
                                mtime: m
                                    .modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                    .and_then(|d| u32::try_from(d.as_secs()).ok()),
                            },
                        ));
                    }
                    Ok(out)
                })
                .await
            }
            Self::Remote(r) => {
                let dir = r
                    .sftp
                    .read_dir(path)
                    .await
                    .map_err(|e| sftp_error(&e, path))?;
                Ok(dir
                    .filter(|e| e.file_name() != "." && e.file_name() != "..")
                    .map(|e| (e.file_name(), remote_meta(&e.metadata())))
                    .collect())
            }
            Self::Ftp(f) => f.read_dir(path).await,
        }
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        match self {
            Self::Local => {
                let p = PathBuf::from(path);
                blocking(move || super::local::mkdir(&p)).await
            }
            Self::Remote(r) => r.mkdir(path).await,
            Self::Ftp(f) => f.mkdir(path).await,
        }
    }

    pub async fn open_read(&self, path: &str) -> AppResult<Reader> {
        match self {
            Self::Local => {
                let f = tokio::fs::File::open(path)
                    .await
                    .map_err(|e| io_error(&e, Path::new(path)))?;
                Ok(Box::pin(f))
            }
            Self::Remote(r) => {
                let f = r.sftp.open(path).await.map_err(|e| sftp_error(&e, path))?;
                Ok(Box::pin(f))
            }
            Self::Ftp(f) => Ok(Box::pin(f.open_read(path).await?)),
        }
    }

    /// Create or truncate `path` for writing.
    pub async fn open_write(&self, path: &str) -> AppResult<Writer> {
        match self {
            Self::Local => {
                let f = tokio::fs::File::create(path)
                    .await
                    .map_err(|e| io_error(&e, Path::new(path)))?;
                Ok(Box::pin(f))
            }
            Self::Remote(r) => {
                let f = r
                    .sftp
                    .open_with_flags(
                        path,
                        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                    )
                    .await
                    .map_err(|e| sftp_error(&e, path))?;
                Ok(Box::pin(f))
            }
            Self::Ftp(f) => Ok(Box::pin(f.open_write(path).await?)),
        }
    }

    /// Move a finished temporary file over the final name (local only; the
    /// rename replaces an existing file on Windows too).
    pub async fn replace(&self, from: &str, to: &str) -> AppResult<()> {
        let (f, t) = (PathBuf::from(from), PathBuf::from(to));
        blocking(move || std::fs::rename(&f, &t).map_err(|e| io_error(&e, &t))).await
    }

    /// Best-effort delete of a partial file after a failed / cancelled copy.
    pub async fn discard(&self, path: &str) {
        match self {
            Self::Local => {
                let _ = tokio::fs::remove_file(path).await;
            }
            Self::Remote(r) => {
                let _ = r.sftp.remove_file(path).await;
            }
            Self::Ftp(f) => {
                let _ = f.remove_file(path).await;
            }
        }
    }

    /// Keep the source's modification time (best effort).
    pub async fn set_mtime(&self, path: &str, mtime: u32) {
        match self {
            Self::Local => {
                let p = PathBuf::from(path);
                let _ = blocking(move || {
                    let f = std::fs::File::options()
                        .write(true)
                        .open(&p)
                        .map_err(|e| io_error(&e, &p))?;
                    f.set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(mtime)))
                        .map_err(|e| io_error(&e, &p))
                })
                .await;
            }
            Self::Remote(r) => {
                let mut attrs = FileAttributes::empty();
                attrs.atime = Some(mtime);
                attrs.mtime = Some(mtime);
                let _ = r.sftp.set_metadata(path, attrs).await;
            }
            Self::Ftp(f) => f.set_mtime(path, mtime).await,
        }
    }
}

fn remote_meta(attrs: &FileAttributes) -> Meta {
    Meta {
        is_dir: attrs.file_type() == FileType::Dir,
        is_symlink: attrs.file_type() == FileType::Symlink,
        size: attrs.size.unwrap_or(0),
        mtime: attrs.mtime,
    }
}

/// Run blocking file-system work off the async runtime.
pub async fn blocking<T, F>(f: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("blocking task failed: {e}")))?
}
