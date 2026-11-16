//! The user's own disks. Every function here is blocking; callers run them on
//! the blocking pool (`tokio::task::spawn_blocking`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{EntryKind, FileEntry, Listing, MAX_LISTING, sort_entries};
use crate::error::{AppError, AppResult};

/// The path that means "list the drives" (Windows) / the root (elsewhere).
pub const ROOT: &str = "";

/// A friendly, user-facing message for a local I/O error on `path`.
pub fn io_error(err: &io::Error, path: &Path) -> AppError {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    tracing::debug!(error = %err, "local file operation failed");
    AppError::FileOp(match err.kind() {
        io::ErrorKind::NotFound => format!("\"{name}\" does not exist."),
        io::ErrorKind::PermissionDenied => format!("Permission denied: \"{name}\"."),
        io::ErrorKind::AlreadyExists => format!("\"{name}\" already exists."),
        io::ErrorKind::DirectoryNotEmpty => format!("The folder \"{name}\" is not empty."),
        io::ErrorKind::StorageFull => "The disk is full.".into(),
        io::ErrorKind::ReadOnlyFilesystem => "The disk is read-only.".into(),
        _ => format!("The operation on \"{name}\" failed: {err}"),
    })
}

pub fn home() -> String {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ROOT.to_owned())
}

pub fn list(path: &str) -> AppResult<Listing> {
    if path == ROOT {
        return Ok(roots());
    }
    let dir = super::paths::local_path(path)?;
    let read = fs::read_dir(&dir).map_err(|e| io_error(&e, &dir))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in read {
        let Ok(item) = item else { continue };
        if entries.len() >= MAX_LISTING {
            truncated = true;
            break;
        }
        let path = item.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        entries.push(entry(
            item.file_name().to_string_lossy().into_owned(),
            &path,
            &meta,
        ));
    }
    sort_entries(&mut entries);
    let parent = match dir.parent() {
        Some(p) => Some(p.display().to_string()),
        // At "C:\" go up to the drive list; at "/" there is nothing above.
        None if cfg!(windows) => Some(ROOT.to_owned()),
        None => None,
    };
    Ok(Listing {
        path: dir.display().to_string(),
        parent,
        entries,
        truncated,
    })
}

fn entry(name: String, path: &Path, meta: &fs::Metadata) -> FileEntry {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let target = if kind == EntryKind::Symlink {
        fs::metadata(path).ok()
    } else {
        None
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());
    FileEntry {
        hidden: is_hidden(&name, meta),
        path: path.display().to_string(),
        name,
        kind,
        link_to_dir: target.as_ref().is_some_and(fs::Metadata::is_dir),
        size: target.as_ref().map_or(meta.len(), fs::Metadata::len),
        modified,
        permissions: permissions(meta),
        owner: None,
        group: None,
    }
}

#[cfg(windows)]
fn is_hidden(name: &str, meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    name.starts_with('.') || meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &fs::Metadata) -> bool {
    name.starts_with('.')
}

#[cfg(unix)]
fn permissions(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn permissions(_meta: &fs::Metadata) -> Option<u32> {
    None
}

/// Windows: one entry per existing drive letter. Elsewhere: just "/".
fn roots() -> Listing {
    let mut entries = Vec::new();
    if cfg!(windows) {
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", char::from(letter));
            if Path::new(&root).exists() {
                entries.push(FileEntry {
                    name: format!("{}:", char::from(letter)),
                    path: root,
                    kind: EntryKind::Dir,
                    link_to_dir: false,
                    size: 0,
                    modified: None,
                    permissions: None,
                    owner: None,
                    group: None,
                    hidden: false,
                });
            }
        }
    } else {
        entries.push(FileEntry {
            name: "/".into(),
            path: "/".into(),
            kind: EntryKind::Dir,
            link_to_dir: false,
            size: 0,
            modified: None,
            permissions: None,
            owner: None,
            group: None,
            hidden: false,
        });
    }
    Listing {
        path: ROOT.to_owned(),
        parent: None,
        entries,
        truncated: false,
    }
}

pub fn mkdir(path: &Path) -> AppResult<()> {
    fs::create_dir(path).map_err(|e| io_error(&e, path))
}

pub fn rename(from: &Path, to: &Path) -> AppResult<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io_error(&io::Error::from(io::ErrorKind::AlreadyExists), to));
    }
    fs::rename(from, to).map_err(|e| io_error(&e, from))
}

/// Delete a file, link or folder tree. `remove_dir_all` does not follow
/// symlinks (std fixed CVE-2022-21658), so a link to `C:\` only removes the link.
pub fn remove(path: &Path) -> AppResult<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_error(&e, path))?;
    let result = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else if meta.file_type().is_symlink() && fs::remove_file(path).is_err() {
        // Windows directory symlinks / junctions are removed as directories.
        fs::remove_dir(path)
    } else if meta.file_type().is_symlink() {
        Ok(())
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| io_error(&e, path))
}

#[cfg(unix)]
pub fn chmod(path: &Path, mode: u32) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
        .map_err(|e| io_error(&e, path))
}

#[cfg(not(unix))]
pub fn chmod(_path: &Path, _mode: u32) -> AppResult<()> {
    Err(AppError::Validation(
        "Permissions can only be changed on the server side.".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_mkdir_rename_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        mkdir(&base.join("sub")).unwrap();
        fs::write(base.join("b.txt"), b"hello").unwrap();
        fs::write(base.join(".hidden"), b"").unwrap();
        fs::write(base.join("sub").join("inner"), b"x").unwrap();

        let listing = list(&base.display().to_string()).unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", ".hidden", "b.txt"]); // folders first
        let b = listing.entries.iter().find(|e| e.name == "b.txt").unwrap();
        assert_eq!(b.size, 5);
        assert!(
            listing
                .entries
                .iter()
                .any(|e| e.name == ".hidden" && e.hidden)
        );

        assert_eq!(
            rename(&base.join("b.txt"), &base.join("sub"))
                .unwrap_err()
                .code(),
            "FILE"
        );
        rename(&base.join("b.txt"), &base.join("c.txt")).unwrap();
        remove(&base.join("sub")).unwrap();
        assert!(!base.join("sub").exists());
        assert_eq!(remove(&base.join("nope")).unwrap_err().code(), "FILE");
    }

    #[cfg(unix)]
    #[test]
    fn remove_does_not_follow_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let dir = tmp.path().join("dir");
        fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        remove(&dir).unwrap();
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn root_listing() {
        let l = list(ROOT).unwrap();
        assert!(!l.entries.is_empty());
        assert!(l.parent.is_none());
    }
}
