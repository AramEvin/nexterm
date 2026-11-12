# ADR 0001: Tauri 2 + Rust + React as the application stack

- Status: Accepted
- Date: 2026-11-12

## Context

NexTerm replaces MobaXterm on Windows 10/11 with a free, fast, low-RAM,
security-focused SSH/SFTP client that can later ship on Linux and macOS.

## Decision

- **Shell:** Tauri 2 (Rust backend, system WebView2 frontend).
- **Backend:** Rust stable, tokio, russh / russh-sftp, rusqlite, RustCrypto
  (argon2, chacha20poly1305), keyring, tracing.
- **Frontend:** React + TypeScript (strict) + Vite, xterm.js, zustand,
  Tailwind CSS v4, shadcn/ui (Radix primitives).

## Alternatives considered

| Option                            | Why not                                                                                               |
| --------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Electron                          | Bundles Chromium + Node: ~150 MB installers, higher idle RAM, bigger attack surface (Node in-process) |
| .NET / WPF                        | Windows-only, contradicts the cross-platform goal                                                     |
| Qt (C++)                          | Memory-unsafe language for a security tool; slower UI iteration                                       |
| Pure-Rust GUI (egui, iced, Slint) | No terminal emulator of xterm.js maturity yet                                                         |

## Consequences

- Small installers (~5–10 MB) and low baseline RAM; WebView2 is shared with the OS.
- Memory-safe networking/crypto code in Rust.
- Two languages and an IPC boundary to maintain; DTOs mirrored manually in `src/types/`.
- The `tauri` crate and `@tauri-apps/*` npm packages must stay on the same
  major.minor version, or the CLI refuses to build.
- Frontend versions at scaffold time: React 19, Vite 8, TypeScript 6, Tailwind 4
  (the spec's "React 18+" is satisfied).
