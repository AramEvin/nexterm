//! Local terminal (Phase 8): PowerShell, Command Prompt or WSL on Windows
//! (ConPTY), the login shell elsewhere. Uses `portable-pty`.
//!
//! Each shell maps to a fixed executable path; nothing is looked up on PATH
//! and the UI can't name a program, so a planted `cmd.exe` in the working
//! directory or a crafted session can't run something else.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::session::blocking::{spawn_reader, spawn_writer};
use crate::session::monitor::ResourceEvent;
use crate::session::options::{LocalOptions, LocalShell};
use crate::session::protocol::{
    OutputSink, ProtocolKind, SessionCommand, SessionHandle, SessionStatus, TerminalProtocol,
    TerminalSize,
};

pub struct LocalProtocol;

/// The program and arguments for a shell.
pub fn command(o: &LocalOptions) -> AppResult<(PathBuf, Vec<String>)> {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let sys32 = root.join("System32");
        let found = |p: PathBuf, what: &str| {
            if p.is_file() {
                Ok(p)
            } else {
                Err(AppError::Validation(format!(
                    "{what} isn't installed on this PC."
                )))
            }
        };
        match o.shell {
            LocalShell::Powershell => Ok((
                found(
                    sys32.join(r"WindowsPowerShell\v1.0\powershell.exe"),
                    "Windows PowerShell",
                )?,
                vec!["-NoLogo".into()],
            )),
            LocalShell::Pwsh => {
                let pf = std::env::var_os("ProgramFiles")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
                Ok((
                    found(pf.join(r"PowerShell\7\pwsh.exe"), "PowerShell 7")?,
                    vec!["-NoLogo".into()],
                ))
            }
            LocalShell::Cmd => Ok((found(sys32.join("cmd.exe"), "Command Prompt")?, Vec::new())),
            LocalShell::Wsl => {
                let mut args = vec!["--cd".to_owned(), "~".to_owned()];
                if !o.wsl_distro.is_empty() {
                    args.extend(["-d".to_owned(), o.wsl_distro.clone()]);
                }
                Ok((found(sys32.join("wsl.exe"), "WSL")?, args))
            }
            LocalShell::Default => {
                Ok((found(sys32.join("cmd.exe"), "Command Prompt")?, Vec::new()))
            }
        }
    }
    #[cfg(not(windows))]
    {
        match o.shell {
            LocalShell::Default => {
                // $SHELL only if it's an absolute path to an existing file.
                let shell = std::env::var_os("SHELL")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute() && p.is_file())
                    .unwrap_or_else(|| PathBuf::from("/bin/sh"));
                Ok((shell, Vec::new()))
            }
            other => Err(AppError::Validation(format!(
                "{} is only available on Windows.",
                other.label()
            ))),
        }
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

fn pty_size(s: TerminalSize) -> PtySize {
    PtySize {
        rows: s.rows,
        cols: s.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl TerminalProtocol for LocalProtocol {
    type Params = LocalOptions;

    async fn open(
        params: LocalOptions,
        size: TerminalSize,
        sink: Arc<dyn OutputSink>,
    ) -> AppResult<SessionHandle> {
        let options = params.validated()?;
        let (program, args) = command(&options)?;
        tracing::info!(shell = options.shell.label(), "local terminal starting");
        let started = tokio::task::spawn_blocking(move || -> AppResult<_> {
            let fail = |what: &str| {
                let what = what.to_owned();
                move |e: anyhow::Error| {
                    tracing::warn!(error = %e, %what, "local terminal");
                    AppError::Connection("Could not start the shell.".into())
                }
            };
            let pair = native_pty_system()
                .openpty(pty_size(size))
                .map_err(fail("openpty"))?;
            let mut cmd = CommandBuilder::new(program);
            cmd.args(args);
            if let Some(dir) = home() {
                cmd.cwd(dir);
            }
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
            let child = pair.slave.spawn_command(cmd).map_err(fail("spawn"))?;
            drop(pair.slave);
            let reader = pair.master.try_clone_reader().map_err(fail("reader"))?;
            let writer = pair.master.take_writer().map_err(fail("writer"))?;
            Ok((pair.master, child, reader, writer))
        })
        .await
        .map_err(|_| AppError::Internal("local start task".into()))??;
        let (master, mut child, reader, writer) = started;

        let stop = Arc::new(AtomicBool::new(false));
        let (read_done_tx, mut read_done_rx) = mpsc::channel(1);
        spawn_reader(
            "local",
            reader,
            Arc::clone(&sink),
            Arc::clone(&stop),
            read_done_tx,
        );
        let writer = spawn_writer("local", writer);

        // Wait for the shell to exit on its own thread. On Windows the PTY
        // reader only sees EOF once the master is dropped, so exit is the signal.
        let mut killer = child.clone_killer();
        let (exit_tx, mut exit_rx) = mpsc::channel::<Option<u32>>(1);
        let _ = std::thread::Builder::new()
            .name("nexterm-local-wait".into())
            .spawn(move || {
                let code = child.wait().ok().map(|s| s.exit_code());
                let _ = exit_tx.blocking_send(code);
            });

        let (tx, mut rx) = mpsc::channel(256);
        let label = options.shell.label();
        let task = tokio::spawn(async move {
            let reason = loop {
                tokio::select! {
                    code = exit_rx.recv() => {
                        // Let the last output drain before closing the PTY.
                        let _ = tokio::time::timeout(Duration::from_millis(300), read_done_rx.recv()).await;
                        break match code.flatten() {
                            Some(0) | None => format!("{label} exited."),
                            Some(c) => format!("{label} exited with code {c}."),
                        };
                    }
                    cmd = rx.recv() => match cmd {
                        Some(SessionCommand::Write(bytes)) => {
                            if writer.send(bytes).is_err() {
                                break format!("{label} stopped.");
                            }
                        }
                        Some(SessionCommand::Resize(s)) => {
                            let _ = master.resize(pty_size(s));
                        }
                        Some(SessionCommand::Monitor(Some(m))) => {
                            m.event(ResourceEvent::Unsupported { reason: "The resource bar is for SSH servers.".into() });
                        }
                        Some(SessionCommand::Monitor(None)) => {}
                        Some(SessionCommand::Close) | None => {
                            let _ = killer.kill();
                            break "Closed.".into();
                        }
                    },
                }
            };
            stop.store(true, Ordering::Relaxed);
            drop(writer);
            drop(master);
            sink.status(SessionStatus::Closed { reason });
        });
        Ok(SessionHandle {
            kind: ProtocolKind::Local,
            commands: tx,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shells_map_to_fixed_programs() {
        let o = |shell| LocalOptions {
            shell,
            wsl_distro: String::new(),
        };
        if cfg!(windows) {
            if let Ok((p, _)) = command(&o(LocalShell::Cmd)) {
                assert!(p.ends_with("System32\\cmd.exe") || p.ends_with("System32/cmd.exe"));
            }
        } else {
            let (p, args) = command(&o(LocalShell::Default)).unwrap();
            assert!(p.is_absolute());
            assert!(args.is_empty());
            assert!(command(&o(LocalShell::Wsl)).is_err());
        }
    }
}
