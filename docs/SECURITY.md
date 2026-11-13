# NexTerm security design

Status: **Phase 1.** Sections marked _(planned)_ describe the design that
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

## 4. Password handling in Phase 1 (quick connect)

- The password is typed into the quick-connect dialog and sent once, in the
  `ssh_connect` request. It is **not** kept in any frontend store; the input field
  is cleared after every attempt.
- In Rust it is deserialized directly into `secrecy::SecretString` (zeroized on
  drop, prints `[REDACTED]` in `Debug`; a unit test checks this). It is dropped
  right after authentication.
- **Known gap:** `russh::authenticate_password` takes a `String`, so russh holds a
  short-lived copy that is not zeroized. The JS string in the webview is likewise
  garbage-collected, not wiped. Phase 4 moves stored credentials into the vault so
  saved passwords never pass through the frontend.

## 5. Vault crypto design _(planned, Phase 4)_

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

## 6. Host-key verification _(planned, Phase 4)_

Unknown host → dialog with key type + SHA256 fingerprint → _Accept once_ / _Accept
& save_ / _Reject_. Changed key → connection blocked with a MITM warning; no
one-click override. Trust store: NexTerm's own known_hosts; optionally read
`%USERPROFILE%\.ssh\known_hosts`.

**Phase 1–3 exception (active):** until Phase 4, every host key is accepted. The
key type and SHA256 fingerprint are logged at WARN level
(`host key accepted WITHOUT verification`), and the quick-connect dialog warns the
user. **Do not use pre-Phase-4 builds against untrusted networks.** To compare a
fingerprint by hand: `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on the server.

## 7. Logging policy (implemented in Phase 0)

- Location `%APPDATA%\NexTerm\logs`, daily rotation, 14 files retained.
- Logged: app start/stop, connection events (host, port, username, session id),
  auth _method_ (not credentials), auth failures, host-key fingerprints/decisions,
  session close reason, errors, panics.
- **Never logged:** passwords, keys, passphrases, vault keys, terminal input/output,
  file contents.
- `AppError` sends only a code + generic message to the UI; details stay in the log.

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
