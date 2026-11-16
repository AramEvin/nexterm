# NexTerm security design

Status: **Phase 5 (part 1).** Sections marked _(planned)_ are requirements for later phases.

## 1. Assets

| Asset                                            | Sensitivity                                          |
| ------------------------------------------------ | ---------------------------------------------------- |
| SSH passwords, private keys, key passphrases     | Critical                                             |
| Vault master password / derived key              | Critical                                             |
| Host-key trust store (known_hosts)               | High (integrity)                                     |
| Session inventory (hosts, usernames, jump paths) | Medium (reconnaissance value)                        |
| Logs                                             | Medium (must contain no secrets or terminal content) |

## 2. Threat model

**In scope**

- **Network attacker (MITM)** between NexTerm and an SSH server → host-key
  verification; changed keys block the connection _(Phase 4)_.
- **Offline disk theft / backup leak** of the user profile → secrets are only
  stored encrypted (vault), DB and logs contain no secrets _(Phase 3–4)_.
- **Malicious content rendered in the UI** (e.g. a crafted hostname, filename or
  terminal escape sequence attempting script injection in the WebView) →
  strict CSP, no `eval`, React escaping, deny-by-default IPC capabilities.
- **Compromised dependency** → lockfiles, `cargo audit` / `npm audit` in CI,
  Dependabot, small dependency set.
- **Other local users** on the same machine → per-user data directories; keychain
  entries are scoped to the Windows user.

**Out of scope** (documented, not defended against)

- Malware running as the same Windows user while the vault is unlocked (it can
  read process memory or keylog). Auto-lock reduces the window.
- A compromised SSH server (it sees everything you type in the session).
- Physical attacks on unlocked, unattended machines beyond auto-lock.

## 3. Desktop shell hardening (implemented in Phase 0)

- **CSP** (`tauri.conf.json`): `default-src 'self'`; `script-src 'self'` (no inline
  scripts, no `unsafe-eval`, no remote origins); `connect-src` limited to Tauri IPC;
  `object-src 'none'`; `frame-ancestors 'none'`; `form-action 'none'`.
- **Inline styles:** `style-src` allows `'unsafe-inline'` because xterm.js and Radix
  set `style` attributes at runtime. Tauri's automatic style nonces are disabled
  for `style-src` only (a nonce would make browsers ignore `'unsafe-inline'`).
  Style injection cannot execute code. Scripts keep full nonce/hash protection.
- **`freezePrototype`: disabled (Phase 1).** It froze `Object.prototype` against
  prototype-pollution gadgets, but xterm.js 6 assigns `obj.toString = ...` on plain
  objects, which throws in strict mode once the prototype is frozen (the JS
  "override mistake") and stopped the UI from loading. Residual risk is low: the
  CSP blocks injected scripts, and the app never merges untrusted objects (the
  usual pollution source). Re-enable if xterm.js stops doing this.
- **Startup error screen:** `src/boot-error.ts` (a module script, CSP-compliant)
  shows startup exceptions instead of a blank window. It displays the error
  message and stack only; no secrets exist at that point.
- **Deny-by-default commands:** app commands are declared in `build.rs`
  (`AppManifest`), so each one needs an explicit `allow-*` grant in
  `capabilities/main.json`. Only the `main` window gets a capability.
- **Minimal core permissions:** `core:app`, `core:event`, `core:window` defaults
  only. No `shell`, no `opener`, no `http` plugin. The `fs` plugin (Phase 5) will be
  scoped to user-selected paths.
- **No remote content:** the UI is served from bundled assets; no CDN fonts or scripts.
- `unsafe_code = "forbid"` in the Rust crate.

## 4. Typed passwords and per-tab connection state

- **Quick connect:** the password is sent once, in the `ssh_connect` request. It is
  not kept in any frontend store, and the input field is cleared after every attempt.
  In Rust it is deserialized straight into `secrecy::SecretString` (zeroized on drop,
  prints `[REDACTED]`; unit-tested).
- **Saved sessions** are connected with `ssh_connect_saved(sessionId)`. Rust reads the
  session, finds its credential (its own, or the nearest folder default), and
  decrypts it with the vault. Nothing secret crosses IPC. A password is typed only if
  no credential is linked, and the UI only asks after Rust replies
  `CREDENTIALS_REQUIRED`.
- **Reconnect / Duplicate:** each tab owns an opaque `connectionId` that points to a
  `ConnectSpec` in Rust:
  - `Direct` (quick connect) keeps the typed password as `SecretString` until the
    tab closes (`ssh_forget`) or the app exits.
  - `Saved` keeps only the session id (plus anything the user had to type). Every
    reconnect re-reads the session and the vault, so a locked vault or an edited
    session takes effect immediately.
  - Failed logins are never remembered, the store is capped at 256 entries, and every
    copy of a secret is an explicit `duplicate()` call.
- **Known gaps:**
  - russh's auth calls take `String`s, so russh holds short-lived copies that aren't
    zeroized.
  - JS strings in the webview (a typed password, a 2FA answer) are garbage-collected,
    not wiped.
  - Locking the vault does not wipe passwords typed for _quick-connect_ tabs. Close
    those tabs to drop them.

## 4a. Session database (Phase 3)

- `%APPDATA%\NexTerm\nexterm.db`, inside the user profile (Windows ACLs limit it
  to that user).
- **Contains no secrets:** host, port, username, folder, tags, colour and notes
  only. Passwords and keys go to the vault in Phase 4; sessions will reference
  them by `credential_id`. Phase 4 adds a test that greps the database file for
  test secrets.
- Quick connect's "Save to sessions" saves host, port and user only, never the
  password.
- `PRAGMA secure_delete = ON`, so deleted rows are overwritten instead of lingering in
  free pages. Foreign keys are enforced.
- **Input validation in Rust** for every field. Ids must be UUIDs, colours
  `#rrggbb`, and hosts/ports pass the same validators as connections. Length
  limits apply to names, notes and tags. All SQL is parameterised; the only
  `format!`ed SQL uses compile-time table and column names.
- Folder moves reject cycles (into itself or a descendant) and cap nesting depth.
- **Note:** session notes are stored in plaintext. Don't put secrets in notes;
  that's what the vault is for.

## 5. Credential vault (implemented in Phase 4)

**Key hierarchy**

```text
master password --Argon2id(salt, m=64 MiB, t=3, p=1)--> KEK
KEK --XChaCha20-Poly1305 unwraps--> vault key (256-bit random)
vault key --XChaCha20-Poly1305 seals--> each credential
```

- **KDF:** Argon2id v1.3, 64 MiB memory, 3 passes, 1 lane, 16-byte random salt.
  - Parameters are stored in `vault_meta`, so they can be raised later.
  - On unlock, stored parameters **below that minimum are refused**. A tampered
    header can't downgrade the KDF (unit-tested).
- **AEAD:** XChaCha20-Poly1305 with a fresh random 192-bit nonce per seal.
  - Blob format: `[version=1][nonce:24][ciphertext+tag]`.
  - The associated data is a label plus, for credentials, the credential id
    (`nexterm:credential:v1:<uuid>`). Copying one record's ciphertext into another row
    fails authentication (unit-tested).
- **Verification:** a wrong master password fails the AEAD tag on the wrapped key
  (`WRONG_MASTER_PASSWORD`). A separate `key_check` value lets a remembered key be
  validated without the password.
- **Changing the master password** re-wraps the vault key with a new salt. Credentials
  are not re-encrypted.
- **"Remember on this device":**
  - The vault key is stored in the OS credential store via `keyring` (Windows
    Credential Manager / DPAPI, macOS Keychain, Linux keyutils), under a random
    per-vault id, and the app unlocks at start-up.
  - Off by default; the UI explains the trade-off (any process running as the user
    can read it).
  - A stale entry that doesn't match the vault is detected and deleted.
- **Locking:** manual lock, or auto-lock after configurable inactivity (default 15
  minutes; 0 = never).
  - The UI reports activity at most every 30 s, and a background task checks every
    15 s, then notifies the UI (`vault-locked` event).
  - Locking drops the key from memory; open SSH sessions stay connected.
  - Unlocking after a manual lock or auto-lock always needs the master password, even
    when the vault is remembered on this device.
- **In memory:** the key is a `Zeroizing<[u8; 32]>`, and decrypted payloads are
  `Zeroizing<Vec<u8>>` → `SecretString`. `CredentialSecret` has a redacting `Debug` and
  no `Serialize`.
- **Private keys** are picked with a native file dialog **opened by Rust**
  (`credential_import_key`). The key text never passes through the webview, and the
  frontend can't make Rust read an arbitrary path.
  - Files over 64 KiB are refused.
  - The key is parsed first (OpenSSH, PEM/PKCS#8, PuTTY `.ppk`; a passphrase is
    verified), then stored together with its passphrase in the vault.
  - The public algorithm and SHA256 fingerprint are kept in clear for display.
- **Credential metadata** (name, kind, username, key fingerprint, usage count) is
  readable while locked. Secrets are not.
- **Acceptance test:** `database_file_contains_no_secrets` creates a vault, a password
  credential and a passphrase-protected key, then greps the database **and its WAL/SHM
  files** for the master password, the password, the passphrase and the key material.
  It also passed when re-checked against the database written by a real GUI session.

## 6. Host-key verification (implemented in Phase 4)

- **Lookup:** NexTerm's own `known_hosts` table, then the user's
  `~/.ssh/known_hosts` (read-only, hashed entries supported, never modified). A
  server's identity is `host:port`.
- **Trusted key** → connect silently (logged with its source).
- **Unknown key** → the UI shows host, port, key type and SHA256 fingerprint (the same
  format as `ssh-keygen -lf`).
  - Choices: **Reject** (focused by default) / **Accept once** / **Accept & save**.
  - If the host previously used a different key type, the prompt says so.
  - No answer within 3 minutes = reject.
- **Changed key** (same type, different key) → **connection blocked**, logged at ERROR.
  - The UI shows a red warning with the expected and presented fingerprints.
  - There is no "connect anyway". The only way forward is to remove the stored key,
    which requires typing the host name, and then go through the unknown-key prompt
    again.
  - A changed key in `~/.ssh/known_hosts` also blocks; that file has to be fixed by the
    user (`ssh-keygen -R`).
- **Fail closed:** if the trust store can't be read, the connection is refused.
- **Certificates:** host certificates are checked by their underlying public key.
  CA-based `@cert-authority` trust is not supported yet.
- **Acceptance test:** `host_key_tofu_trust_and_changed_key_is_blocked` (real OpenSSH
  server) covers prompt → saved, trusted without prompt, changed → `HOST_KEY_CHANGED`
  with no prompt shown, and reject → `HOST_KEY_REJECTED`.

### Authentication methods

| Method                     | Secret source                                                            | Notes                                                                                                           |
| -------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| Password                   | Vault credential, or typed                                               | If the server only allows keyboard-interactive, the password answers one hidden "Password" prompt automatically |
| Public key                 | Vault credential                                                         | RSA uses the best SHA-2 signature the server supports (`server-sig-algs`)                                       |
| Agent                      | Windows OpenSSH agent pipe, then Pageant (unix: `SSH_AUTH_SOCK`)         | Nothing stored in NexTerm; every agent identity is tried                                                        |
| Keyboard-interactive (2FA) | Prompts shown to the user; a stored password answers the password prompt | At most 10 rounds; answers are never logged                                                                     |

After a successful first method, a server that asks for a second factor
(partial success) gets a keyboard-interactive round.

## 6a. File manager (implemented in Phase 5)

The file manager reads and writes the user's disk and remote servers, and it
consumes names chosen by the server. Threats and controls:

| Threat                                                                                                                   | Control                                                                                                                                                                                                                                                                                                                                 |
| ------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A malicious server returns entries like `..\..\AppData\x`, `C:\evil`, `x:stream`, `CON` inside a folder being downloaded | Every name from the source is checked with `paths::safe_local_name` before it becomes a local path: no separators, `.`/`..`, NUL, control chars, `<>:"\|?*`, trailing dot/space or reserved device names. Unsafe names are skipped and listed in the result (integration test `hostile_names_from_the_server_never_escape_the_target`). |
| Symlink loops or links pointing outside the folder                                                                       | Recursive copy and delete use `lstat`/`symlink_metadata` and never follow links inside a tree; `remove_dir_all` doesn't follow links (CVE-2022-21658 fix in std). Depth ≤ 64, ≤ 200,000 items per job.                                                                                                                                  |
| Data loss through clashes                                                                                                | Default for clashes in a folder that isn't on screen is "keep both". Copying a file onto itself or a folder into itself is refused. Destructive confirmation dialogs focus Cancel.                                                                                                                                                      |
| Half-written files                                                                                                       | Local targets are written to `*.nexterm-part` and renamed only when complete; failed or cancelled copies delete their partial file.                                                                                                                                                                                                     |
| Compromised UI sending arbitrary paths                                                                                   | Same trust as today's UI (strict CSP, no remote content). Every path and name is still validated in Rust (`commands::sftp`): absolute local paths only, length limits, no NUL, new names can't contain separators, drives/root can't be deleted, renamed or copied whole.                                                               |
| Credentials                                                                                                              | Remote panes use the same `ConnectSpec` resolution as terminals: saved sessions re-read the vault, terminal-tab panes reuse the tab's remembered spec in Rust. The UI only holds an opaque `fsId`.                                                                                                                                      |
| Host keys                                                                                                                | SFTP connections go through the same `ssh::connect`, including TOFU and changed-key blocking.                                                                                                                                                                                                                                           |

Remote ↔ remote transfers are relayed through this PC (read from A, write to
B); no credentials are forwarded to either server.

## 7. Logging policy (implemented in Phase 0)

- Location `%APPDATA%\NexTerm\logs`, daily rotation, 14 files retained.
- Logged: app start/stop, connection events (host, port, username, session id),
  auth _method_ (not credentials), auth failures, host-key fingerprints/decisions,
  session close reason, errors, panics.
- **Never logged:** passwords, keys, passphrases, vault keys, terminal input/output,
  file contents.
- `AppError` sends only a code + generic message to the UI; details stay in the log.
- Phase 4 adds: vault created/unlocked/locked/auto-locked, failed master password,
  credential created/deleted (id, kind, key fingerprint), host-key decisions, and
  `HOST KEY CHANGED` at ERROR level.
- Phase 5 adds: SFTP session opened/closed (host, port), transfer started/finished/
  cancelled/failed (file count, bytes, direction only), items deleted (count). File and
  folder names are **not** logged, and `FILE` errors are logged by code only.
- **Acceptance test:** `logs_contain_no_secrets` captures all log output (TRACE level)
  across password, key, wrong-password and keyboard-interactive logins. It asserts that
  no password, passphrase or key line appears, while the audit events are present.

## 8. Supply chain

- `Cargo.lock` and `package-lock.json` committed; CI uses `--locked` / `npm ci`.
- CI gates: `cargo audit` (any advisory fails); `npm audit --omit=dev
--audit-level=moderate` (shipped JS); `npm audit --audit-level=high` (dev tooling).
- Known accepted findings (2026-11-12):
  - npm: moderate ReDoS in `minimatch`, pulled in by ESLint / typescript-eslint.
    Dev-only, never shipped; revisit when upstream updates.
  - cargo-audit _warnings_ (not vulnerabilities): `glib` 0.18 unsound iterator
    (RUSTSEC-2024-0429) and `proc-macro-error` unmaintained (RUSTSEC-2024-0370).
    Both come from Tauri's Linux GTK stack. `cargo tree --target
x86_64-pc-windows-msvc` confirms they are not in Windows builds. Resolved when
    Tauri moves to gtk-rs ≥ 0.20.
- Phase 0 audit fixed: `quick-xml` RUSTSEC-2026-0194/0195 (high) and `time`
  RUSTSEC-2026-0009 (medium), via lockfile update (tauri 2.12.0 → 2.12.5).
- **Exception (Phase 1): RUSTSEC-2023-0071** (`rsa`, Marvin timing attack on RSA
  _decryption_), ignored in `src-tauri/.cargo/audit.toml`. russh 0.65.1 pins
  `rsa =0.10.0-rc.18`, which already uses constant-time `crypto-bigint` (the upstream
  fix) and is flagged only because a pre-release sorts below `0.10.0`. SSH uses RSA
  only for signatures; russh never performs RSA decryption. Review when russh moves
  to `rsa >= 0.10.0`, or by 2027-02-01.
- **Crypto backend:** russh is built with `ring` instead of its default `aws-lc-rs`
  (smaller, simpler to build for Windows from any host).
- No `rust-version` (MSRV) is set: an MSRV makes Cargo's resolver hold back
  patched crates. The repo tracks current stable via `rust-toolchain.toml`.
- Dependabot weekly for cargo, npm, GitHub Actions. Tauri JS and Rust packages must
  move together (same major.minor).

## 9. Reporting vulnerabilities

Until a dedicated address exists, open a GitHub **private security advisory** on
the repository. Please don't file public issues for vulnerabilities.
