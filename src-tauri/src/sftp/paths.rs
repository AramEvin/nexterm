//! Path validation for the file manager.
//!
//! Two sources of untrusted names:
//! - the frontend (paths and new names it sends over IPC), and
//! - **the server** (file names inside a folder we download recursively).
//!
//! The second one matters most: a malicious server can return a directory
//! entry called `../../AppData/Roaming/...` or `C:\Windows\x`. Every name that
//! becomes part of a local path goes through [`safe_local_name`] first, so a
//! download can only ever write *inside* the folder the user chose.

use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// Longest remote path we accept (Linux PATH_MAX).
pub const MAX_REMOTE_PATH: usize = 4096;
/// Longest local path we accept (Windows extended-length limit).
pub const MAX_LOCAL_PATH: usize = 32_767;
/// Longest single file name (bytes); ext4/NTFS limit.
pub const MAX_NAME: usize = 255;

/// A remote (POSIX) path from the frontend.
pub fn remote_path(raw: &str) -> AppResult<String> {
    if raw.is_empty() {
        return Err(AppError::Validation("The path is empty.".into()));
    }
    if raw.len() > MAX_REMOTE_PATH {
        return Err(AppError::Validation("The path is too long.".into()));
    }
    if raw.contains('\0') {
        return Err(AppError::Validation(
            "The path contains invalid characters.".into(),
        ));
    }
    Ok(raw.to_owned())
}

/// A local path from the frontend. Must be absolute (the empty string means
/// "the list of drives" and is handled by the caller before this).
pub fn local_path(raw: &str) -> AppResult<PathBuf> {
    if raw.len() > MAX_LOCAL_PATH || raw.contains('\0') {
        return Err(AppError::Validation("The path is not valid.".into()));
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(AppError::Validation("Expected a full path.".into()));
    }
    Ok(path)
}

/// A new file/folder name typed by the user (rename, new folder).
pub fn new_name(raw: &str, local: bool) -> AppResult<String> {
    if raw.trim().is_empty() {
        return Err(AppError::Validation("Enter a name.".into()));
    }
    let ok = if local {
        safe_local_name(raw, cfg!(windows))
    } else {
        safe_remote_name(raw)
    };
    if !ok {
        return Err(AppError::Validation(if local && cfg!(windows) {
            "That name is not allowed (no \\ / : * ? \" < > |, and not a reserved name like CON)."
                .into()
        } else {
            "That name is not allowed (no '/', not '.' or '..').".into()
        }));
    }
    Ok(raw.to_owned())
}

/// A single path component that is safe to use on the server.
pub fn safe_remote_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
}

/// Device names Windows reserves in every folder (with or without extension).
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A single path component that is safe to create on the local disk.
///
/// Always rejects separators and `.`/`..` (path traversal). With `windows`,
/// also rejects characters and names Windows can't store or treats specially
/// (`C:` drive prefixes, `NUL`, alternate data streams via `:`, trailing dots).
pub fn safe_local_name(name: &str, windows: bool) -> bool {
    if name.is_empty()
        || name.len() > MAX_NAME
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return false;
    }
    if !windows {
        return true;
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    !WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stem))
}

/// `dir` + `/` + `name` for POSIX paths.
pub fn remote_join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent of a POSIX path (`None` at `/`).
pub fn remote_parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_owned()),
        Some(i) => Some(trimmed[..i].to_owned()),
        None => None,
    }
}

/// Last component of a path in either style.
pub fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
}

/// "report.pdf" -> "report (1).pdf", "report (2).pdf", ... until `taken` says no.
pub fn unique_name(name: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = format!("{stem} ({n}){ext}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{stem} ({}){ext}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_names_are_rejected_everywhere() {
        for bad in ["..", ".", "../x", "a/b", "a\\b", "", "x\0y"] {
            assert!(!safe_local_name(bad, false), "{bad:?}");
            assert!(!safe_local_name(bad, true), "{bad:?}");
        }
        assert!(!safe_remote_name(".."));
        assert!(!safe_remote_name("a/b"));
        assert!(safe_remote_name("a\\b")); // a legal (odd) POSIX name
    }

    #[test]
    fn windows_specific_names() {
        for bad in [
            "CON",
            "con.txt",
            "Lpt1",
            "nul.tar.gz",
            "C:",
            "a:stream",
            "what?",
            "x*",
            "trailing.",
            "trailing ",
            "tab\there",
        ] {
            assert!(!safe_local_name(bad, true), "{bad:?}");
        }
        for ok in [
            "console.log",
            "report (1).pdf",
            ".bashrc",
            "über.txt",
            "COM10",
        ] {
            assert!(safe_local_name(ok, true), "{ok:?}");
        }
        assert!(safe_local_name("a:b", false));
    }

    #[test]
    fn posix_helpers() {
        assert_eq!(remote_join("/", "etc"), "/etc");
        assert_eq!(remote_join("/home/u", "x"), "/home/u/x");
        assert_eq!(remote_join("/home/u/", "x"), "/home/u/x");
        assert_eq!(remote_parent("/home/u"), Some("/home".into()));
        assert_eq!(remote_parent("/home"), Some("/".into()));
        assert_eq!(remote_parent("/"), None);
        assert_eq!(base_name("/home/u/file.txt"), "file.txt");
        assert_eq!(base_name(r"C:\Users\me\dir\"), "dir");
    }

    #[test]
    fn unique_names() {
        let taken = ["a.txt", "a (1).txt", "dir"];
        let is_taken = |n: &str| taken.contains(&n);
        assert_eq!(unique_name("a.txt", is_taken), "a (2).txt");
        assert_eq!(unique_name("dir", is_taken), "dir (1)");
        assert_eq!(unique_name("new.txt", is_taken), "new.txt");
        assert_eq!(unique_name(".bashrc", |n| n == ".bashrc"), ".bashrc (1)");
    }

    #[test]
    fn validation() {
        assert!(remote_path("").is_err());
        assert!(remote_path(&"a".repeat(MAX_REMOTE_PATH + 1)).is_err());
        assert!(local_path("relative/path").is_err());
        assert!(new_name("..", false).is_err());
        assert!(new_name("  ", true).is_err());
        assert!(new_name("ok name", true).is_ok());
    }
}
