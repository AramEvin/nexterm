//! A remote file system over SFTP (one SSH connection per pane).

use russh::Disconnect;
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, FileType, StatusCode};

use super::paths::{remote_join, remote_parent};
use super::{EntryKind, FileEntry, Listing, MAX_LISTING, sort_entries};
use crate::error::{AppError, AppResult};
use crate::ssh::{SshClient, SshConnectParams, connect};

/// Seconds the server may take to answer one SFTP request.
const REQUEST_TIMEOUT_SECS: u64 = 30;

pub struct RemoteFs {
    client: SshClient,
    pub sftp: SftpSession,
    /// "user@host:port", for logs and the UI.
    pub label: String,
}

impl std::fmt::Debug for RemoteFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteFs")
            .field("label", &self.label)
            .finish()
    }
}

impl RemoteFs {
    /// Connect (host key + auth as for terminals) and start the SFTP subsystem.
    pub async fn open(params: SshConnectParams) -> AppResult<Self> {
        let label = format!("{}@{}:{}", params.username, params.host, params.port);
        let client = connect(params).await?;
        let sftp = match start_sftp(&client).await {
            Ok(s) => s,
            Err(e) => {
                let _ = client
                    .handle
                    .disconnect(Disconnect::ByApplication, "", "en")
                    .await;
                return Err(e);
            }
        };
        sftp.set_timeout(REQUEST_TIMEOUT_SECS);
        tracing::info!(host = %client.host, port = client.port, "SFTP session opened");
        Ok(Self {
            client,
            sftp,
            label,
        })
    }

    pub async fn close(&self) {
        let _ = self.sftp.close().await;
        let _ = self
            .client
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
        tracing::info!(host = %self.client.host, port = self.client.port, "SFTP session closed");
    }

    pub fn is_closed(&self) -> bool {
        self.client.handle.is_closed()
    }

    /// The login folder (`realpath(".")`).
    pub async fn home(&self) -> AppResult<String> {
        self.sftp
            .canonicalize(".")
            .await
            .map_err(|e| sftp_error(&e, "home folder"))
    }

    pub async fn list(&self, path: &str) -> AppResult<Listing> {
        let canonical = self
            .sftp
            .canonicalize(path)
            .await
            .map_err(|e| sftp_error(&e, path))?;
        let dir = self
            .sftp
            .read_dir(canonical.clone())
            .await
            .map_err(|e| sftp_error(&e, &canonical))?;
        let mut entries = Vec::new();
        let mut truncated = false;
        let mut links = Vec::new();
        for item in dir {
            let name = item.file_name();
            if name == "." || name == ".." {
                continue;
            }
            if entries.len() >= MAX_LISTING {
                truncated = true;
                break;
            }
            let meta = item.metadata();
            let path = remote_join(&canonical, &name);
            let entry = entry_from_attrs(name, path, &meta);
            if entry.kind == EntryKind::Symlink {
                links.push(entries.len());
            }
            entries.push(entry);
        }
        // Resolve where symlinks point (bounded: at most 200 extra requests).
        for idx in links.into_iter().take(200) {
            if let Ok(target) = self.sftp.metadata(entries[idx].path.clone()).await {
                entries[idx].link_to_dir = target.file_type() == FileType::Dir;
                if !entries[idx].link_to_dir {
                    entries[idx].size = target.len();
                }
            }
        }
        sort_entries(&mut entries);
        Ok(Listing {
            parent: remote_parent(&canonical),
            path: canonical,
            entries,
            truncated,
        })
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| sftp_error(&e, path))
    }

    pub async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        if self.exists(to).await {
            return Err(AppError::FileOp(format!(
                "\"{}\" already exists.",
                super::paths::base_name(to)
            )));
        }
        self.sftp
            .rename(from, to)
            .await
            .map_err(|e| sftp_error(&e, from))
    }

    pub async fn exists(&self, path: &str) -> bool {
        self.sftp.symlink_metadata(path).await.is_ok()
    }

    /// Delete a file, symlink or folder (recursively; symlinks are removed,
    /// never followed). Returns the number of items deleted.
    pub async fn remove(&self, path: &str) -> AppResult<u64> {
        let meta = self
            .sftp
            .symlink_metadata(path)
            .await
            .map_err(|e| sftp_error(&e, path))?;
        let mut count = 0;
        if meta.file_type() == FileType::Dir {
            Box::pin(self.remove_tree(path.to_owned(), 0, &mut count)).await?;
        } else {
            self.sftp
                .remove_file(path)
                .await
                .map_err(|e| sftp_error(&e, path))?;
            count += 1;
        }
        Ok(count)
    }

    async fn remove_tree(&self, dir: String, depth: usize, count: &mut u64) -> AppResult<()> {
        if depth > super::fs::MAX_DEPTH {
            return Err(AppError::FileOp("Folder nesting is too deep.".into()));
        }
        let items = self
            .sftp
            .read_dir(dir.clone())
            .await
            .map_err(|e| sftp_error(&e, &dir))?;
        for item in items {
            let name = item.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child = remote_join(&dir, &name);
            if item.file_type() == FileType::Dir {
                Box::pin(self.remove_tree(child, depth + 1, count)).await?;
            } else {
                self.sftp
                    .remove_file(child.clone())
                    .await
                    .map_err(|e| sftp_error(&e, &child))?;
                *count += 1;
            }
        }
        self.sftp
            .remove_dir(dir.clone())
            .await
            .map_err(|e| sftp_error(&e, &dir))?;
        *count += 1;
        Ok(())
    }

    /// Set permission bits (`0o644`); file-type bits are never touched.
    pub async fn chmod(&self, path: &str, mode: u32) -> AppResult<()> {
        let mut attrs = FileAttributes::empty();
        attrs.permissions = Some(mode & 0o7777);
        self.sftp
            .set_metadata(path, attrs)
            .await
            .map_err(|e| sftp_error(&e, path))
    }
}

async fn start_sftp(client: &SshClient) -> AppResult<SftpSession> {
    let fail = |e: &dyn std::fmt::Display| {
        tracing::warn!(host = %client.host, error = %e, "SFTP subsystem failed");
        AppError::Connection(
            "The server does not offer SFTP (the sftp subsystem may be disabled).".into(),
        )
    };
    let channel = client
        .handle
        .channel_open_session()
        .await
        .map_err(|e| fail(&e))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| fail(&e))?;
    let config = russh_sftp::client::Config {
        request_timeout_secs: REQUEST_TIMEOUT_SECS,
        ..Default::default()
    };
    SftpSession::new_with_config(channel.into_stream(), config)
        .await
        .map_err(|e| fail(&e))
}

pub fn entry_from_attrs(name: String, path: String, meta: &FileAttributes) -> FileEntry {
    let kind = match meta.file_type() {
        FileType::Dir => EntryKind::Dir,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    };
    FileEntry {
        hidden: name.starts_with('.'),
        name,
        path,
        kind,
        link_to_dir: false,
        size: meta.size.unwrap_or(0),
        modified: meta.mtime.map(i64::from),
        permissions: meta.permissions.map(|p| p & 0o7777),
        owner: meta
            .user
            .clone()
            .or_else(|| meta.uid.map(|u| u.to_string())),
        group: meta
            .group
            .clone()
            .or_else(|| meta.gid.map(|g| g.to_string())),
    }
}

/// Turn an SFTP failure into a short, user-facing message about `what`.
pub fn sftp_error(err: &SftpError, what: &str) -> AppError {
    let name = super::paths::base_name(what);
    tracing::debug!(error = %err, "SFTP request failed");
    match err {
        SftpError::Status(s) => AppError::FileOp(match s.status_code {
            StatusCode::NoSuchFile => format!("\"{name}\" does not exist."),
            StatusCode::PermissionDenied => format!("Permission denied: \"{name}\"."),
            StatusCode::OpUnsupported => "The server does not support this operation.".into(),
            StatusCode::Failure if s.error_message.is_empty() => format!(
                "The server refused the operation on \"{name}\" (it may already exist, or a \
                 folder may not be empty)."
            ),
            _ if !s.error_message.is_empty() => format!("{}: \"{name}\"", s.error_message),
            _ => format!("The operation on \"{name}\" failed."),
        }),
        SftpError::Timeout => AppError::Connection("The server did not respond in time.".into()),
        SftpError::IO(_) => AppError::Connection("The SFTP connection was lost.".into()),
        SftpError::Limited(m) => AppError::FileOp(format!("Server limit exceeded: {m}")),
        // Seen when the server's sftp-server process ends (killed, idle
        // timeout) while the SSH connection itself is still up.
        SftpError::UnexpectedPacket | SftpError::UnexpectedBehavior(_) => AppError::Connection(
            "The SFTP session ended unexpectedly. Reconnect to continue.".into(),
        ),
    }
}
