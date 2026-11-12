# NexTerm architecture

## Overview

```
┌──────────────── Frontend (React + TS + xterm.js) ─────────────────┐
│ Sidebar (session tree) │ Tabs │ Terminal │ SFTP panel │ Settings    │
└──────────────────────────────┬─────────────────────────────────────┘
                               │ Tauri IPC: commands (request/response)
                               │ + tauri::ipc::Channel (streaming output)
┌──────────────────────────────┴──── Rust core ──────────────────────┐
│ commands/   thin IPC layer, input validation only                   │
│ session/    SessionManager: lifecycle of live connections; protocol │
│ ssh/        connect, auth, PTY, resize, jump host, host keys        │
│ sftp/       file operations + transfer queue with progress          │
│ vault/      master key, encrypt/decrypt, keychain, lock/unlock      │
│ storage/    SQLite repositories + migrations                        │
│ import/     MobaXterm.ini + PuTTY registry importers                │
│ settings/   app settings model                                      │
└────────────────────────────────────────────────────────────────────┘
```

Tauri runs one native process. The UI is a WebView2 page loaded from bundled
files (never from the network). The page can only reach Rust through IPC
commands that are explicitly granted in `src-tauri/capabilities/`.

## Key rules

1. **Secrets stay in Rust.** The frontend references credentials by `credential_id`;
   decryption happens in `vault/` at connect time.
2. **One IPC entry point per side.** Frontend: `src/lib/ipc.ts` is the only file
   allowed to call `invoke()` (enforced by ESLint). Rust: `commands/` only validates
   and delegates.
3. **Errors:** every command returns `Result<T, AppError>`. `AppError` serializes to
   `{ code, message }` with a user-safe message; details go to the log only.
4. **Streaming:** terminal output uses one `tauri::ipc::Channel<Vec<u8>>` per
   session, not global events. Input: `ssh_write`; resize: `ssh_resize`.
5. **Isolation:** each live session runs in its own tokio task.
6. **Protocols:** SSH is the first implementation behind the protocol boundary in
   `session/protocol.rs` (see ADR 0002).

## Adding an IPC command (checklist)

1. Implement it in `src-tauri/src/commands/<area>.rs`, returning `AppResult<T>`.
2. Register it in `generate_handler![]` in `lib.rs`.
3. Add its name to `AppManifest::commands(&[...])` in `build.rs`.
4. Grant `allow-<command-name>` in `capabilities/main.json`.
5. Add the TS type to `src/types/` and a wrapper to `src/lib/ipc.ts`.

Skipping step 3 or 4 makes the call fail with a "not allowed" error: that's the
deny-by-default model working.

## Frontend layout

| Path                       | Purpose                                           |
| -------------------------- | ------------------------------------------------- |
| `src/components/layout/`   | App shell: tab bar, status bar, panes             |
| `src/components/sidebar/`  | Session tree (Phase 3)                            |
| `src/components/terminal/` | xterm.js wrapper (Phase 1)                        |
| `src/components/sftp/`     | SFTP panel (Phase 5)                              |
| `src/components/dialogs/`  | Modal dialogs                                     |
| `src/components/ui/`       | shadcn/ui primitives (copied source, owned by us) |
| `src/stores/`              | zustand stores                                    |
| `src/lib/ipc.ts`           | Typed IPC wrappers                                |
| `src/types/`               | TS mirrors of Rust DTOs                           |

## Data locations (Windows)

| What               | Where                                                    |
| ------------------ | -------------------------------------------------------- |
| Logs               | `%APPDATA%\NexTerm\logs` (daily rotation, 14 files kept) |
| Database (Phase 3) | app data dir, decided in Phase 3                         |
| WebView2 profile   | `%LOCALAPPDATA%\com.nexterm.app` (managed by Tauri)      |
