//! Structured, rotating logs via `tracing`.
//!
//! - Location: `<data dir>/NexTerm/logs` (on Windows: `%APPDATA%\NexTerm\logs`).
//! - Rotation: daily, keeping the 14 most recent files.
//! - Level: `info` by default; override with the `NEXTERM_LOG` environment
//!   variable using `tracing` filter syntax, e.g. `NEXTERM_LOG=debug`.
//!
//! **Never log secrets or terminal content.** Log *events* (connect, auth
//! method used, disconnect, host-key decision), not *data*.

use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

const LOG_ENV_VAR: &str = "NEXTERM_LOG";
const LOG_RETENTION_FILES: usize = 14;

/// Keeps the background log writer alive. Dropping it flushes and stops
/// file logging, so it is stored in Tauri's managed state for the app lifetime.
pub struct LogGuard {
    _guard: WorkerGuard,
    pub log_dir: PathBuf,
}

/// Resolve the log directory under the OS per-user data directory.
pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("NexTerm").join("logs")
}

/// Initialise the global tracing subscriber (file + stderr in debug builds).
pub fn init(log_dir: PathBuf) -> anyhow::Result<LogGuard> {
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("nexterm")
        .filename_suffix("log")
        .max_log_files(LOG_RETENTION_FILES)
        .build(&log_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_env(LOG_ENV_VAR)
        .unwrap_or_else(|_| EnvFilter::new("info,tao=warn,wry=warn"));

    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true);

    // Console output only for development builds.
    let console_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()?;

    install_panic_hook();

    Ok(LogGuard {
        _guard: guard,
        log_dir,
    })
}

/// Route panics into the log so crashes leave an audit trail.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "application panicked");
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_is_under_nexterm() {
        let dir = log_dir(Path::new("/data"));
        assert!(dir.ends_with(Path::new("NexTerm").join("logs")));
    }
}
