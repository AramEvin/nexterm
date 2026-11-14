# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

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
