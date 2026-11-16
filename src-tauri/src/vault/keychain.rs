//! "Remember on this device": the vault key in the OS credential store.
//!
//! Windows: Credential Manager (protected by DPAPI under the user's logon).
//! macOS: Keychain. Linux: kernel keyutils (cleared at logout; Secret Service
//! support can be added later).
//!
//! Behind a trait so tests don't touch the real OS store.

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};

const SERVICE: &str = "com.nexterm.app";

pub trait DeviceKeyStore: Send + Sync {
    fn get(&self, vault_id: &str) -> AppResult<Option<Zeroizing<Vec<u8>>>>;
    fn set(&self, vault_id: &str, key: &[u8]) -> AppResult<()>;
    fn delete(&self, vault_id: &str) -> AppResult<()>;
}

fn account(vault_id: &str) -> String {
    format!("vault-key-{vault_id}")
}

/// The real OS credential store.
pub struct OsKeyStore;

impl OsKeyStore {
    fn entry(vault_id: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(SERVICE, &account(vault_id))
            .map_err(|e| AppError::Internal(format!("keychain unavailable: {e}")))
    }
}

impl DeviceKeyStore for OsKeyStore {
    fn get(&self, vault_id: &str) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
        match Self::entry(vault_id)?.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => {
                // Not fatal: the user just types the master password.
                tracing::warn!(error = %e, "could not read the remembered vault key");
                Ok(None)
            }
        }
    }

    fn set(&self, vault_id: &str, key: &[u8]) -> AppResult<()> {
        Self::entry(vault_id)?.set_secret(key).map_err(|e| {
            AppError::Internal(format!("could not store the key in the keychain: {e}"))
        })
    }

    fn delete(&self, vault_id: &str) -> AppResult<()> {
        match Self::entry(vault_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::Internal(format!(
                "could not remove the key from the keychain: {e}"
            ))),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryKeyStore {
    map: Mutex<HashMap<String, Vec<u8>>>,
}

impl DeviceKeyStore for MemoryKeyStore {
    fn get(&self, vault_id: &str) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
        let map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(map.get(&account(vault_id)).cloned().map(Zeroizing::new))
    }

    fn set(&self, vault_id: &str, key: &[u8]) -> AppResult<()> {
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.insert(account(vault_id), key.to_vec());
        Ok(())
    }

    fn delete(&self, vault_id: &str) -> AppResult<()> {
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.remove(&account(vault_id));
        Ok(())
    }
}
