//! Open SFTP sessions, one per remote file-manager pane, by opaque id.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use super::{FtpFs, RemoteFs};

/// An open remote file system: SFTP or FTP(S).
#[derive(Clone)]
pub enum RemoteHandle {
    Sftp(Arc<RemoteFs>),
    Ftp(Arc<FtpFs>),
}

impl RemoteHandle {
    fn is_closed(&self) -> bool {
        match self {
            Self::Sftp(s) => s.is_closed(),
            // FTP connections reconnect on demand.
            Self::Ftp(_) => false,
        }
    }

    async fn close(&self) {
        match self {
            Self::Sftp(s) => s.close().await,
            Self::Ftp(f) => f.close().await,
        }
    }

    fn only_user(&self) -> bool {
        match self {
            Self::Sftp(s) => Arc::strong_count(s) == 1,
            Self::Ftp(f) => Arc::strong_count(f) == 1,
        }
    }
}
use crate::error::{AppError, AppResult};

/// Upper bound on simultaneously open SFTP sessions.
pub const MAX_FILE_SESSIONS: usize = 64;

#[derive(Default)]
pub struct FileSessions {
    map: Mutex<HashMap<Uuid, RemoteHandle>>,
}

impl FileSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, fs: RemoteHandle) -> AppResult<Uuid> {
        let mut map = self.map.lock().await;
        if map.len() >= MAX_FILE_SESSIONS {
            fs.close().await;
            return Err(AppError::Validation(format!(
                "Too many open file sessions (max {MAX_FILE_SESSIONS}). Close some panes first."
            )));
        }
        let id = Uuid::new_v4();
        map.insert(id, fs);
        Ok(id)
    }

    /// The session, or `NOT_FOUND`/`CONNECTION` if it is gone or dropped.
    pub async fn get(&self, id: Uuid) -> AppResult<RemoteHandle> {
        let fs = self
            .map
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("File session".into()))?;
        if fs.is_closed() {
            return Err(AppError::Connection(
                "The connection to the server was lost. Reconnect the pane.".into(),
            ));
        }
        Ok(fs)
    }

    /// Remove and close. Transfers still holding the session finish or fail
    /// on their own; the connection closes when the last user drops it.
    pub async fn remove(&self, id: Uuid) -> bool {
        let fs = self.map.lock().await.remove(&id);
        match fs {
            Some(fs) => {
                if fs.only_user() {
                    fs.close().await;
                }
                true
            }
            None => false,
        }
    }
}
