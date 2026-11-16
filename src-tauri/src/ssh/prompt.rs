//! Questions Rust asks the user in the middle of a connection: "trust this
//! host key?" and keyboard-interactive (2FA / OTP) prompts.
//!
//! Rust sends an [`SshPrompt`] over the connection's prompt channel and waits
//! (with a timeout) for the UI to call `ssh_prompt_respond` with the same id.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// How long the user has to answer before the connection attempt is abandoned.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptField {
    pub text: String,
    /// false = hide input (password / OTP).
    pub echo: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SshPrompt {
    #[serde(rename_all = "camelCase")]
    HostKey {
        prompt_id: String,
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        /// Key types previously trusted for this host (a type switch is unusual).
        other_known_types: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    KeyboardInteractive {
        prompt_id: String,
        host: String,
        name: String,
        instructions: String,
        fields: Vec<PromptField>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostKeyDecision {
    AcceptOnce,
    AcceptAndSave,
    Reject,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PromptResponse {
    HostKey {
        decision: HostKeyDecision,
    },
    /// Answers to keyboard-interactive fields, in order.
    Answers {
        answers: Vec<SecretString>,
    },
    Cancel,
}

/// Where prompts are delivered (the Tauri channel, or a test double).
pub trait PromptSink: Send + Sync {
    /// Returns false if the receiver is gone.
    fn send(&self, prompt: SshPrompt) -> bool;
}

/// Pending questions waiting for the UI.
#[derive(Default)]
pub struct PromptBroker {
    pending: Mutex<HashMap<Uuid, oneshot::Sender<PromptResponse>>>,
}

/// Upper bound on simultaneously pending prompts (a buggy UI can't grow it).
const MAX_PENDING: usize = 64;

impl PromptBroker {
    pub fn new() -> Self {
        Self::default()
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, oneshot::Sender<PromptResponse>>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Send a prompt and wait for the answer.
    pub async fn ask(
        &self,
        sink: &dyn PromptSink,
        build: impl FnOnce(String) -> SshPrompt,
        timeout: Duration,
    ) -> AppResult<PromptResponse> {
        let id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        {
            let mut table = self.table();
            if table.len() >= MAX_PENDING {
                return Err(AppError::Validation("Too many open prompts.".into()));
            }
            table.insert(id, tx);
        }
        if !sink.send(build(id.to_string())) {
            self.table().remove(&id);
            return Err(AppError::Cancelled);
        }
        let result = tokio::time::timeout(timeout, rx).await;
        self.table().remove(&id);
        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(AppError::Cancelled),
            Err(_) => {
                tracing::info!("prompt timed out");
                Err(AppError::Cancelled)
            }
        }
    }

    /// Deliver the UI's answer. Unknown or expired ids are rejected.
    pub fn respond(&self, prompt_id: &str, response: PromptResponse) -> AppResult<()> {
        let id = Uuid::parse_str(prompt_id)
            .map_err(|_| AppError::Validation("Invalid prompt id.".into()))?;
        let sender = self
            .table()
            .remove(&id)
            .ok_or_else(|| AppError::NotFound("Prompt".into()))?;
        // The waiting side may have timed out in the meantime; that's fine.
        let _ = sender.send(response);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// Captures prompts; a test task answers them through the broker.
    struct Capture(tokio::sync::mpsc::UnboundedSender<SshPrompt>);
    impl PromptSink for Capture {
        fn send(&self, prompt: SshPrompt) -> bool {
            self.0.send(prompt).is_ok()
        }
    }

    fn host_prompt(id: String) -> SshPrompt {
        SshPrompt::HostKey {
            prompt_id: id,
            host: "h".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:x".into(),
            other_known_types: vec![],
        }
    }

    #[tokio::test]
    async fn ask_and_respond() {
        let broker = Arc::new(PromptBroker::new());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let b = broker.clone();
        tokio::spawn(async move {
            if let Some(SshPrompt::HostKey { prompt_id, .. }) = rx.recv().await {
                b.respond(
                    &prompt_id,
                    PromptResponse::HostKey {
                        decision: HostKeyDecision::AcceptAndSave,
                    },
                )
                .unwrap();
            }
        });
        let resp = broker
            .ask(&Capture(tx), host_prompt, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(matches!(
            resp,
            PromptResponse::HostKey {
                decision: HostKeyDecision::AcceptAndSave
            }
        ));
        assert!(broker.table().is_empty());
    }

    #[tokio::test]
    async fn timeout_cancels_and_cleans_up() {
        let broker = PromptBroker::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let err = broker
            .ask(&Capture(tx), host_prompt, Duration::from_millis(50))
            .await
            .err()
            .unwrap();
        assert_eq!(err.code(), "CANCELLED");
        assert!(broker.table().is_empty());
    }

    #[test]
    fn respond_rejects_unknown_ids() {
        let broker = PromptBroker::new();
        assert_eq!(
            broker
                .respond(&Uuid::new_v4().to_string(), PromptResponse::Cancel)
                .unwrap_err()
                .code(),
            "NOT_FOUND"
        );
        assert_eq!(
            broker
                .respond("nope", PromptResponse::Cancel)
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
    }
}
