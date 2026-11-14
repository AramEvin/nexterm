# NexTerm

A free, open-source, security-focused SSH & SFTP client for Windows (Linux/macOS
later): a modern alternative to MobaXterm.

> **Status: Phase 2.** Multiple SSH sessions in tabs (password auth).
> Host keys are **not verified** until Phase 4: only use it on networks you trust.

## Planned v1 features

- Tabbed SSH terminal (xterm.js, WebGL)
- Session manager with folders, tags and search
- Encrypted credential vault (Argon2id + XChaCha20-Poly1305), OS keychain integration
- Key / agent / keyboard-interactive auth; strict host-key verification
- SFTP side panel with transfer queue
- Jump hosts (ProxyJump)
- Import from MobaXterm and PuTTY
- MSI/NSIS installers with signed auto-update

## Current features

**Phase 2: tabs**

- Any number of sessions, one per tab; closing or crashing one never affects the others
- Rename (double-click / F2 / right-click), drag to reorder, middle-click to close
- Right-click menu: Rename, Duplicate session, Reconnect / Disconnect, Close, Close other tabs
- **Reconnect** and **Duplicate** reuse the login without retyping the password (kept in Rust memory only, wiped when the tab closes)
- Status bar: host, user and state of the active tab; connected/open tab count

**Phase 1: SSH terminal**

- Quick connect (host, port, username, password)
- Full terminal: xterm.js with WebGL rendering (DOM fallback), 256 colours, UTF-8 / emoji / CJK
- Works with full-screen apps (`htop`, `vim`, `top`, `mc`); resizes with the window
- Copy/paste: **Ctrl+Shift+C / Ctrl+Shift+V**, Shift+Insert, right-click (copy selection / paste)
- Clear messages for wrong password, refused/timed-out connections, dropped sessions and `exit`

**Phase 0: foundation**

- Tauri 2 desktop shell: sidebar, tab bar, status bar
- Hardened defaults: strict CSP, deny-by-default IPC permissions, no shell/opener plugins
- Structured, daily-rotated logs in `%APPDATA%\NexTerm\logs` (14 days)
- CI on Windows: fmt, clippy, tests, `cargo audit`, `npm audit`, installer build

## Run it

Set up the toolchain first: **[docs/SETUP-WINDOWS.md](docs/SETUP-WINDOWS.md)**.

```powershell
npm ci
npm run tauri dev
```

## Development commands

| Command                                                                          | What it does                                                                |
| -------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `npm run tauri dev`                                                              | Run the app with hot reload                                                 |
| `npx tauri build`                                                                | Build release MSI + NSIS installers into `src-tauri/target/release/bundle/` |
| `npm run lint` / `npm run typecheck` / `npm run format`                          | Frontend checks / formatting                                                |
| `cd src-tauri; cargo fmt; cargo clippy --all-targets -- -D warnings; cargo test` | Rust checks                                                                 |

Log level: `$env:NEXTERM_LOG="debug"` before starting.

**Keep Tauri versions in lockstep:** the `tauri` crate (`src-tauri/Cargo.toml`)
and `@tauri-apps/api` / `@tauri-apps/cli` (`package.json`) must share
major.minor, otherwise the build stops with "version mismatched Tauri packages".

## Docs

- [Architecture](docs/ARCHITECTURE.md)
- [Security design & threat model](docs/SECURITY.md)
- [Roadmap](docs/ROADMAP.md)
- [Architecture decision records](docs/adr/)
- [Changelog](CHANGELOG.md)
