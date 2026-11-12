# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

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

### Security

- Strict CSP (`script-src 'self'`, no `unsafe-eval`, IPC-only `connect-src`), `freezePrototype`.
- Deny-by-default app command permissions via `build.rs` app manifest + `capabilities/main.json`.
- Removed the template's `opener` plugin; no `shell` plugin.
- `unsafe_code = "forbid"`; clippy denies `unwrap`/`expect` outside tests.
- CI: `cargo audit`, `npm audit`, Dependabot for cargo / npm / GitHub Actions.

### Fixed

- RUSTSEC-2026-0194/0195 (`quick-xml`, high) and RUSTSEC-2026-0009 (`time`, medium) via
  dependency update (tauri 2.12.5).
