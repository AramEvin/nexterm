//! In-memory store of connection parameters for open tabs.
//!
//! Reconnect and Duplicate need the credentials again, but the frontend never
//! keeps the password. Instead, each tab owns an opaque `connection_id`; the
//! parameters (password as `SecretString`) live here, in Rust memory only,
//! until the tab is closed (`ssh_forget`) or the app exits. Nothing is written
//! to disk. Phase 4 replaces the stored password with a vault `credential_id`.

use std::collections::HashMap;

use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::SshConnectParams;
use crate::error::{AppError, AppResult};

/// Upper bound on remembered connections (one per open tab). Protects against
/// a buggy or hostile frontend growing this map without limit.
pub const MAX_CONNECTIONS: usize = 256;

#[derive(Default)]
pub struct SshConnections {
    map: Mutex<HashMap<Uuid, SshConnectParams>>,
}

impl SshConnectParams {
    /// Explicit deep copy. `SecretString` is deliberately not `Clone`, so
    /// every copy of a password is visible in the code.
    pub fn duplicate(&self) -> Self {
        Self {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            password: SecretString::from(self.password.expose_secret().to_owned()),
        }
    }
}

impl SshConnections {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember `params` under a new random id.
    pub async fn insert(&self, params: SshConnectParams) -> AppResult<Uuid> {
        let mut map = self.map.lock().await;
        if map.len() >= MAX_CONNECTIONS {
            return Err(AppError::Validation(format!(
                "Too many open tabs (max {MAX_CONNECTIONS}). Close some first."
            )));
        }
        let id = Uuid::new_v4();
        map.insert(id, params);
        Ok(id)
    }

    /// A copy of the stored parameters, for opening another connection.
    pub async fn get_copy(&self, id: Uuid) -> AppResult<SshConnectParams> {
        self.map
            .lock()
            .await
            .get(&id)
            .map(SshConnectParams::duplicate)
            .ok_or_else(|| AppError::NotFound("Connection".into()))
    }

    /// Drop the stored parameters (the password is zeroized on drop).
    /// Returns whether an entry existed.
    pub async fn remove(&self, id: Uuid) -> bool {
        self.map.lock().await.remove(&id).is_some()
    }

    pub async fn len(&self) -> usize {
        self.map.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(host: &str) -> SshConnectParams {
        SshConnectParams {
            host: host.into(),
            port: 22,
            username: "u".into(),
            password: SecretString::from("pw"),
        }
    }

    #[tokio::test]
    async fn insert_copy_remove() {
        let store = SshConnections::new();
        let id = store.insert(params("a")).await.unwrap();
        let copy = store.get_copy(id).await.unwrap();
        assert_eq!(copy.host, "a");
        assert_eq!(copy.password.expose_secret(), "pw");
        assert!(store.remove(id).await);
        assert!(!store.remove(id).await);
        assert_eq!(store.get_copy(id).await.unwrap_err().code(), "NOT_FOUND");
    }

    #[test]
    fn debug_redacts_password() {
        let p = SshConnectParams {
            password: SecretString::from("hunter2-secret"),
            ..params("h")
        };
        let dbg = format!("{:?}", p.duplicate());
        assert!(!dbg.contains("hunter2"), "password leaked: {dbg}");
    }

    #[tokio::test]
    async fn enforces_capacity() {
        let store = SshConnections::new();
        for _ in 0..MAX_CONNECTIONS {
            store.insert(params("h")).await.unwrap();
        }
        assert_eq!(
            store.insert(params("h")).await.unwrap_err().code(),
            "VALIDATION"
        );
    }
}
