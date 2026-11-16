//! Credential vault.
//!
//! Key hierarchy (SECURITY.md §5):
//!
//! ```text
//! master password --Argon2id(salt)--> KEK --unwraps--> vault key --seals--> each secret
//! ```
//!
//! - The vault key is random (256-bit). Changing the master password only
//!   re-wraps it; secrets are not re-encrypted.
//! - "Remember on this device" stores the vault key in the OS credential store
//!   (Windows: Credential Manager / DPAPI) so the app unlocks at start-up.
//!   Manual lock and auto-lock still require the master password to unlock.
//! - While locked, no key material is in memory.

pub mod credentials;
pub mod crypto;
pub mod keychain;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension, params};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use uuid::Uuid;
use zeroize::Zeroizing;

use self::crypto::{KEY_LEN, KdfParams, OpenError, SALT_LEN, SecretKey};
use self::keychain::DeviceKeyStore;
use crate::error::{AppError, AppResult};
use crate::storage::{Database, now_ms};

const WRAP_AAD: &[u8] = b"nexterm:vault-key:v1";
const CHECK_AAD: &[u8] = b"nexterm:key-check:v1";
const CHECK_PLAINTEXT: &[u8] = b"nexterm-vault-ok";
pub const MIN_MASTER_PASSWORD_LEN: usize = 8;
pub const MAX_MASTER_PASSWORD_LEN: usize = 1024;
pub const MAX_AUTO_LOCK_MINUTES: u32 = 24 * 60;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub initialized: bool,
    pub unlocked: bool,
    /// The vault key is stored in the OS credential store.
    pub remembered: bool,
    /// 0 = never.
    pub auto_lock_minutes: u32,
}

struct Meta {
    vault_id: String,
    kdf: KdfParams,
    salt: Vec<u8>,
    wrapped_key: Vec<u8>,
    key_check: Vec<u8>,
    auto_lock_minutes: u32,
}

struct Inner {
    key: Option<SecretKey>,
    last_activity: Instant,
}

pub struct Vault {
    db: Database,
    keystore: Arc<dyn DeviceKeyStore>,
    kdf: KdfParams,
    inner: Mutex<Inner>,
}

fn read_meta(conn: &Connection) -> AppResult<Option<Meta>> {
    Ok(conn
        .query_row(
            "SELECT vault_id, kdf_m_kib, kdf_t, kdf_p, salt, wrapped_key, key_check, auto_lock_minutes
             FROM vault_meta WHERE id = 1",
            [],
            |r| {
                Ok(Meta {
                    vault_id: r.get(0)?,
                    kdf: KdfParams {
                        m_kib: r.get(1)?,
                        t: r.get(2)?,
                        p: r.get(3)?,
                    },
                    salt: r.get(4)?,
                    wrapped_key: r.get(5)?,
                    key_check: r.get(6)?,
                    auto_lock_minutes: r.get(7)?,
                })
            },
        )
        .optional()?)
}

fn check_master_password(password: &SecretString) -> AppResult<()> {
    let len = password.expose_secret().chars().count();
    if len < MIN_MASTER_PASSWORD_LEN {
        return Err(AppError::Validation(format!(
            "The master password must be at least {MIN_MASTER_PASSWORD_LEN} characters."
        )));
    }
    if len > MAX_MASTER_PASSWORD_LEN {
        return Err(AppError::Validation(
            "The master password is too long.".into(),
        ));
    }
    Ok(())
}

fn to_key(bytes: &[u8]) -> AppResult<SecretKey> {
    let arr: [u8; KEY_LEN] = bytes
        .try_into()
        .map_err(|_| AppError::Internal("vault key has the wrong length".into()))?;
    Ok(Zeroizing::new(arr))
}

/// True if `key` is this vault's key (decrypts the check value).
fn key_matches(meta: &Meta, key: &[u8; KEY_LEN]) -> bool {
    crypto::open(key, CHECK_AAD, &meta.key_check)
        .map(|p| p.as_slice() == CHECK_PLAINTEXT)
        .unwrap_or(false)
}

/// Unwrap the vault key with the master password.
fn unwrap_with_password(meta: &Meta, password: &SecretString) -> AppResult<SecretKey> {
    meta.kdf.check_minimum()?;
    let kek = crypto::derive_kek(password.expose_secret().as_bytes(), &meta.salt, meta.kdf)?;
    match crypto::open(&kek, WRAP_AAD, &meta.wrapped_key) {
        Ok(bytes) => to_key(&bytes),
        Err(OpenError::Authentication) => Err(AppError::WrongMasterPassword),
        Err(OpenError::Malformed) => Err(AppError::Internal("vault header is corrupted".into())),
    }
}

impl Vault {
    pub fn new(db: Database, keystore: Arc<dyn DeviceKeyStore>) -> Self {
        Self::with_kdf(db, keystore, KdfParams::DEFAULT)
    }

    /// Custom KDF cost (tests only use cheaper parameters via `new_for_tests`).
    fn with_kdf(db: Database, keystore: Arc<dyn DeviceKeyStore>, kdf: KdfParams) -> Self {
        Self {
            db,
            keystore,
            kdf,
            inner: Mutex::new(Inner {
                key: None,
                last_activity: Instant::now(),
            }),
        }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    pub fn status(&self) -> AppResult<VaultStatus> {
        let meta = self.db.with(|c| read_meta(c))?;
        let unlocked = self.inner().key.is_some();
        Ok(match meta {
            None => VaultStatus {
                initialized: false,
                unlocked: false,
                remembered: false,
                auto_lock_minutes: 15,
            },
            Some(m) => VaultStatus {
                initialized: true,
                unlocked,
                remembered: self.keystore.get(&m.vault_id)?.is_some(),
                auto_lock_minutes: m.auto_lock_minutes,
            },
        })
    }

    /// Create the vault. Slow (runs the KDF): call from a blocking thread.
    pub fn setup(&self, master: &SecretString, remember: bool) -> AppResult<()> {
        check_master_password(master)?;
        if self.db.with(|c| read_meta(c))?.is_some() {
            return Err(AppError::Validation("The vault is already set up.".into()));
        }
        let vault_id = Uuid::new_v4().to_string();
        let salt = crypto::random_bytes::<SALT_LEN>()?;
        let vault_key = crypto::random_bytes::<KEY_LEN>()?;
        let kek = crypto::derive_kek(master.expose_secret().as_bytes(), salt.as_ref(), self.kdf)?;
        let wrapped = crypto::seal(&kek, WRAP_AAD, vault_key.as_ref())?;
        let check = crypto::seal(&vault_key, CHECK_AAD, CHECK_PLAINTEXT)?;
        let now = now_ms();
        self.db.with(|c| {
            c.execute(
                "INSERT INTO vault_meta (id, vault_id, kdf_m_kib, kdf_t, kdf_p, salt, wrapped_key,
                                         key_check, created_at, updated_at)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    vault_id,
                    self.kdf.m_kib,
                    self.kdf.t,
                    self.kdf.p,
                    salt.as_slice(),
                    wrapped,
                    check,
                    now
                ],
            )?;
            Ok(())
        })?;
        if remember {
            self.keystore.set(&vault_id, vault_key.as_ref())?;
        }
        let mut inner = self.inner();
        inner.key = Some(vault_key);
        inner.last_activity = Instant::now();
        tracing::info!(remember, "vault created");
        Ok(())
    }

    /// Unlock with the master password. Slow (runs the KDF).
    pub fn unlock(&self, master: &SecretString, remember: bool) -> AppResult<()> {
        let meta = self
            .db
            .with(|c| read_meta(c))?
            .ok_or_else(|| AppError::Validation("The vault is not set up yet.".into()))?;
        let key = match unwrap_with_password(&meta, master) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("vault unlock failed (wrong master password)");
                return Err(e);
            }
        };
        if remember {
            self.keystore.set(&meta.vault_id, key.as_ref())?;
        }
        let mut inner = self.inner();
        inner.key = Some(key);
        inner.last_activity = Instant::now();
        tracing::info!(remember, "vault unlocked");
        Ok(())
    }

    /// At start-up: unlock silently if the key is remembered on this device.
    pub fn unlock_from_device(&self) -> AppResult<bool> {
        let Some(meta) = self.db.with(|c| read_meta(c))? else {
            return Ok(false);
        };
        let Some(stored) = self.keystore.get(&meta.vault_id)? else {
            return Ok(false);
        };
        let key = to_key(&stored)?;
        if !key_matches(&meta, &key) {
            // Stale entry (e.g. vault was reset): remove it, ask for the password.
            tracing::warn!("remembered vault key does not match; removing it");
            self.keystore.delete(&meta.vault_id)?;
            return Ok(false);
        }
        let mut inner = self.inner();
        inner.key = Some(key);
        inner.last_activity = Instant::now();
        tracing::info!("vault unlocked from device keychain");
        Ok(true)
    }

    /// Drop the key from memory.
    pub fn lock(&self) {
        let was_unlocked = self.inner().key.take().is_some();
        if was_unlocked {
            tracing::info!("vault locked");
        }
    }

    /// Remove the remembered key from the OS credential store.
    pub fn forget_device(&self) -> AppResult<()> {
        if let Some(meta) = self.db.with(|c| read_meta(c))? {
            self.keystore.delete(&meta.vault_id)?;
            tracing::info!("vault key removed from device keychain");
        }
        Ok(())
    }

    /// Re-wrap the vault key under a new master password. Slow (KDF twice).
    pub fn change_master_password(&self, old: &SecretString, new: &SecretString) -> AppResult<()> {
        check_master_password(new)?;
        let meta = self
            .db
            .with(|c| read_meta(c))?
            .ok_or_else(|| AppError::Validation("The vault is not set up yet.".into()))?;
        let key = unwrap_with_password(&meta, old)?;
        let salt = crypto::random_bytes::<SALT_LEN>()?;
        let kek = crypto::derive_kek(new.expose_secret().as_bytes(), salt.as_ref(), self.kdf)?;
        let wrapped = crypto::seal(&kek, WRAP_AAD, key.as_ref())?;
        self.db.with(|c| {
            c.execute(
                "UPDATE vault_meta SET salt = ?1, wrapped_key = ?2, kdf_m_kib = ?3, kdf_t = ?4,
                        kdf_p = ?5, updated_at = ?6 WHERE id = 1",
                params![
                    salt.as_slice(),
                    wrapped,
                    self.kdf.m_kib,
                    self.kdf.t,
                    self.kdf.p,
                    now_ms()
                ],
            )?;
            Ok(())
        })?;
        tracing::info!("vault master password changed");
        Ok(())
    }

    pub fn set_auto_lock(&self, minutes: u32) -> AppResult<()> {
        if minutes > MAX_AUTO_LOCK_MINUTES {
            return Err(AppError::Validation(format!(
                "Auto-lock can be at most {MAX_AUTO_LOCK_MINUTES} minutes."
            )));
        }
        self.db.with(|c| {
            let changed = c.execute(
                "UPDATE vault_meta SET auto_lock_minutes = ?1, updated_at = ?2 WHERE id = 1",
                params![minutes, now_ms()],
            )?;
            if changed == 0 {
                return Err(AppError::Validation("The vault is not set up yet.".into()));
            }
            Ok(())
        })
    }

    /// Record user activity (resets the auto-lock timer).
    pub fn touch(&self) {
        self.inner().last_activity = Instant::now();
    }

    /// Lock if the vault has been idle longer than the configured time.
    /// Returns true if it locked now. Called periodically by a background task.
    pub fn lock_if_idle(&self) -> AppResult<bool> {
        let minutes = match self.db.with(|c| read_meta(c))? {
            Some(m) => m.auto_lock_minutes,
            None => return Ok(false),
        };
        if minutes == 0 {
            return Ok(false);
        }
        let mut inner = self.inner();
        let idle = inner.last_activity.elapsed() >= Duration::from_secs(u64::from(minutes) * 60);
        if inner.key.is_some() && idle {
            inner.key = None;
            tracing::info!(minutes, "vault auto-locked after inactivity");
            return Ok(true);
        }
        Ok(false)
    }

    /// Run `f` with the vault key. Fails with `VaultLocked` if locked.
    pub(crate) fn with_key<T>(
        &self,
        f: impl FnOnce(&[u8; KEY_LEN]) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut inner = self.inner();
        inner.last_activity = Instant::now();
        let key = inner.key.as_ref().ok_or(AppError::VaultLocked)?;
        f(key)
    }

    #[cfg(test)]
    pub fn new_for_tests(db: Database, keystore: Arc<dyn DeviceKeyStore>) -> Self {
        Self::with_kdf(
            db,
            keystore,
            KdfParams {
                m_kib: 64 * 1024,
                t: 3,
                p: 1,
            },
        )
    }

    #[cfg(test)]
    fn force_idle(&self, ago: Duration) {
        if let Some(t) = Instant::now().checked_sub(ago) {
            self.inner().last_activity = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::keychain::MemoryKeyStore;
    use super::*;

    fn vault() -> (Vault, Arc<MemoryKeyStore>) {
        let store = Arc::new(MemoryKeyStore::default());
        let v = Vault::new_for_tests(Database::open_in_memory().unwrap(), store.clone());
        (v, store)
    }

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    #[test]
    fn setup_lock_unlock_cycle() {
        let (v, _) = vault();
        assert!(!v.status().unwrap().initialized);
        assert_eq!(v.with_key(|_| Ok(())).unwrap_err().code(), "VAULT_LOCKED");

        v.setup(&pw("correct horse battery"), false).unwrap();
        let s = v.status().unwrap();
        assert!(s.initialized && s.unlocked && !s.remembered);
        v.with_key(|_| Ok(())).unwrap();

        v.lock();
        assert!(!v.status().unwrap().unlocked);
        assert_eq!(
            v.unlock(&pw("wrong password!"), false).unwrap_err().code(),
            "WRONG_MASTER_PASSWORD"
        );
        v.unlock(&pw("correct horse battery"), false).unwrap();
        assert!(v.status().unwrap().unlocked);
    }

    #[test]
    fn setup_rules() {
        let (v, _) = vault();
        assert_eq!(
            v.setup(&pw("short"), false).unwrap_err().code(),
            "VALIDATION"
        );
        v.setup(&pw("long enough pw"), false).unwrap();
        assert_eq!(
            v.setup(&pw("another one!!"), false).unwrap_err().code(),
            "VALIDATION",
            "setup twice must fail"
        );
    }

    #[test]
    fn remember_on_device() {
        let (v, store) = vault();
        v.setup(&pw("correct horse battery"), true).unwrap();
        assert!(v.status().unwrap().remembered);
        v.lock();
        assert!(
            v.unlock_from_device().unwrap(),
            "remembered key unlocks at start-up"
        );
        v.forget_device().unwrap();
        v.lock();
        assert!(!v.unlock_from_device().unwrap());
        assert!(!v.status().unwrap().remembered);

        // A stale/foreign key in the keychain is rejected and removed.
        let meta = v.db.with(|c| read_meta(c)).unwrap().unwrap();
        store.set(&meta.vault_id, &[9u8; KEY_LEN]).unwrap();
        assert!(!v.unlock_from_device().unwrap());
        assert!(store.get(&meta.vault_id).unwrap().is_none());
    }

    #[test]
    fn change_master_password_keeps_key() {
        let (v, _) = vault();
        v.setup(&pw("old password 1"), false).unwrap();
        let before = v.with_key(|k| Ok(*k)).unwrap();
        assert_eq!(
            v.change_master_password(&pw("not the old one"), &pw("new password 2"))
                .unwrap_err()
                .code(),
            "WRONG_MASTER_PASSWORD"
        );
        v.change_master_password(&pw("old password 1"), &pw("new password 2"))
            .unwrap();
        v.lock();
        assert_eq!(
            v.unlock(&pw("old password 1"), false).unwrap_err().code(),
            "WRONG_MASTER_PASSWORD"
        );
        v.unlock(&pw("new password 2"), false).unwrap();
        assert_eq!(
            v.with_key(|k| Ok(*k)).unwrap(),
            before,
            "secrets stay readable"
        );
    }

    #[test]
    fn auto_lock_after_idle() {
        let (v, _) = vault();
        v.setup(&pw("correct horse battery"), false).unwrap();
        v.set_auto_lock(1).unwrap();
        assert!(!v.lock_if_idle().unwrap());
        v.force_idle(Duration::from_secs(61));
        assert!(v.lock_if_idle().unwrap());
        assert!(!v.status().unwrap().unlocked);

        v.unlock(&pw("correct horse battery"), false).unwrap();
        v.set_auto_lock(0).unwrap();
        v.force_idle(Duration::from_secs(10_000));
        assert!(!v.lock_if_idle().unwrap(), "0 = never");
        assert!(v.set_auto_lock(MAX_AUTO_LOCK_MINUTES + 1).is_err());
    }

    #[test]
    fn tampered_kdf_params_are_refused() {
        let (v, _) = vault();
        v.setup(&pw("correct horse battery"), false).unwrap();
        v.lock();
        v.db.with(|c| Ok(c.execute("UPDATE vault_meta SET kdf_t = 1", [])?))
            .unwrap();
        assert_eq!(
            v.unlock(&pw("correct horse battery"), false)
                .unwrap_err()
                .code(),
            "INTERNAL"
        );
    }
}
