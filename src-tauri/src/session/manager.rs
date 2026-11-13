//! `SessionManager`: registry of live sessions, keyed by a random UUID.
//!
//! Each session runs in its own tokio task. A watcher task awaits it, removes
//! the entry when it ends and turns a panic into a "session crashed" status,
//! so one broken session can never take down the others or the app.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use super::protocol::{OutputSink, SessionCommand, SessionHandle, SessionStatus};
use crate::error::{AppError, AppResult};

#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<Uuid, mpsc::Sender<SessionCommand>>>,
}

impl SessionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a freshly opened session and start watching its task.
    pub async fn register(
        self: &Arc<Self>,
        handle: SessionHandle,
        sink: Arc<dyn OutputSink>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.sessions.lock().await.insert(id, handle.commands);

        let manager = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(err) = handle.task.await {
                tracing::error!(session_id = %id, error = %err, "session task crashed");
                sink.status(SessionStatus::Closed {
                    reason: "The session crashed unexpectedly. See the log for details.".into(),
                });
            }
            manager.sessions.lock().await.remove(&id);
            tracing::debug!(session_id = %id, "session removed from registry");
        });

        id
    }

    /// Send a command to a live session.
    pub async fn send(&self, id: Uuid, command: SessionCommand) -> AppResult<()> {
        let sender = self
            .sessions
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("Session".into()))?;
        sender
            .send(command)
            .await
            .map_err(|_| AppError::NotFound("Session".into()))
    }

    /// Number of live sessions (diagnostics and tests).
    pub async fn len(&self) -> usize {
        self.sessions.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::protocol::ProtocolKind;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingSink(StdMutex<Vec<SessionStatus>>);
    impl OutputSink for RecordingSink {
        fn data(&self, _bytes: Vec<u8>) -> bool {
            true
        }
        fn status(&self, status: SessionStatus) {
            self.0.lock().unwrap().push(status);
        }
    }

    #[tokio::test]
    async fn panicking_session_is_isolated_and_removed() {
        let manager = SessionManager::new();
        let sink = Arc::new(RecordingSink::default());

        let (tx, _rx) = mpsc::channel(1);
        let task = tokio::spawn(async { panic!("boom") });
        let id = manager
            .register(
                SessionHandle {
                    kind: ProtocolKind::Ssh,
                    commands: tx,
                    task,
                },
                sink.clone(),
            )
            .await;

        // Give the watcher a moment to observe the panic.
        for _ in 0..50 {
            if manager.is_empty().await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(manager.is_empty().await);
        assert!(matches!(
            manager.send(id, SessionCommand::Close).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            sink.0.lock().unwrap().first(),
            Some(SessionStatus::Closed { .. })
        ));
    }

    #[tokio::test]
    async fn send_to_unknown_session_is_not_found() {
        let manager = SessionManager::new();
        let err = manager
            .send(Uuid::new_v4(), SessionCommand::Close)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }
}
