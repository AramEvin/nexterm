-- NexTerm schema v2 (Phase 4): credential vault, known hosts, folder default credentials.
-- Secrets are stored ONLY as AEAD ciphertext (see SECURITY.md §5). Everything
-- in clear text here is non-secret metadata (names, usernames, key fingerprints).

-- Single-row vault header.
CREATE TABLE vault_meta (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    vault_id           TEXT NOT NULL,                 -- random; names the keychain entry
    kdf                TEXT NOT NULL DEFAULT 'argon2id',
    kdf_m_kib          INTEGER NOT NULL,
    kdf_t              INTEGER NOT NULL,
    kdf_p              INTEGER NOT NULL,
    salt               BLOB NOT NULL,                 -- 16 random bytes
    wrapped_key        BLOB NOT NULL,                 -- vault key sealed under the KEK
    key_check          BLOB NOT NULL,                 -- constant sealed under the vault key
    auto_lock_minutes  INTEGER NOT NULL DEFAULT 15,   -- 0 = never
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE TABLE credentials (
    id               TEXT PRIMARY KEY NOT NULL,       -- UUID v4, also the AEAD associated data
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('password', 'key')),
    username         TEXT NOT NULL DEFAULT '',
    key_algorithm    TEXT,                            -- e.g. ssh-ed25519 (public info)
    key_fingerprint  TEXT,                            -- SHA256:... of the public key
    secret           BLOB NOT NULL,                   -- sealed JSON {password|privateKey, passphrase}
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

-- NexTerm's own trust store (OpenSSH's known_hosts is read but never written).
CREATE TABLE known_hosts (
    host         TEXT NOT NULL COLLATE NOCASE,
    port         INTEGER NOT NULL,
    key_type     TEXT NOT NULL,
    public_key   TEXT NOT NULL,                       -- OpenSSH base64
    fingerprint  TEXT NOT NULL,                       -- SHA256:...
    added_at     INTEGER NOT NULL,
    PRIMARY KEY (host, port, key_type)
);

ALTER TABLE folders ADD COLUMN default_credential_id TEXT
    REFERENCES credentials(id) ON DELETE SET NULL;
