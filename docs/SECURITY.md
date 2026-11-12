# NexTerm security design

Status: **Phase 0 draft.** Sections marked _(planned)_ describe the design that
later phases must implement. They are requirements, not current behaviour.

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
- **`freezePrototype: true`**: freezes `Object.prototype` to block
  prototype-pollution gadgets.
- **Deny-by-default commands:** app commands are declared in `build.rs`
  (`AppManifest`), so each one needs an explicit `allow-*` grant in
  `capabilities/main.json`. Only the `main` window gets a capability.
- **Minimal core permissions:** `core:app`, `core:event`, `core:window` defaults
  only. No `shell`, no `opener`, no `http` plugin. The `fs` plugin (Phase 5) will be
  scoped to user-selected paths.
- **No remote content:** the UI is served from bundled assets; no CDN fonts or scripts.
- `unsafe_code = "forbid"` in the Rust crate.

## 4. Vault crypto design _(planned, Phase 4)_

- **KDF:** Argon2id, memory ≥ 64 MiB, iterations ≥ 3, parallelism 1–4, random
  16-byte salt → 256-bit key-encryption key (KEK). Parameters are stored with the
  vault so they can be raised later without breaking old vaults.
- **Key hierarchy:** a random 256-bit **vault key** encrypts secrets; the KEK wraps
  the vault key. Changing the master password re-wraps one key instead of
  re-encrypting every secret.
- **AEAD:** XChaCha20-Poly1305 with a random 24-byte nonce per encryption.
  Record format: `{version, salt, nonce, ciphertext}`. The record's `credential_id`
  is bound as associated data so ciphertexts can't be swapped between records.
- **"Remember on this device":** the vault key is stored in Windows Credential
  Manager via `keyring` (protected by DPAPI under the user's logon). Trade-off: any
  process running as that user can read it. Off by default; documented in the UI.
- **Memory hygiene:** `secrecy::SecretString`, `zeroize::Zeroizing<Vec<u8>>`; secret
  types implement neither `Debug` nor `Display` and are never `Serialize`d.
- **Auto-lock:** configurable idle timeout + manual lock; locking drops the key.

## 5. Host-key verification _(planned, Phase 4)_

Unknown host → dialog with key type + SHA256 fingerprint → _Accept once_ / _Accept
& save_ / _Reject_. Changed key → connection blocked with a MITM warning; no
one-click override. Trust store: NexTerm's own known_hosts; optionally read
`%USERPROFILE%\.ssh\known_hosts`.

**Phase 1–3 exception:** until Phase 4, host keys are accepted and a warning is
logged. **Do not use pre-Phase-4 builds against untrusted networks.**

## 6. Logging policy (implemented in Phase 0)

- Location `%APPDATA%\NexTerm\logs`, daily rotation, 14 files retained.
- Logged: app start/stop, connection events, auth _method_ (not credentials),
  host-key decisions, errors, panics.
- **Never logged:** passwords, keys, passphrases, vault keys, terminal input/output,
  file contents.
- `AppError` sends only a code + generic message to the UI; details stay in the log.

## 7. Supply chain

- `Cargo.lock` and `package-lock.json` committed; CI uses `--locked` / `npm ci`.
- CI gates: `cargo audit` (any advisory fails); `npm audit --omit=dev
--audit-level=moderate` (shipped JS); `npm audit --audit-level=high` (dev tooling).
- Known accepted finding (2026-11-12): moderate ReDoS in `minimatch`, pulled in by
  ESLint / typescript-eslint. Dev-only, never shipped; revisit when upstream updates.
- Dependabot weekly for cargo, npm, GitHub Actions. Tauri JS and Rust packages must
  move together (same major.minor).

## 8. Reporting vulnerabilities

Until a dedicated address exists, open a GitHub **private security advisory** on
the repository. Please don't file public issues for vulnerabilities.
