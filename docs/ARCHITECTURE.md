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
4. **Streaming:** each session gets two channels from the frontend:
   `Channel<tauri::ipc::Response>` for terminal output (raw bytes, delivered to JS as
   an `ArrayBuffer`, not a JSON number array) and `Channel<SessionStatus>` for
   lifecycle events. Input: `ssh_write`; resize: `ssh_resize`; close: `ssh_disconnect`.
5. **Ordered input:** IPC calls run concurrently in Rust, so per-keystroke writes
   could be reordered. `TerminalController` keeps one `ssh_write` in flight and
   batches keys typed meanwhile, which preserves order and cuts IPC volume.
6. **Isolation:** each live session runs in its own tokio task.
7. **Protocols:** SSH is the first implementation behind the protocol boundary in
   `session/protocol.rs` (see ADR 0002).

## Tabs and sessions (Phase 2)

| Concept                                   | Lives in                   | Lifetime                                |
| ----------------------------------------- | -------------------------- | --------------------------------------- |
| Tab (`SessionTab`)                        | `src/stores/session.ts`    | Until the user closes it                |
| `TerminalController` (xterm + scrollback) | one per tab                | Same as the tab; survives reconnects    |
| `connectionId` → params incl. password    | Rust `ssh::SshConnections` | Same as the tab (`ssh_forget` on close) |
| `sessionId` → live connection             | Rust `SessionManager`      | One connection; new id on reconnect     |

A "closed" status is applied only if it belongs to the tab's _current_ session, so
a late message from a previous session can't close a fresh reconnect. Only the
visible terminal holds a WebGL context (Chromium caps live contexts at about 16).
Tab reordering uses pointer events, because HTML5 drag-and-drop is blocked in
WebView2 while Tauri's native file drop is on.

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

## Saved sessions (Phase 3)

- **Storage:** SQLite (`rusqlite`, bundled), one file per user. Schema in
  `src-tauri/migrations/NNNN_*.sql`, applied in order inside a transaction and
  tracked with `PRAGMA user_version`. A database from a newer NexTerm is refused
  rather than modified.
- **Tables:** `folders` (nested via `parent_id`, cascade delete), `sessions`
  (with `protocol`, `options` JSON, `credential_id` for Phase 4, `jump_host_id`
  for Phase 7), `session_tags`, `settings`.
- **Rust:** `storage::{folders, sessions}` are plain functions over a
  `Connection`. Commands run them on the blocking pool (`Database::run`). If the
  file can't be opened, the app still starts and the sidebar shows the error.
- **Frontend:** `stores/library.ts` keeps a copy of everything. Every mutation goes
  to Rust and then reloads, so the UI never shows state the database doesn't have.
  Tree building and search are pure functions in `lib/library.ts`, filtered in
  memory and deferred (`useDeferredValue`), which keeps typing instant with
  thousands of sessions. The tree uses one shared context menu, and search renders
  at most 300 rows.

## Vault, credentials and host keys (Phase 4)

```
UI ──ssh_connect_saved(sessionId)──▶ commands::ssh::resolve
                                       ├─ storage::sessions::get
                                       ├─ vault::credentials::resolve_for_session (own → folder chain)
                                       └─ vault::credentials::load_secret ──▶ Vault::with_key (VAULT_LOCKED if locked)
                                     ssh::client::open
                                       ├─ ClientHandler::check_server_key ──▶ known_hosts::check
                                       │      unknown ──▶ PromptBroker.ask ──▶ UI dialog ──▶ ssh_prompt_respond
                                       └─ authenticate (password | publickey | agent | keyboard-interactive)
```

- **`vault/`:** `crypto` (Argon2id, XChaCha20-Poly1305 seal/open), `keychain` (OS store
  behind a trait; in-memory for tests), `mod.rs` (setup/unlock/lock/auto-lock state
  machine), `credentials` (encrypted CRUD; metadata only over IPC).
- **`ssh/prompt.rs`:** the `PromptBroker` matches UI answers to waiting
  connections by random id, with a 3-minute timeout. Each connection supplies a
  third IPC channel (`onPrompt`).
- **`ssh/known_hosts.rs`:** NexTerm's table plus a read-only `~/.ssh/known_hosts`.
- **`ssh/connections.rs`:** `ConnectSpec::{Direct, Saved}`, so reconnects of saved
  sessions re-resolve through the vault.
- **Frontend:** `stores/vault.ts` (status, credentials, activity pings,
  `vault-locked` event), `stores/prompts.ts` (prompt queue), and `lib/connect.ts`
  (one place that turns error codes into dialogs: unlock + retry, changed-key
  warning, ask for credentials).

## File manager (Phase 5)

```
FilesView (2 × FilePane, TransferQueue)
   │ fs_list / fs_mkdir / fs_rename / fs_delete / fs_chmod  (Endpoint = local | remote{fsId})
   │ sftp_open_saved / sftp_open_connection ──▶ ssh::connect ──▶ RemoteFs (SFTP subsystem)
   │ transfer_start(request, onEvent) ──▶ Transfers (≤3 jobs) ──▶ Run: plan ▸ copy
   ▼
sftp::fs::Fs { Local | Remote(Arc<RemoteFs>) }  ── one read/write interface for both sides
```

- **`sftp/remote.rs`:** one SFTP session per remote pane, on its own SSH
  connection (`ssh::connect` is shared with terminals). Errors become short
  `FILE` / `CONNECTION` messages.
- **`sftp/local.rs`:** the local disk (blocking, run on the blocking pool); "" =
  the Windows drive list.
- **`sftp/fs.rs`:** `Fs`, so local↔remote, remote↔local and remote↔remote
  (relay) share one transfer path.
- **`sftp/transfer.rs`:** the queue. Plan (expand folders, check names,
  conflicts, totals), then stream 256 KiB chunks with progress every ~150 ms.
- **`sftp/paths.rs`:** validation, including names that come from the server.
- **`sftp/registry.rs`:** open SFTP sessions by opaque id (max 64).
- **Frontend:** `stores/files.ts` (tabs and panes), `stores/transfers.ts`
  (queue rows, retry), `lib/fileActions.ts` (one implementation of each action
  for menus, keys and drag & drop), `components/files/*`. File-manager tabs
  share the tab bar with terminals (`registerExternalTab`).

## Data locations (Windows)

| What             | Where                                                    |
| ---------------- | -------------------------------------------------------- |
| Logs             | `%APPDATA%\NexTerm\logs` (daily rotation, 14 files kept) |
| Database         | `%APPDATA%\NexTerm\nexterm.db` (SQLite, WAL mode)        |
| WebView2 profile | `%LOCALAPPDATA%\com.nexterm.app` (managed by Tauri)      |
