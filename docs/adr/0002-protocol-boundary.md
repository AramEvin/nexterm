# ADR 0002: Protocol boundary for future RDP / VNC / serial support

- Status: Accepted (Phase 1)
- Date: 2026-11-12

## Context

v1 ships SSH only. RDP, VNC, serial and local shells are planned. Adding one must
not require rewriting the session manager, tabs or IPC.

## Decision (direction)

- `session::protocol::ProtocolKind` enumerates session types (`ssh` today). It is
  the value stored in SQLite and sent over IPC.
- In Phase 1, a `Protocol`-style trait will be extracted from the working SSH
  implementation, covering the operations every _terminal-like_ protocol shares:
  open (returns a session handle + output channel), write, resize, close, status.
- Graphical protocols (RDP/VNC) have a different data plane (frame buffers, not
  byte streams). They will get a sibling trait rather than being forced into the
  terminal one. The shared part is lifecycle + status reporting in `SessionManager`.

## Phase 1 implementation

`src-tauri/src/session/protocol.rs`:

- `TerminalProtocol::open(params, size, sink) -> SessionHandle`: connect, auth,
  start a shell. `SshProtocol` is the first implementation.
- `SessionHandle { kind, commands: mpsc::Sender<SessionCommand>, task }`: the app
  talks to any protocol through `Write(bytes)`, `Resize(size)` and `Close`.
- `OutputSink`: the protocol pushes raw bytes and `SessionStatus` into it. The
  Tauri implementation forwards to two IPC channels. Tests use an in-memory sink,
  so protocols are tested headless against real servers.
- `SessionManager` stores only the command sender per session id, plus a watcher
  task that removes ended sessions and turns a panic into a "session crashed"
  status (crash isolation, unit-tested).

## Why the trait wasn't defined in Phase 0

Designing the trait before one real implementation exists tends to produce the
wrong shape. Phase 0 reserves the boundary (module + enum + this ADR). Phase 1
fills it in from working code.
