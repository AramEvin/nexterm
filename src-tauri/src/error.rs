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

    /// Could not establish or keep a connection. The message is user-facing
    /// and must not contain secrets.
    #[error("connection failed: {0}")]
    Connection(String),

    /// The server rejected the supplied credentials.
    #[error("authentication failed")]
    AuthFailed,

    /// A saved credential is needed but the vault is locked (UI shows the unlock screen).
    #[error("vault is locked")]
    VaultLocked,

    /// The master password did not decrypt the vault.
    #[error("wrong master password")]
    WrongMasterPassword,

    /// The server's host key differs from the stored one: possible MITM.
    /// The message names the host and both fingerprints (no secrets).
    #[error("host key changed: {0}")]
    HostKeyChanged(String),

    /// The user declined an unknown host key.
    #[error("host key rejected by user")]
    HostKeyRejected,

    /// The connection needs input the request didn't carry (e.g. a password
    /// for a session without a saved credential). The UI asks and retries.
    #[error("credentials required: {0}")]
    CredentialsRequired(String),

    /// The user cancelled an interactive step (2FA prompt, dialog).
    #[error("cancelled")]
    Cancelled,

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
            Self::Connection(_) => "CONNECTION",
            Self::AuthFailed => "AUTH_FAILED",
            Self::VaultLocked => "VAULT_LOCKED",
            Self::WrongMasterPassword => "WRONG_MASTER_PASSWORD",
            Self::HostKeyChanged(_) => "HOST_KEY_CHANGED",
            Self::HostKeyRejected => "HOST_KEY_REJECTED",
            Self::CredentialsRequired(_) => "CREDENTIALS_REQUIRED",
            Self::Cancelled => "CANCELLED",
            Self::Io(_) => "IO",
            Self::Internal(_) => "INTERNAL",
        }
    }

    /// Message that is safe to show to the user.
    pub fn user_message(&self) -> String {
        match self {
            Self::Validation(msg) => msg.clone(),
            Self::NotFound(what) => format!("{what} was not found."),
            Self::Connection(msg) => msg.clone(),
            Self::VaultLocked => "The vault is locked. Unlock it to use saved credentials.".into(),
            Self::WrongMasterPassword => "Wrong master password.".into(),
            Self::HostKeyChanged(msg) | Self::CredentialsRequired(msg) => msg.clone(),
            Self::HostKeyRejected => {
                "Connection cancelled: the server's host key was not accepted.".into()
            }
            Self::Cancelled => "Cancelled.".into(),
            Self::AuthFailed => "Authentication failed. Check the username and password.".into(),
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
