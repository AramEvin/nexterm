//! Local SQLite storage: saved sessions, folders, tags, settings.
//!
//! - One database file: `<data dir>/NexTerm/nexterm.db`
//!   (Windows: `%APPDATA%\NexTerm\nexterm.db`).
//! - Schema changes are versioned migrations in `src-tauri/migrations/`, applied
//!   in order inside a transaction and tracked with `PRAGMA user_version`.
//! - **No secrets are ever stored here** (enforced by a test that greps the file).
//!
//! rusqlite's `Connection` is not thread-safe, so it sits behind a mutex. All
//! queries are short; commands run them on Tokio's blocking pool so the async
//! runtime and the UI thread never wait on disk I/O.

pub mod folders;
pub mod models;
pub mod sessions;
mod validate;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// Migrations in order. Index `i` brings the schema from version `i` to `i + 1`.
/// Never edit a released migration; add a new one.
const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_initial.sql")];

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        // Details go to the log via AppError's Serialize impl; the user sees a
        // generic message.
        AppError::Internal(format!("database error: {err}"))
    }
}

/// Shared handle to the database. Cheap to clone.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    path: Option<PathBuf>,
}

/// Default database location under the OS per-user data directory.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("NexTerm").join("nexterm.db")
}

/// Current time as Unix epoch milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl Database {
    /// Open (or create) the database file and apply pending migrations.
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, Some(path.to_path_buf()))
    }

    /// In-memory database (tests).
    pub fn open_in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(mut conn: Connection, path: Option<PathBuf>) -> AppResult<Self> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA secure_delete = ON;",
        )?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run `f` with exclusive access to the connection (synchronously).
    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> AppResult<T>) -> AppResult<T> {
        let mut conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut conn)
    }

    /// Run `f` on the blocking thread pool (use from async commands).
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    ) -> AppResult<T> {
        let db = self.clone();
        tauri::async_runtime::spawn_blocking(move || db.with(f))
            .await
            .map_err(|e| AppError::Internal(format!("database task failed: {e}")))?
    }
}

/// Apply all migrations newer than the file's `user_version`.
fn migrate(conn: &mut Connection) -> AppResult<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let latest = i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX);
    if current > latest {
        // Opened by an older NexTerm after a newer one upgraded the schema.
        // Refuse rather than risk corrupting data.
        return Err(AppError::Validation(format!(
            "The session database was created by a newer NexTerm version (schema {current}, \
             this build supports {latest}). Please update NexTerm."
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        let version = i64::try_from(index + 1).unwrap_or(i64::MAX);
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "database migrated");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_database_to_latest() {
        let db = Database::open_in_memory().unwrap();
        let version: i64 = db
            .with(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn migration_is_idempotent_on_reopen() {
        let dir = std::env::temp_dir().join(format!("nexterm-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.db");
        drop(Database::open(&path).unwrap());
        // Reopening must not re-run migrations (would fail on CREATE TABLE).
        drop(Database::open(&path).unwrap());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = std::env::temp_dir().join(format!("nexterm-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.db");
        drop(Database::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 999)
            .unwrap();
        let err = Database::open(&path).err().unwrap();
        assert_eq!(err.code(), "VALIDATION");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Database::open_in_memory().unwrap();
        let on: i64 = db
            .with(|c| Ok(c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(on, 1);
    }
}
