# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

## [0.5.1] - 2026-11-16

### Changed

- Changed-host-key warning: removed the "type the host name → Remove key" box. It only cleared
  NexTerm's own list, so it did nothing when the old key came from `~/.ssh/known_hosts`. The
  dialog now says where the old key lives and how to remove it (`ssh-keygen -R <host>` or
  Credentials → Known hosts).

## [0.5.0] - 2026-11-16 — Phase 4: vault, key/agent/2FA login, host-key verification

### Added

- Credential vault: Argon2id (64 MiB, t=3) → KEK → random vault key → XChaCha20-Poly1305 per
  secret, with the credential id as associated data. First-run setup wizard, unlock screen,
  manual lock, auto-lock (5 min to 4 h, or never), change master password.
- "Remember on this device" (Windows Credential Manager / macOS Keychain / Linux keyutils).
- Credentials manager: passwords and private keys (OpenSSH, PEM, PuTTY .ppk, with passphrase).
  Keys are picked in a native file dialog opened by Rust, so the key never passes through the UI.
- Default credential per folder (inherited by subfolders); per-session login method and credential.
- Login methods: password, public key (SHA-2 RSA signatures when supported), SSH agent (Windows
  OpenSSH pipe, then Pageant; `SSH_AUTH_SOCK` on unix), keyboard-interactive / 2FA. A stored
  password answers a "Password:" prompt automatically.
- Host-key verification: unknown → fingerprint dialog (Reject / Accept once / Accept & save);
  changed → connection blocked with a red warning and no override; reads `~/.ssh/known_hosts`
  read-only. Known-hosts manager.
- Saved sessions connect through `ssh_connect_saved`; Rust resolves the credential. The UI asks
  for a password only when Rust reports `CREDENTIALS_REQUIRED`, and a locked vault opens the
  unlock screen and retries.
- New error codes: `VAULT_LOCKED`, `WRONG_MASTER_PASSWORD`, `HOST_KEY_CHANGED`,
  `HOST_KEY_REJECTED`, `CREDENTIALS_REQUIRED`, `CANCELLED`.
- Schema migration 0002 (vault_meta, credentials, known_hosts, folder default credential).
- Tests: crypto, vault state machine, credentials (incl. ciphertext swap), known hosts, prompt
  broker; acceptance tests that grep the database files and captured logs for secrets;
  integration tests for host-key TOFU / changed-key blocking, key, passphrase key, wrong key,
  agent and keyboard-interactive logins against real OpenSSH.

### Changed

- Reconnect / duplicate of saved sessions re-reads the session and vault every time.

### Security

- Host keys are now verified (the Phase 1–3 "accept and log" behaviour is gone).
- See SECURITY.md §4–§7 for the vault design, host-key policy, logging and known gaps.

## [0.4.0] - 2026-11-14 — Phase 3: session manager

### Added

- SQLite storage (`%APPDATA%\NexTerm\nexterm.db`) with versioned migrations; refuses a
  database from a newer version instead of touching it.
- Saved sessions: name, protocol, host, port, username, folder, colour, tags, notes, favourite;
  last-connected time.
- Nested folders (max 12 levels) with recursive session counts; deleting a folder deletes its
  contents after a confirmation that states how many sessions are affected.
- Sidebar library: favourites section, expand/collapse (remembered), drag & drop of sessions and
  folders, one shared right-click menu (connect, edit, duplicate, favourite, delete; new session,
  new subfolder, rename, delete folder), inline folder rename, keyboard (Enter, F2, Delete).
- Search by name, host, username, folder path or tag; `tag:` prefix; results grouped by folder.
- Session editor dialog; connect dialog for saved sessions asks only for the password.
- Quick connect: "Save to sessions" (host, port, user only).
- Tabs opened from a saved session use its name and show its colour.
- 12 IPC commands for the library, each granted explicitly in the capability file.
- Tests: 24 Rust storage tests (migrations, CRUD, cascade, cycles, depth, ordering, 2,000-row
  listing), 11 Vitest tests for tree building and search (1,000 sessions searched in under 50 ms).

### Fixed

- Native form controls (dropdowns, checkboxes) now follow the dark theme (`color-scheme`).
- Dragging tabs or tree rows no longer paints a text selection.

### Security

- No secrets in the database; `secure_delete` on; all inputs validated in Rust; parameterised
  SQL. See SECURITY.md §4a.

## [0.3.0] - 2026-11-14 — Phase 2: tabs and multiple sessions

### Added

- Multiple SSH sessions, one per tab, each in its own backend task.
- Tab actions: rename (double-click, F2, context menu), drag to reorder, middle-click close,
  duplicate session, reconnect, disconnect, close, close other tabs.
- Reconnect keeps the tab and its scrollback; Duplicate opens next to the source tab.
- Status bar shows the active tab's user@host:port and state, plus connected/open tab count.
- IPC: `ssh_reconnect`, `ssh_duplicate`, `ssh_forget`; `ssh_connect` now also returns a
  `connectionId`.
- Vitest unit tests for the tab store (lifecycle, cleanup, stale status, reorder, rename).
- Integration test: 20 concurrent sessions, closing one leaves 19 working, registry empty after
  repeated open/close.

### Changed

- WebGL renderer is attached only to the visible terminal (Chromium caps live WebGL contexts at
  about 16); hidden tabs use the DOM renderer.
- Home tab is pinned; session tabs scroll horizontally (mouse wheel) when they don't fit.

### Fixed

- Tab reordering uses pointer events: HTML5 drag-and-drop does not work inside the webview on
  Windows while Tauri's native file drop is enabled (needed for SFTP in Phase 5).

### Security

- Connection parameters (incl. password) for open tabs are kept in Rust memory as
  `SecretString` to support Reconnect/Duplicate; wiped when the tab closes, never written to
  disk, capped at 256 entries. See SECURITY.md §4.

## [0.2.0] - 2026-11-13 — Phase 1: first SSH terminal

### Added

- Quick-connect dialog (host, port, username, password) with client- and server-side validation.
- SSH via `russh` 0.65 (ring backend): password auth, PTY `xterm-256color`, login shell,
  keepalive (30 s), 15 s connect timeout.
- xterm.js 6 terminal: fit-to-pane, WebGL renderer with DOM fallback, Unicode 11 widths,
  10k-line scrollback, Ctrl+Shift+C/V, Shift+Insert, right-click copy/paste.
- Protocol boundary (`TerminalProtocol`, `OutputSink`, `SessionHandle`) and `SessionManager`
  with per-session tokio tasks and crash isolation.
- IPC: `ssh_connect`, `ssh_write`, `ssh_resize`, `ssh_disconnect`; terminal output streamed
  as raw bytes over a per-session `Channel`.
- Disconnect banner with reason (exit status, dropped connection, user disconnect).
- Startup error screen instead of a blank window.
- Integration tests against a real OpenSSH server (PTY size, resize, UTF-8, exit, auth failure,
  connection refused); CI job runs them in a container.

### Fixed

- Keystrokes could arrive out of order when typed quickly (concurrent IPC calls). Input is
  now queued with one write in flight.
- Terminal now gets keyboard focus after quick connect.

### Security

- Host keys are accepted **without verification** until Phase 4; fingerprint logged at WARN.
- `freezePrototype` disabled (incompatible with xterm.js 6); rationale in SECURITY.md.
- Password goes straight into `SecretString` (zeroized, redacted in Debug); never stored in
  frontend state.
- RUSTSEC-2023-0071 (rsa, via russh) documented as not applicable; see SECURITY.md §8.

## [0.1.0] - 2026-11-12

### Added — Phase 0: scaffold

- Tauri 2 + React 19 + TypeScript (strict) + Vite project, identifier `com.nexterm.app`,
  window title "NexTerm", minimum size 1000×650.
- Empty application shell: activity rail, sessions panel, tab bar, welcome pane, status bar.
- Tailwind CSS v4, shadcn/ui (Radix) base components, lucide icons, zustand store.
- ESLint (typescript-eslint strict, type-checked) + Prettier; `invoke()` restricted to `src/lib/ipc.ts`.
- Rust core module skeleton (`commands`, `session`, `ssh`, `sftp`, `vault`, `storage`,
  `import`, `settings`) with `AppError` (`{code, message}`, no internal details leaked).
- `tracing` logging to `%APPDATA%\NexTerm\logs`, daily rotation, 14 files kept; panics logged.
- `get_app_info` IPC command (version shown in the status bar).

### Added — app icon & Windows installer

- Original NexTerm app icon (`assets/icon.svg`), generated into all sizes via `tauri icon`.
- `scripts/build-windows-installer.sh`: build the NSIS installer from Linux (experimental cross-build).

### Security

- Strict CSP (`script-src 'self'`, no `unsafe-eval`, IPC-only `connect-src`), `freezePrototype`.
- Deny-by-default app command permissions via `build.rs` app manifest + `capabilities/main.json`.
- Removed the template's `opener` plugin; no `shell` plugin.
- `unsafe_code = "forbid"`; clippy denies `unwrap`/`expect` outside tests.
- CI: `cargo audit`, `npm audit`, Dependabot for cargo / npm / GitHub Actions.

### Fixed

- RUSTSEC-2026-0194/0195 (`quick-xml`, high) and RUSTSEC-2026-0009 (`time`, medium) via
  dependency update (tauri 2.12.5).
