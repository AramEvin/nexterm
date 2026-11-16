//! Open SFTP sessions, one per remote file-manager pane, by opaque id.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use super::RemoteFs;
use crate::error::{AppError, AppResult};

/// Upper bound on simultaneously open SFTP sessions.
pub const MAX_FILE_SESSIONS: usize = 64;

#[derive(Default)]
pub struct FileSessions {
    map: Mutex<HashMap<Uuid, Arc<RemoteFs>>>,
}

impl FileSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, fs: RemoteFs) -> AppResult<Uuid> {
        let mut map = self.map.lock().await;
        if map.len() >= MAX_FILE_SESSIONS {
            fs.close().await;
            return Err(AppError::Validation(format!(
                "Too many open file sessions (max {MAX_FILE_SESSIONS}). Close some panes first."
            )));
        }
        let id = Uuid::new_v4();
        map.insert(id, Arc::new(fs));
        Ok(id)
    }

    /// The session, or `NOT_FOUND`/`CONNECTION` if it is gone or dropped.
    pub async fn get(&self, id: Uuid) -> AppResult<Arc<RemoteFs>> {
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
                if Arc::strong_count(&fs) == 1 {
                    fs.close().await;
                }
                true
            }
            None => false,
        }
    }
}
