//! Host-key trust (SECURITY.md §6).
//!
//! Lookup order for `host:port`:
//! 1. NexTerm's own store (`known_hosts` table).
//! 2. The user's OpenSSH `~/.ssh/known_hosts`, **read-only** (also understands
//!    hashed entries).
//!
//! - Same key type, same key → trusted.
//! - Same key type, different key → **changed**: the connection is blocked.
//! - No key of that type → unknown: the user decides (accept once / save /
//!   reject), and is told if the server previously used a different key type.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use russh::keys::{HashAlg, PublicKey, PublicKeyOrCertificate};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::storage::now_ms;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted { source: &'static str },
    Unknown { other_known_types: Vec<String> },
    Changed { stored: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub added_at: i64,
}

/// The plain public key presented by the server (certificates are checked by
/// their underlying key; CA-based host certificates are not supported yet).
pub fn server_public_key(key: &PublicKeyOrCertificate) -> PublicKey {
    match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
        PublicKeyOrCertificate::Certificate(cert) => {
            PublicKey::new(cert.public_key().clone(), String::new())
        }
    }
}

pub fn describe(key: &PublicKey) -> (String, String) {
    (
        key.algorithm().to_string(),
        key.fingerprint(HashAlg::Sha256).to_string(),
    )
}

fn openssh_body(key: &PublicKey) -> AppResult<String> {
    let line = PublicKey::new(key.key_data().clone(), String::new())
        .to_openssh()
        .map_err(|e| AppError::Internal(format!("encode host key: {e}")))?;
    Ok(line.trim().to_string())
}

/// Check `key` for `host:port` against NexTerm's store and (optionally) the
/// user's OpenSSH known_hosts file.
pub fn check(
    conn: &Connection,
    host: &str,
    port: u16,
    key: &PublicKey,
    openssh_file: Option<&Path>,
) -> AppResult<HostKeyStatus> {
    let (key_type, _) = describe(key);
    let mut stmt = conn.prepare(
        "SELECT key_type, public_key, fingerprint FROM known_hosts
         WHERE host = ?1 AND port = ?2 AND key_type <> 'tls-certificate'",
    )?;
    let stored: Vec<(String, String, String)> = stmt
        .query_map(params![host, port], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    if let Some((_, public, fingerprint)) = stored.iter().find(|(t, _, _)| *t == key_type) {
        return Ok(if *public == openssh_body(key)? {
            HostKeyStatus::Trusted { source: "nexterm" }
        } else {
            HostKeyStatus::Changed {
                stored: format!("{key_type} {fingerprint} (saved in NexTerm)"),
            }
        });
    }

    if let Some(path) = openssh_file.filter(|p| p.exists()) {
        // Compare key material only: russh's equality also compares the
        // comment field, which known_hosts entries don't carry.
        let bare = PublicKey::new(key.key_data().clone(), String::new());
        match russh::keys::check_known_hosts_path(host, port, &bare, path) {
            Ok(true) => return Ok(HostKeyStatus::Trusted { source: "openssh" }),
            Ok(false) => {}
            Err(russh::keys::Error::KeyChanged { line }) => {
                return Ok(HostKeyStatus::Changed {
                    stored: format!(
                        "a different {key_type} key in ~/.ssh/known_hosts (line {line})"
                    ),
                });
            }
            Err(e) => tracing::warn!(error = %e, "could not read ~/.ssh/known_hosts; ignoring it"),
        }
    }

    Ok(HostKeyStatus::Unknown {
        other_known_types: stored.into_iter().map(|(t, _, _)| t).collect(),
    })
}

/// Trust `key` for `host:port` from now on.
pub fn save(conn: &Connection, host: &str, port: u16, key: &PublicKey) -> AppResult<()> {
    let (key_type, fingerprint) = describe(key);
    conn.execute(
        "INSERT INTO known_hosts (host, port, key_type, public_key, fingerprint, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (host, port, key_type) DO UPDATE SET
             public_key = excluded.public_key, fingerprint = excluded.fingerprint,
             added_at = excluded.added_at",
        params![
            host,
            port,
            key_type,
            openssh_body(key)?,
            fingerprint,
            now_ms()
        ],
    )?;
    Ok(())
}

/// `key_type` used for pinned FTPS certificates in the same table.
pub const CERT_TYPE: &str = "tls-certificate";

/// Check a pinned TLS certificate (SHA-256 fingerprint, `SHA256:<base64>`) for
/// `host:port`. Only consulted for certificates that are **not** signed by a
/// trusted CA (self-signed / internal CA): CA-valid certificates never get here.
pub fn check_cert(
    conn: &Connection,
    host: &str,
    port: u16,
    fingerprint: &str,
) -> AppResult<HostKeyStatus> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT fingerprint FROM known_hosts WHERE host = ?1 AND port = ?2 AND key_type = ?3",
            params![host, port, CERT_TYPE],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match stored {
        Some(fp) if fp == fingerprint => HostKeyStatus::Trusted { source: "nexterm" },
        Some(fp) => HostKeyStatus::Changed {
            stored: format!("certificate {fp} (saved in NexTerm)"),
        },
        None => HostKeyStatus::Unknown {
            other_known_types: Vec::new(),
        },
    })
}

/// Pin a TLS certificate fingerprint for `host:port`.
pub fn save_cert(conn: &Connection, host: &str, port: u16, fingerprint: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO known_hosts (host, port, key_type, public_key, fingerprint, added_at)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)
         ON CONFLICT (host, port, key_type) DO UPDATE SET
             public_key = excluded.public_key, fingerprint = excluded.fingerprint,
             added_at = excluded.added_at",
        params![host, port, CERT_TYPE, fingerprint, now_ms()],
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> AppResult<Vec<KnownHost>> {
    let mut stmt = conn.prepare(
        "SELECT host, port, key_type, fingerprint, added_at FROM known_hosts
         ORDER BY host COLLATE NOCASE, port, key_type",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(KnownHost {
            host: r.get(0)?,
            port: r.get(1)?,
            key_type: r.get(2)?,
            fingerprint: r.get(3)?,
            added_at: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Forget every stored key for `host:port`. Returns how many were removed.
pub fn remove(conn: &Connection, host: &str, port: u16) -> AppResult<usize> {
    Ok(conn.execute(
        "DELETE FROM known_hosts WHERE host = ?1 AND port = ?2",
        params![host, port],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    fn key(fixture: &str) -> PublicKey {
        PublicKey::from_openssh(fixture).unwrap()
    }

    const KEY_A: &str = include_str!("../../tests/fixtures/test_ed25519.pub");
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHo0b/OsK3MI4yXM+vWWWf3Kxr8hAe8wDdrfVXCmUWYG other";

    #[test]
    fn unknown_then_trusted_then_changed() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let a = key(KEY_A);
            assert_eq!(
                check(c, "srv", 22, &a, None)?,
                HostKeyStatus::Unknown {
                    other_known_types: vec![]
                }
            );
            save(c, "srv", 22, &a)?;
            assert_eq!(
                check(c, "SRV", 22, &a, None)?,
                HostKeyStatus::Trusted { source: "nexterm" },
                "host match is case-insensitive"
            );
            // Same type, different key: blocked.
            let b = key(KEY_B);
            assert!(matches!(
                check(c, "srv", 22, &b, None)?,
                HostKeyStatus::Changed { .. }
            ));
            // Port is part of the identity.
            assert!(matches!(
                check(c, "srv", 2222, &b, None)?,
                HostKeyStatus::Unknown { .. }
            ));
            assert_eq!(list(c)?.len(), 1);
            assert_eq!(remove(c, "srv", 22)?, 1);
            assert!(matches!(
                check(c, "srv", 22, &b, None)?,
                HostKeyStatus::Unknown { .. }
            ));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reads_openssh_known_hosts_read_only() {
        let dir = std::env::temp_dir().join(format!("nexterm-kh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("known_hosts");
        let a = key(KEY_A);
        std::fs::write(&file, format!("[srv]:2222 {}\n", KEY_A.trim())).unwrap();
        let before = std::fs::read(&file).unwrap();

        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            assert_eq!(
                check(c, "srv", 2222, &a, Some(&file))?,
                HostKeyStatus::Trusted { source: "openssh" }
            );
            assert!(matches!(
                check(c, "srv", 2222, &key(KEY_B), Some(&file))?,
                HostKeyStatus::Changed { .. }
            ));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "OpenSSH file must not be modified"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
