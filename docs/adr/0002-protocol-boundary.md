# ADR 0002: Protocol boundary for future RDP / VNC / serial support

- Status: Proposed (finalised in Phase 1)
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

## Why not define the full trait now

Designing the trait before one real implementation exists tends to produce the
wrong shape. Phase 0 reserves the boundary (module + enum + this ADR). Phase 1
fills it in from working code.
