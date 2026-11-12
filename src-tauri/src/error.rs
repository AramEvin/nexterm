//! Application-wide error type returned by every IPC command.
//!
//! Design rules:
//! - Every variant maps to a stable, machine-readable `code` the frontend can
//!   switch on, plus a **user-safe** `message`.
//! - Internal details (OS error text, file paths, library errors) are logged
//!   via `tracing` but are never sent across the IPC boundary.
//! - Secrets must never be placed into an `AppError`.

use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Result alias used by all IPC commands.
pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Input from the frontend failed validation. The message is shown to the
    /// user, so it must describe *what* is wrong without echoing secrets.
    #[error("validation failed: {0}")]
    Validation(String),

    /// A requested entity (session, tab, credential, ...) does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// Filesystem / OS I/O failure. Details are logged, not exposed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Anything unexpected. Details are logged, not exposed.
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// Stable error code for the frontend (part of the IPC contract).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "VALIDATION",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Io(_) => "IO",
            Self::Internal(_) => "INTERNAL",
        }
    }

    /// Message that is safe to show to the user.
    pub fn user_message(&self) -> String {
        match self {
            Self::Validation(msg) => msg.clone(),
            Self::NotFound(what) => format!("{what} was not found."),
            Self::Io(_) => "A file or system operation failed. See the log for details.".into(),
            Self::Internal(_) => "An unexpected error occurred. See the log for details.".into(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Log the full (internal) error once, at the moment it crosses the IPC boundary.
        tracing::warn!(code = self.code(), error = %self, "command failed");

        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.user_message())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_code_and_safe_message() {
        let err = AppError::Validation("Port must be between 1 and 65535.".into());
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "VALIDATION");
        assert_eq!(json["message"], "Port must be between 1 and 65535.");
    }

    #[test]
    fn io_errors_do_not_leak_details() {
        let io = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            r"C:\Users\secret-user\private.key",
        );
        let json = serde_json::to_value(AppError::from(io)).unwrap();
        assert_eq!(json["code"], "IO");
        assert!(!json["message"].as_str().unwrap().contains("secret-user"));
    }

    #[test]
    fn internal_errors_do_not_leak_details() {
        let json = serde_json::to_value(AppError::Internal("db at /tmp/x failed".into())).unwrap();
        assert!(!json["message"].as_str().unwrap().contains("/tmp/x"));
    }
}
