//! Input validation for SSH connection parameters.
//!
//! Everything arriving over IPC is untrusted. These checks run in Rust no
//! matter what the frontend already validated.

use crate::error::{AppError, AppResult};
use crate::session::protocol::TerminalSize;

const MAX_HOST_LEN: usize = 253; // DNS name limit
const MAX_USERNAME_LEN: usize = 256;
const MAX_PASSWORD_LEN: usize = 1024;
const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;

/// Validate and normalise a host name or IP address.
///
/// Accepts DNS names, IPv4 and IPv6 literals (optionally wrapped in `[...]`).
pub fn host(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    let host = trimmed
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(trimmed);

    if host.is_empty() {
        return Err(AppError::Validation("Host is required.".into()));
    }
    if host.len() > MAX_HOST_LEN {
        return Err(AppError::Validation("Host is too long.".into()));
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':');
    if !host.chars().all(allowed) {
        return Err(AppError::Validation(
            "Host may only contain letters, digits, '.', '-', '_' and ':'.".into(),
        ));
    }
    if host.starts_with('-') {
        return Err(AppError::Validation("Host must not start with '-'.".into()));
    }
    Ok(host.to_owned())
}

/// Validate a TCP port (1-65535).
pub fn port(raw: u32) -> AppResult<u16> {
    match u16::try_from(raw) {
        Ok(p) if p != 0 => Ok(p),
        _ => Err(AppError::Validation(
            "Port must be between 1 and 65535.".into(),
        )),
    }
}

/// Validate a remote username.
pub fn username(raw: &str) -> AppResult<String> {
    if raw.trim().is_empty() {
        return Err(AppError::Validation("Username is required.".into()));
    }
    if raw.len() > MAX_USERNAME_LEN {
        return Err(AppError::Validation("Username is too long.".into()));
    }
    if raw != raw.trim() || raw.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "Username must not contain spaces at the ends or control characters.".into(),
        ));
    }
    Ok(raw.to_owned())
}

/// Validate a password's size (content is never inspected or echoed).
pub fn password_len(len: usize) -> AppResult<()> {
    if len > MAX_PASSWORD_LEN {
        return Err(AppError::Validation("Password is too long.".into()));
    }
    Ok(())
}

/// Validate terminal dimensions.
pub fn terminal_size(cols: u16, rows: u16) -> AppResult<TerminalSize> {
    if !(1..=MAX_COLS).contains(&cols) || !(1..=MAX_ROWS).contains(&rows) {
        return Err(AppError::Validation(format!(
            "Terminal size must be 1-{MAX_COLS} columns and 1-{MAX_ROWS} rows."
        )));
    }
    Ok(TerminalSize { cols, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_hosts() {
        assert_eq!(host("example.com").unwrap(), "example.com");
        assert_eq!(host("  10.0.0.5 ").unwrap(), "10.0.0.5");
        assert_eq!(host("[::1]").unwrap(), "::1");
        assert_eq!(host("fe80::1").unwrap(), "fe80::1");
        assert_eq!(host("my_host-01").unwrap(), "my_host-01");
    }

    #[test]
    fn rejects_bad_hosts() {
        for bad in [
            "",
            "   ",
            "user@host",
            "host name",
            "host;rm -rf /",
            "-oProxyCommand=x",
            "a/b",
            "ho\nst",
        ] {
            assert!(host(bad).is_err(), "should reject {bad:?}");
        }
        assert!(host(&"a".repeat(254)).is_err());
    }

    #[test]
    fn port_range() {
        assert_eq!(port(22).unwrap(), 22);
        assert_eq!(port(65535).unwrap(), 65535);
        assert!(port(0).is_err());
        assert!(port(65536).is_err());
    }

    #[test]
    fn usernames() {
        assert_eq!(username("root").unwrap(), "root");
        assert_eq!(username(r"CORP\alice").unwrap(), r"CORP\alice");
        assert!(username("").is_err());
        assert!(username(" root").is_err());
        assert!(username("ro\u{0}ot").is_err());
        assert!(username(&"u".repeat(257)).is_err());
    }

    #[test]
    fn password_length() {
        assert!(password_len(0).is_ok());
        assert!(password_len(1024).is_ok());
        assert!(password_len(1025).is_err());
    }

    #[test]
    fn terminal_sizes() {
        assert_eq!(
            terminal_size(80, 24).unwrap(),
            TerminalSize { cols: 80, rows: 24 }
        );
        assert!(terminal_size(0, 24).is_err());
        assert!(terminal_size(80, 0).is_err());
        assert!(terminal_size(1001, 24).is_err());
    }
}
