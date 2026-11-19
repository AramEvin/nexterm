//! Saved credentials: passwords and private keys, encrypted with the vault key.
//!
//! The frontend only ever sees [`Credential`] (metadata). Secrets are
//! decrypted inside Rust at connect time via [`load_secret`] and never cross
//! the IPC boundary.

use rusqlite::{Connection, OptionalExtension, params};
use russh::keys::{HashAlg, PrivateKey, decode_secret_key};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{Vault, crypto};
use crate::error::{AppError, AppResult};
use crate::storage::now_ms;

const MAX_NAME_LEN: usize = 128;
const MAX_SECRET_LEN: usize = 4096;
/// Private key files larger than this are not SSH keys.
pub const MAX_KEY_FILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialKind {
    Password,
    Key,
}

/// Non-secret metadata, safe to show in the UI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub kind: CredentialKind,
    pub username: String,
    pub key_algorithm: Option<String>,
    pub key_fingerprint: Option<String>,
    /// Saved sessions and folders that reference this credential.
    pub used_by: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Decrypted secret material. Never `Serialize`d, never sent to the frontend.
pub struct CredentialSecret {
    pub password: Option<SecretString>,
    pub private_key: Option<SecretString>,
    pub passphrase: Option<SecretString>,
}

impl std::fmt::Debug for CredentialSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialSecret([REDACTED])")
    }
}

impl CredentialSecret {
    /// Parse the stored private key (with its passphrase, if any).
    pub fn decode_key(&self) -> AppResult<PrivateKey> {
        let key = self
            .private_key
            .as_ref()
            .ok_or_else(|| AppError::Validation("This credential has no private key.".into()))?;
        decode_key(key, self.passphrase.as_ref())
    }
}

/// JSON shape inside the ciphertext (borrowed to avoid extra plaintext copies).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretOut<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    private_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    passphrase: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretIn {
    password: Option<String>,
    private_key: Option<String>,
    passphrase: Option<String>,
}

fn aad(id: &str) -> Vec<u8> {
    format!("nexterm:credential:v1:{id}").into_bytes()
}

fn seal_secret(vault: &Vault, id: &str, secret: &CredentialSecret) -> AppResult<Vec<u8>> {
    let out = SecretOut {
        password: secret.password.as_ref().map(|s| s.expose_secret()),
        private_key: secret.private_key.as_ref().map(|s| s.expose_secret()),
        passphrase: secret.passphrase.as_ref().map(|s| s.expose_secret()),
    };
    let json = Zeroizing::new(
        serde_json::to_vec(&out).map_err(|e| AppError::Internal(format!("serialize: {e}")))?,
    );
    vault.with_key(|key| crypto::seal(key, &aad(id), &json))
}

fn open_secret(vault: &Vault, id: &str, blob: &[u8]) -> AppResult<CredentialSecret> {
    let json = vault.with_key(|key| {
        crypto::open(key, &aad(id), blob)
            .map_err(|_| AppError::Internal("credential could not be decrypted".into()))
    })?;
    let parsed: SecretIn = serde_json::from_slice(&json)
        .map_err(|_| AppError::Internal("credential payload is corrupted".into()))?;
    Ok(CredentialSecret {
        password: parsed.password.map(SecretString::from),
        private_key: parsed.private_key.map(SecretString::from),
        passphrase: parsed.passphrase.map(SecretString::from),
    })
}

fn clean_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::Validation("Name is required.".into()));
    }
    if name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "Name is too long or has invalid characters.".into(),
        ));
    }
    Ok(name.to_string())
}

fn clean_username(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    crate::ssh::validate::username(value)
}

fn check_secret_len(s: &SecretString, what: &str) -> AppResult<()> {
    if s.expose_secret().len() > MAX_SECRET_LEN {
        return Err(AppError::Validation(format!("{what} is too long.")));
    }
    Ok(())
}

/// Decode a private key (OpenSSH, PEM/PKCS#8, PuTTY .ppk), with passphrase.
pub fn decode_key(key: &SecretString, passphrase: Option<&SecretString>) -> AppResult<PrivateKey> {
    decode_secret_key(key.expose_secret(), passphrase.map(|p| p.expose_secret())).map_err(|e| {
        let msg = e.to_string().to_lowercase();
        if msg.contains("encrypt") || msg.contains("passphrase") || msg.contains("decrypt") {
            AppError::Validation(
                "This key is protected by a passphrase. Enter the correct passphrase.".into(),
            )
        } else {
            AppError::Validation(
                "Not a supported private key (OpenSSH, PEM or PuTTY .ppk expected).".into(),
            )
        }
    })
}

fn row_to_credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<Credential> {
    let kind: String = row.get("kind")?;
    Ok(Credential {
        id: row.get("id")?,
        name: row.get("name")?,
        kind: if kind == "key" {
            CredentialKind::Key
        } else {
            CredentialKind::Password
        },
        username: row.get("username")?,
        key_algorithm: row.get("key_algorithm")?,
        key_fingerprint: row.get("key_fingerprint")?,
        used_by: row.get("used_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

const SELECT: &str = "SELECT c.id, c.name, c.kind, c.username, c.key_algorithm, c.key_fingerprint,
        c.created_at, c.updated_at,
        (SELECT COUNT(*) FROM sessions s WHERE s.credential_id = c.id)
          + (SELECT COUNT(*) FROM folders f WHERE f.default_credential_id = c.id) AS used_by
     FROM credentials c";

/// Metadata of all credentials (works while the vault is locked).
pub fn list(conn: &Connection) -> AppResult<Vec<Credential>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY c.name COLLATE NOCASE"))?;
    let rows = stmt.query_map([], row_to_credential)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Credential> {
    let id = crate::storage::validate_id(id)?;
    conn.query_row(
        &format!("{SELECT} WHERE c.id = ?1"),
        [&id],
        row_to_credential,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound("Credential".into()))
}

pub fn create_password(
    conn: &Connection,
    vault: &Vault,
    name: &str,
    username: &str,
    password: SecretString,
) -> AppResult<Credential> {
    let name = clean_name(name)?;
    let username = clean_username(username)?;
    check_secret_len(&password, "Password")?;
    if password.expose_secret().is_empty() {
        return Err(AppError::Validation("Password is required.".into()));
    }
    let id = Uuid::new_v4().to_string();
    let blob = seal_secret(
        vault,
        &id,
        &CredentialSecret {
            password: Some(password),
            private_key: None,
            passphrase: None,
        },
    )?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO credentials (id, name, kind, username, secret, created_at, updated_at)
         VALUES (?1, ?2, 'password', ?3, ?4, ?5, ?5)",
        params![id, name, username, blob, now],
    )?;
    tracing::info!(credential = %id, kind = "password", "credential created");
    get(conn, &id)
}

/// Store a private key (already read by Rust from a user-picked file).
pub fn create_key(
    conn: &Connection,
    vault: &Vault,
    name: &str,
    username: &str,
    key_text: SecretString,
    passphrase: Option<SecretString>,
) -> AppResult<Credential> {
    let name = clean_name(name)?;
    let username = clean_username(username)?;
    if key_text.expose_secret().len() > MAX_KEY_FILE_BYTES {
        return Err(AppError::Validation("The key file is too large.".into()));
    }
    let passphrase = passphrase.filter(|p| !p.expose_secret().is_empty());
    let parsed = decode_key(&key_text, passphrase.as_ref())?;
    let public = parsed.public_key();
    let algorithm = public.algorithm().to_string();
    let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();

    let id = Uuid::new_v4().to_string();
    let blob = seal_secret(
        vault,
        &id,
        &CredentialSecret {
            password: None,
            private_key: Some(key_text),
            passphrase,
        },
    )?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO credentials (id, name, kind, username, key_algorithm, key_fingerprint,
                                  secret, created_at, updated_at)
         VALUES (?1, ?2, 'key', ?3, ?4, ?5, ?6, ?7, ?7)",
        params![id, name, username, algorithm, fingerprint, blob, now],
    )?;
    tracing::info!(credential = %id, kind = "key", %algorithm, %fingerprint, "credential created");
    get(conn, &id)
}

/// Rename / change username; optionally replace the password (password kind).
pub fn update(
    conn: &Connection,
    vault: &Vault,
    id: &str,
    name: &str,
    username: &str,
    new_password: Option<SecretString>,
) -> AppResult<Credential> {
    let existing = get(conn, id)?;
    let name = clean_name(name)?;
    let username = clean_username(username)?;
    let now = now_ms();
    match new_password.filter(|p| !p.expose_secret().is_empty()) {
        Some(password) => {
            if existing.kind != CredentialKind::Password {
                return Err(AppError::Validation(
                    "Only password credentials have a password.".into(),
                ));
            }
            check_secret_len(&password, "Password")?;
            let blob = seal_secret(
                vault,
                &existing.id,
                &CredentialSecret {
                    password: Some(password),
                    private_key: None,
                    passphrase: None,
                },
            )?;
            conn.execute(
                "UPDATE credentials SET name = ?2, username = ?3, secret = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![existing.id, name, username, blob, now],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE credentials SET name = ?2, username = ?3, updated_at = ?4 WHERE id = ?1",
                params![existing.id, name, username, now],
            )?;
        }
    }
    get(conn, &existing.id)
}

/// Delete a credential and detach it from sessions and folders.
pub fn delete(conn: &mut Connection, id: &str) -> AppResult<()> {
    let id = crate::storage::validate_id(id)?;
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE sessions SET credential_id = NULL WHERE credential_id = ?1",
        [&id],
    )?;
    // folders.default_credential_id has ON DELETE SET NULL.
    if tx.execute("DELETE FROM credentials WHERE id = ?1", [&id])? == 0 {
        return Err(AppError::NotFound("Credential".into()));
    }
    tx.commit()?;
    tracing::info!(credential = %id, "credential deleted");
    Ok(())
}

/// Decrypt a credential (Rust-internal; never exposed over IPC).
pub fn load_secret(
    conn: &Connection,
    vault: &Vault,
    id: &str,
) -> AppResult<(Credential, CredentialSecret)> {
    let meta = get(conn, id)?;
    let blob: Vec<u8> = conn.query_row(
        "SELECT secret FROM credentials WHERE id = ?1",
        [&meta.id],
        |r| r.get(0),
    )?;
    let secret = open_secret(vault, &meta.id, &blob)?;
    Ok((meta, secret))
}

/// The credential a session should use: its own, else the nearest folder default.
pub fn resolve_for_session(conn: &Connection, session_id: &str) -> AppResult<Option<String>> {
    let id = crate::storage::validate_id(session_id)?;
    let (own, mut folder): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT credential_id, folder_id FROM sessions WHERE id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound("Session".into()))?;
    if own.is_some() {
        return Ok(own);
    }
    // Walk up the folder chain (bounded; folders are at most 12 deep).
    for _ in 0..64 {
        let Some(fid) = folder else { return Ok(None) };
        let (default, parent): (Option<String>, Option<String>) = conn.query_row(
            "SELECT default_credential_id, parent_id FROM folders WHERE id = ?1",
            [&fid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if default.is_some() {
            return Ok(default);
        }
        folder = parent;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::storage::Database;
    use crate::vault::keychain::MemoryKeyStore;

    /// Unencrypted ed25519 test key (generated for tests only).
    pub const TEST_KEY: &str = include_str!("../../tests/fixtures/test_ed25519");
    /// Same key protected with passphrase "test-passphrase".
    pub const TEST_KEY_ENC: &str = include_str!("../../tests/fixtures/test_ed25519_enc");

    fn setup() -> (Vault, Database) {
        let db = Database::open_in_memory().unwrap();
        let vault = Vault::new_for_tests(db.clone(), Arc::new(MemoryKeyStore::default()));
        vault
            .setup(
                &SecretString::from("correct horse battery".to_string()),
                false,
            )
            .unwrap();
        (vault, db)
    }

    fn s(v: &str) -> SecretString {
        SecretString::from(v.to_string())
    }

    #[test]
    fn password_roundtrip_and_update() {
        let (vault, db) = setup();
        db.with(|c| {
            let cred = create_password(c, &vault, "prod root", "root", s("Pa55-word!"))?;
            assert_eq!(cred.kind, CredentialKind::Password);
            let (_, secret) = load_secret(c, &vault, &cred.id)?;
            assert_eq!(secret.password.unwrap().expose_secret(), "Pa55-word!");

            update(
                c,
                &vault,
                &cred.id,
                "prod admin",
                "admin",
                Some(s("new-pass")),
            )?;
            let (meta, secret) = load_secret(c, &vault, &cred.id)?;
            assert_eq!(meta.name, "prod admin");
            assert_eq!(secret.password.unwrap().expose_secret(), "new-pass");

            // Rename only keeps the secret.
            update(c, &vault, &cred.id, "renamed", "admin", None)?;
            let (_, secret) = load_secret(c, &vault, &cred.id)?;
            assert_eq!(secret.password.unwrap().expose_secret(), "new-pass");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn locked_vault_blocks_secrets_but_not_metadata() {
        let (vault, db) = setup();
        let id = db
            .with(|c| Ok(create_password(c, &vault, "x", "", s("pw"))?.id))
            .unwrap();
        vault.lock();
        db.with(|c| {
            assert_eq!(list(c)?.len(), 1, "metadata is readable while locked");
            assert_eq!(
                load_secret(c, &vault, &id).unwrap_err().code(),
                "VAULT_LOCKED"
            );
            assert_eq!(
                create_password(c, &vault, "y", "", s("pw"))
                    .unwrap_err()
                    .code(),
                "VAULT_LOCKED"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn keys_plain_and_encrypted() {
        let (vault, db) = setup();
        db.with(|c| {
            let plain = create_key(c, &vault, "deploy", "git", s(TEST_KEY), None)?;
            assert_eq!(plain.key_algorithm.as_deref(), Some("ssh-ed25519"));
            assert!(
                plain
                    .key_fingerprint
                    .as_deref()
                    .unwrap()
                    .starts_with("SHA256:")
            );
            let (_, secret) = load_secret(c, &vault, &plain.id)?;
            secret.decode_key()?;

            assert_eq!(
                create_key(c, &vault, "enc", "", s(TEST_KEY_ENC), None)
                    .unwrap_err()
                    .code(),
                "VALIDATION",
                "encrypted key without passphrase"
            );
            assert_eq!(
                create_key(c, &vault, "enc", "", s(TEST_KEY_ENC), Some(s("nope")))
                    .unwrap_err()
                    .code(),
                "VALIDATION"
            );
            let enc = create_key(
                c,
                &vault,
                "enc",
                "",
                s(TEST_KEY_ENC),
                Some(s("test-passphrase")),
            )?;
            assert_eq!(
                enc.key_fingerprint, plain.key_fingerprint,
                "same key, same fingerprint"
            );
            let (_, secret) = load_secret(c, &vault, &enc.id)?;
            secret.decode_key()?;

            assert_eq!(
                create_key(c, &vault, "junk", "", s("not a key"), None)
                    .unwrap_err()
                    .code(),
                "VALIDATION"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn swapping_ciphertexts_between_records_fails() {
        let (vault, db) = setup();
        db.with(|c| {
            let a = create_password(c, &vault, "a", "", s("secret-a"))?;
            let b = create_password(c, &vault, "b", "", s("secret-b"))?;
            c.execute(
                "UPDATE credentials SET secret = (SELECT secret FROM credentials WHERE id = ?1)
                 WHERE id = ?2",
                params![a.id, b.id],
            )?;
            assert_eq!(
                load_secret(c, &vault, &b.id).unwrap_err().code(),
                "INTERNAL"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_detaches_and_folder_default_resolution() {
        use crate::session::protocol::ProtocolKind;
        use crate::storage::models::SessionInput;
        use crate::storage::{folders, sessions};

        let (vault, db) = setup();
        db.with(|c| {
            let cred = create_password(c, &vault, "team", "ops", s("pw"))?;
            let prod = folders::create(c, None, "Prod")?;
            let web = folders::create(c, Some(&prod.id), "Web")?;
            c.execute(
                "UPDATE folders SET default_credential_id = ?1 WHERE id = ?2",
                params![cred.id, prod.id],
            )?;
            let input = SessionInput {
                folder_id: Some(web.id.clone()),
                name: "web-01".into(),
                protocol: ProtocolKind::Ssh,
                host: "10.0.0.1".into(),
                port: 22,
                username: String::new(),
                auth_method: "password".into(),
                credential_id: None,
                color: None,
                notes: String::new(),
                favorite: false,
                resource_monitor: true,
                tags: vec![],
            };
            let sess = sessions::create(c, &input)?;
            // Inherited from the grandparent folder.
            assert_eq!(resolve_for_session(c, &sess.id)?, Some(cred.id.clone()));
            assert_eq!(get(c, &cred.id)?.used_by, 1);

            delete(c, &cred.id)?;
            assert_eq!(resolve_for_session(c, &sess.id)?, None);
            let default: Option<String> = c.query_row(
                "SELECT default_credential_id FROM folders WHERE id = ?1",
                [&prod.id],
                |r| r.get(0),
            )?;
            assert_eq!(default, None, "folder default cleared by FK");
            Ok(())
        })
        .unwrap();
    }

    /// Phase 4 acceptance: no secret ever appears in the SQLite file.
    #[test]
    fn database_file_contains_no_secrets() {
        let dir = std::env::temp_dir().join(format!("nexterm-grep-{}", Uuid::new_v4()));
        let path = dir.join("t.db");
        let secrets = [
            "MasterPw-Unique-7731",
            "PasswordSecret-Unique-4412",
            "test-passphrase",
        ];
        {
            let db = Database::open(&path).unwrap();
            let vault = Vault::new_for_tests(db.clone(), Arc::new(MemoryKeyStore::default()));
            vault.setup(&s(secrets[0]), false).unwrap();
            db.with(|c| {
                create_password(c, &vault, "p", "root", s(secrets[1]))?;
                create_key(c, &vault, "k", "root", s(TEST_KEY_ENC), Some(s(secrets[2])))?;
                Ok(())
            })
            .unwrap();
            db.with(|c| Ok(c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?))
                .unwrap();
        }
        // Check the main file and any WAL/SHM side files.
        let key_body_line = TEST_KEY_ENC
            .lines()
            .nth(1)
            .unwrap()
            .trim()
            .as_bytes()
            .to_vec();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            for secret in secrets {
                assert!(
                    !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                    "secret {secret:?} found in database file"
                );
            }
            assert!(
                !bytes
                    .windows(key_body_line.len())
                    .any(|w| w == key_body_line),
                "private key material found in database file"
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn debug_output_is_redacted() {
        let secret = CredentialSecret {
            password: Some(s("hunter2")),
            private_key: None,
            passphrase: None,
        };
        assert!(!format!("{secret:?}").contains("hunter2"));
    }
}
