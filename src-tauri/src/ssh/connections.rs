//! What each open tab needs to reconnect / duplicate.
//!
//! Each tab owns an opaque `connection_id`. Behind it is a [`ConnectSpec`]:
//! - `Direct`: a quick connect. The typed password lives here, in Rust memory
//!   only, until the tab closes (`ssh_forget`) or the app exits.
//! - `Saved`: a saved session. Only the session id (plus anything the user had
//!   to type) is kept; host, credentials and auth method are re-read from the
//!   database and the vault on every (re)connect, so a locked vault or an
//!   edited session takes effect immediately. Telnet, serial and local
//!   sessions go through the same path (Phase 8).
//! - `Local`: an unsaved local terminal (just the shell choice).

use std::collections::HashMap;

use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::SshAuth;
use crate::error::{AppError, AppResult};
use crate::session::options::LocalOptions;

/// Upper bound on remembered connections (one per open tab).
pub const MAX_CONNECTIONS: usize = 256;

pub enum ConnectSpec {
    Direct {
        host: String,
        port: u16,
        username: String,
        auth: SshAuth,
    },
    Saved {
        session_id: String,
        /// Typed at connect time because the session had none.
        username: Option<String>,
        /// Typed at connect time because no credential was linked.
        password: Option<SecretString>,
    },
    /// A local terminal that isn't a saved session.
    Local { options: LocalOptions },
}

impl ConnectSpec {
    /// Explicit deep copy; every copy of a secret is visible in code review.
    pub fn duplicate(&self) -> Self {
        match self {
            Self::Direct {
                host,
                port,
                username,
                auth,
            } => Self::Direct {
                host: host.clone(),
                port: *port,
                username: username.clone(),
                auth: auth.duplicate(),
            },
            Self::Saved {
                session_id,
                username,
                password,
            } => Self::Saved {
                session_id: session_id.clone(),
                username: username.clone(),
                password: password
                    .as_ref()
                    .map(|p| SecretString::from(p.expose_secret().to_owned())),
            },
            Self::Local { options } => Self::Local {
                options: options.clone(),
            },
        }
    }
}

#[derive(Default)]
pub struct SshConnections {
    map: Mutex<HashMap<Uuid, ConnectSpec>>,
}

impl SshConnections {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, spec: ConnectSpec) -> AppResult<Uuid> {
        let mut map = self.map.lock().await;
        if map.len() >= MAX_CONNECTIONS {
            return Err(AppError::Validation(format!(
                "Too many open tabs (max {MAX_CONNECTIONS}). Close some first."
            )));
        }
        let id = Uuid::new_v4();
        map.insert(id, spec);
        Ok(id)
    }

    pub async fn get_copy(&self, id: Uuid) -> AppResult<ConnectSpec> {
        self.map
            .lock()
            .await
            .get(&id)
            .map(ConnectSpec::duplicate)
            .ok_or_else(|| AppError::NotFound("Connection".into()))
    }

    /// Drop the spec (typed passwords are zeroized on drop).
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

    fn direct(pw: &str) -> ConnectSpec {
        ConnectSpec::Direct {
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth: SshAuth::Password(SecretString::from(pw.to_string())),
        }
    }

    #[tokio::test]
    async fn insert_copy_remove() {
        let store = SshConnections::new();
        let id = store.insert(direct("pw")).await.unwrap();
        match store.get_copy(id).await.unwrap() {
            ConnectSpec::Direct {
                auth: SshAuth::Password(p),
                ..
            } => assert_eq!(p.expose_secret(), "pw"),
            _ => panic!("wrong variant"),
        }
        assert!(store.remove(id).await);
        assert!(!store.remove(id).await);
        assert_eq!(store.get_copy(id).await.err().unwrap().code(), "NOT_FOUND");
    }

    #[tokio::test]
    async fn enforces_capacity() {
        let store = SshConnections::new();
        for _ in 0..MAX_CONNECTIONS {
            store.insert(direct("p")).await.unwrap();
        }
        assert_eq!(
            store.insert(direct("p")).await.err().unwrap().code(),
            "VALIDATION"
        );
    }
}
