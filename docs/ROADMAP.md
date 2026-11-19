# NexTerm roadmap (v2, agreed 2026-11-14)

The original 8-phase plan grew with these requests: saved credentials per
connection, many protocols, paste features, split panes, a WinSCP-style file
manager with server-to-server transfer, and a live resource bar. Order is
"safety first": nothing stores a password before the encrypted vault exists.

| Phase | Version | Scope                                                                                                                                                                                                                                                                                                                          | Status |
| ----- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ |
| 0     | 0.1     | Scaffold, hardened shell, CI                                                                                                                                                                                                                                                                                                   | Done   |
| 1     | 0.2     | SSH terminal (password auth)                                                                                                                                                                                                                                                                                                   | Done   |
| 2     | 0.3     | Tabs, reconnect, duplicate                                                                                                                                                                                                                                                                                                     | Done   |
| 3     | 0.4     | **Session manager**: nested folders, saved connections (name, protocol, host, port, username, colour, notes, tags, favourite), drag & drop, instant search (500+), context menu, double-click to connect. SQLite with migrations. Usernames saved; passwords not yet.                                                          | Done   |
| 4     | 0.5     | **Credential vault**: saved passwords, private keys, passphrases (Argon2id + XChaCha20-Poly1305), credentials reusable across sessions and settable as a folder default, unlock screen, auto-lock, "remember on this device". Key / agent / keyboard-interactive (2FA) auth. Host-key verification (TOFU, block changed keys). | Done   |
| 5     | 0.6     | **WinSCP-style file manager**: SFTP side panel next to each SSH tab, plus a two-pane file-manager tab (local ↔ remote and **remote ↔ remote**). Transfer queue with progress, speed, cancel, retry; >4 GB streaming; drag & drop from Explorer; rename, delete, mkdir, chmod. FTP / FTPS added here.                           | Done   |
| 6     | 0.7     | **Split panes + resource bar + paste**: split a tab horizontally/vertically into several terminals; live CPU / RAM / disk (and load, uptime) bar under each SSH terminal; **multi-exec** (type into many terminals at once); **paste safety check** for multi-line or large pastes.                                            | Done   |
| 7     | 0.8     | Jump hosts (ProxyJump, chainable); import from MobaXterm and PuTTY.                                                                                                                                                                                                                                                            | Next   |
| 8     | 0.9     | **Terminal protocols**: Telnet, Serial (COM port), local terminal (PowerShell, CMD, WSL).                                                                                                                                                                                                                                      |        |
| 9     | 0.10    | **Graphical protocols**: VNC and RDP (design decision record first).                                                                                                                                                                                                                                                           |        |
| 10    | 1.0     | Settings & themes, keyboard shortcuts, command palette, single instance, window state, signed auto-update, MSI + NSIS release.                                                                                                                                                                                                 |        |

## Design notes for the new features

**Saved passwords (Phase 4).** Stored only in the encrypted vault, never in the
session database. A session references a `credential_id`; one credential can
be shared by many sessions or set as a folder default ("all servers in _Prod_
use this account").

**Server-to-server transfer (Phase 5).** Default is **relay**: NexTerm streams
from server A to server B through your PC. Nothing touches your disk, and it
works even when A and B can't reach each other. The cost is that the data flows
through your connection. A later optional **direct** mode (A pushes to B) would
need credentials or agent forwarding on server A, so it stays off by default.

**Resource bar (Phase 6).** Agentless: nothing is installed on the server. A
separate SSH exec channel on the same connection reads `/proc/stat`,
`/proc/meminfo`, `/proc/loadavg` and `df` every 2–3 s. This doesn't touch your
shell or its history. Linux first; other systems show "not supported". It can be
turned off per session. (Implemented in 0.7.0: every 3 s, only while the
terminal is visible; a split tab shows one bar.)

**Multi-exec (Phase 6).** Typing into many servers at once is powerful and
dangerous, so it's an explicit mode: tabs in multi-exec get a coloured border,
and turning it on needs a confirmation. (Implemented in 0.7.0: you pick the
terminals; only real keystrokes and reviewed pastes are copied, never the
terminal's automatic replies.)

**Plaintext protocols (Phases 5, 8).** Telnet and plain FTP send passwords
unencrypted. NexTerm shows a visible "unencrypted" badge and suggests SSH/SFTP
or FTPS.

**RDP / VNC (Phase 9).** The biggest item. VNC can be implemented in-app. For
RDP the options are embedding FreeRDP (Apache-2.0) or, as an interim, launching
Windows' own `mstsc.exe` with the saved settings. Decided in an ADR at the start
of Phase 9.

**Phase 3 prepares for all of this:** each saved session has a `protocol` field
and protocol-specific options, so new protocols don't need a database rewrite.
