//! Vault cryptography. Small, boring, and fully covered by tests.
//!
//! - KDF: Argon2id (RFC 9106). Parameters are stored with the vault so they can
//!   be raised later without breaking existing vaults.
//! - AEAD: XChaCha20-Poly1305 with a random 192-bit nonce per message (random
//!   nonces are safe at this size; no counter state to persist).
//! - Sealed blob format: `[version:1][nonce:24][ciphertext+tag]`.
//! - Every seal binds an AAD string (e.g. the credential id) so a ciphertext
//!   can't be moved to another record undetected.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Generate, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const BLOB_VERSION: u8 = 1;

/// A 256-bit symmetric key that is wiped from memory on drop.
pub type SecretKey = Zeroizing<[u8; KEY_LEN]>;

/// Argon2id cost parameters (stored in the vault header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Parallelism.
    pub p: u32,
}

impl KdfParams {
    /// Production default: 64 MiB, 3 passes, 1 lane (SECURITY.md §5 minimum).
    pub const DEFAULT: Self = Self {
        m_kib: 64 * 1024,
        t: 3,
        p: 1,
    };

    /// Lower bound accepted when opening a vault, so a tampered header can't
    /// downgrade the KDF to something trivially brute-forceable.
    pub const MINIMUM: Self = Self {
        m_kib: 64 * 1024,
        t: 3,
        p: 1,
    };

    pub fn check_minimum(&self) -> AppResult<()> {
        if self.m_kib < Self::MINIMUM.m_kib || self.t < Self::MINIMUM.t || self.p < 1 {
            return Err(AppError::Internal(
                "vault KDF parameters are below the allowed minimum".into(),
            ));
        }
        Ok(())
    }
}

/// Cryptographically secure random bytes from the OS.
pub fn random_bytes<const N: usize>() -> AppResult<Zeroizing<[u8; N]>> {
    let mut out = Zeroizing::new([0u8; N]);
    // A 256-bit AEAD key from the OS RNG; slice what we need.
    let mut filled = 0;
    while filled < N {
        let key = chacha20poly1305::Key::generate();
        let take = (N - filled).min(key.len());
        out[filled..filled + take].copy_from_slice(&key[..take]);
        filled += take;
    }
    Ok(out)
}

/// Derive a key-encryption key from the master password.
pub fn derive_kek(password: &[u8], salt: &[u8], params: KdfParams) -> AppResult<SecretKey> {
    let p = Params::new(params.m_kib, params.t, params.p, Some(KEY_LEN))
        .map_err(|e| AppError::Internal(format!("invalid KDF parameters: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password, salt, out.as_mut())
        .map_err(|e| AppError::Internal(format!("key derivation failed: {e}")))?;
    Ok(out)
}

/// Encrypt `plaintext` under `key`, bound to `aad`.
pub fn seal(key: &[u8; KEY_LEN], aad: &[u8], plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::generate();
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| AppError::Internal("encryption failed".into()))?;
    let mut blob = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    blob.push(BLOB_VERSION);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Decrypt a blob produced by [`seal`]. Fails on a wrong key, wrong AAD or any
/// modification of the blob (Poly1305 tag check).
pub fn open(key: &[u8; KEY_LEN], aad: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, OpenError> {
    if blob.len() < 1 + NONCE_LEN + 16 || blob[0] != BLOB_VERSION {
        return Err(OpenError::Malformed);
    }
    let nonce = XNonce::try_from(&blob[1..=NONCE_LEN]).map_err(|_| OpenError::Malformed)?;
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &blob[1 + NONCE_LEN..],
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| OpenError::Authentication)
}

#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not a blob we understand (truncated or unknown version).
    Malformed,
    /// Wrong key, wrong AAD, or tampered data.
    Authentication,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters for fast unit tests only.
    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    };

    #[test]
    fn seal_open_roundtrip() {
        let key = random_bytes::<KEY_LEN>().unwrap();
        let blob = seal(&key, b"cred:1", b"hunter2").unwrap();
        assert_eq!(blob[0], BLOB_VERSION);
        assert!(
            !blob.windows(7).any(|w| w == b"hunter2"),
            "plaintext visible in blob"
        );
        assert_eq!(open(&key, b"cred:1", &blob).unwrap().as_slice(), b"hunter2");
    }

    #[test]
    fn nonces_are_unique() {
        let key = random_bytes::<KEY_LEN>().unwrap();
        let a = seal(&key, b"x", b"same").unwrap();
        let b = seal(&key, b"x", b"same").unwrap();
        assert_ne!(a, b, "same plaintext must not produce the same blob");
    }

    #[test]
    fn wrong_key_aad_or_tamper_fails() {
        let key = random_bytes::<KEY_LEN>().unwrap();
        let other = random_bytes::<KEY_LEN>().unwrap();
        let blob = seal(&key, b"cred:1", b"secret").unwrap();
        assert_eq!(
            open(&other, b"cred:1", &blob),
            Err(OpenError::Authentication)
        );
        // Moving a ciphertext to another record (different AAD) is detected.
        assert_eq!(open(&key, b"cred:2", &blob), Err(OpenError::Authentication));
        let mut tampered = blob.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert_eq!(
            open(&key, b"cred:1", &tampered),
            Err(OpenError::Authentication)
        );
        assert_eq!(
            open(&key, b"cred:1", &blob[..10]),
            Err(OpenError::Malformed)
        );
        let mut bad_version = blob;
        bad_version[0] = 9;
        assert_eq!(
            open(&key, b"cred:1", &bad_version),
            Err(OpenError::Malformed)
        );
    }

    #[test]
    fn kdf_is_deterministic_and_salted() {
        let salt = [7u8; SALT_LEN];
        let a = derive_kek(b"correct horse", &salt, TEST_KDF).unwrap();
        let b = derive_kek(b"correct horse", &salt, TEST_KDF).unwrap();
        let c = derive_kek(b"correct horse", &[8u8; SALT_LEN], TEST_KDF).unwrap();
        let d = derive_kek(b"wrong horse", &salt, TEST_KDF).unwrap();
        assert_eq!(*a, *b);
        assert_ne!(*a, *c);
        assert_ne!(*a, *d);
    }

    #[test]
    fn rejects_weak_stored_params() {
        assert!(KdfParams::DEFAULT.check_minimum().is_ok());
        assert!(TEST_KDF.check_minimum().is_err());
        assert!(
            KdfParams {
                t: 1,
                ..KdfParams::DEFAULT
            }
            .check_minimum()
            .is_err()
        );
    }

    #[test]
    fn random_bytes_fill_any_length() {
        let a = random_bytes::<SALT_LEN>().unwrap();
        let b = random_bytes::<SALT_LEN>().unwrap();
        assert_ne!(*a, *b);
        let long = random_bytes::<48>().unwrap();
        assert!(long[32..].iter().any(|&x| x != 0));
    }
}
