//! Validation for stored entities. Network-facing fields (host, port, username)
//! reuse the SSH validators so a saved session can always be connected.

use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::ssh::validate as net;

pub const MAX_NAME_LEN: usize = 128;
pub const MAX_FOLDER_NAME_LEN: usize = 64;
pub const MAX_NOTES_LEN: usize = 4000;
pub const MAX_TAGS: usize = 20;
pub const MAX_TAG_LEN: usize = 32;
/// Deepest allowed folder nesting (root = depth 1).
pub const MAX_FOLDER_DEPTH: usize = 12;

fn clean_text(raw: &str, what: &str, max: usize) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(AppError::Validation(format!("{what} is required.")));
    }
    if value.chars().count() > max {
        return Err(AppError::Validation(format!(
            "{what} is too long (max {max} characters)."
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(AppError::Validation(format!(
            "{what} contains invalid characters."
        )));
    }
    Ok(value.to_string())
}

pub fn session_name(raw: &str) -> AppResult<String> {
    clean_text(raw, "Name", MAX_NAME_LEN)
}

pub fn folder_name(raw: &str) -> AppResult<String> {
    clean_text(raw, "Folder name", MAX_FOLDER_NAME_LEN)
}

pub fn host(raw: &str) -> AppResult<String> {
    net::host(raw)
}

pub fn port(raw: u32) -> AppResult<u16> {
    net::port(raw)
}

/// Username may be empty for saved sessions (asked at connect time).
pub fn optional_username(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        Ok(String::new())
    } else {
        net::username(value)
    }
}

pub fn notes(raw: &str) -> AppResult<String> {
    if raw.chars().count() > MAX_NOTES_LEN {
        return Err(AppError::Validation(format!(
            "Notes are too long (max {MAX_NOTES_LEN} characters)."
        )));
    }
    Ok(raw.to_string())
}

/// `#rrggbb` (lower-cased) or none.
pub fn color(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let valid = value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !valid {
        return Err(AppError::Validation(
            "Colour must look like #1a2b3c.".into(),
        ));
    }
    Ok(Some(value.to_ascii_lowercase()))
}

/// Trim, drop empties, de-duplicate case-insensitively, keep first spelling.
pub fn tags(raw: &[String]) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for tag in raw {
        let t = tag.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > MAX_TAG_LEN || t.chars().any(|c| c.is_control() || c == ',') {
            return Err(AppError::Validation(format!(
                "Tag \"{}\" is invalid (max {MAX_TAG_LEN} characters, no commas).",
                t.chars().take(MAX_TAG_LEN).collect::<String>()
            )));
        }
        if !out.iter().any(|e| e.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    if out.len() > MAX_TAGS {
        return Err(AppError::Validation(format!(
            "Too many tags (max {MAX_TAGS})."
        )));
    }
    Ok(out)
}

pub const AUTH_METHODS: &[&str] = &["password", "publickey", "agent", "keyboard-interactive"];

pub fn auth_method(raw: &str) -> AppResult<String> {
    if AUTH_METHODS.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(AppError::Validation(
            "Unknown authentication method.".into(),
        ))
    }
}

/// Ids come from the frontend; only accept well-formed UUIDs.
pub fn id(raw: &str) -> AppResult<String> {
    Uuid::parse_str(raw)
        .map(|u| u.to_string())
        .map_err(|_| AppError::Validation("Invalid id.".into()))
}

pub fn optional_id(raw: Option<&str>) -> AppResult<Option<String>> {
    raw.map(id).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(session_name("  web-01 ").unwrap(), "web-01");
        assert!(session_name("").is_err());
        assert!(session_name("a\tb").is_err());
        assert!(session_name(&"x".repeat(129)).is_err());
        assert!(folder_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn colours() {
        assert_eq!(color(Some("#1A2b3C")).unwrap().as_deref(), Some("#1a2b3c"));
        assert_eq!(color(Some("  ")).unwrap(), None);
        assert_eq!(color(None).unwrap(), None);
        for bad in ["red", "#12345", "#1234567", "#gggggg", "url(x)"] {
            assert!(color(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn tag_rules() {
        let t = tags(&["Prod".into(), " prod ".into(), "".into(), "db".into()]).unwrap();
        assert_eq!(t, vec!["Prod", "db"]);
        assert!(tags(&["a,b".into()]).is_err());
        assert!(tags(&["x".repeat(33)]).is_err());
        let many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
        assert!(tags(&many).is_err());
    }

    #[test]
    fn ids() {
        assert!(id("nope").is_err());
        assert!(id("'; DROP TABLE sessions; --").is_err());
        let u = Uuid::new_v4().to_string();
        assert_eq!(id(&u).unwrap(), u);
        assert_eq!(optional_id(None).unwrap(), None);
    }

    #[test]
    fn username_may_be_empty() {
        assert_eq!(optional_username("  ").unwrap(), "");
        assert_eq!(optional_username("root").unwrap(), "root");
        assert!(optional_username("a\nb").is_err());
    }
}
