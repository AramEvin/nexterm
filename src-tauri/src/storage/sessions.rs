//! Saved-session repository.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::folders::{self, reorder};
use super::models::{SavedSession, SessionInput};
use super::{now_ms, validate};
use crate::error::{AppError, AppResult};
use crate::session::protocol::ProtocolKind;

const SELECT_COLUMNS: &str = "id, folder_id, name, protocol, host, port, username, auth_method,
     credential_id, jump_host_id, color, notes, favorite, sort_order, created_at, updated_at,
     last_connected_at";

/// Validated `SessionInput`.
struct Valid {
    folder_id: Option<String>,
    name: String,
    protocol: ProtocolKind,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    credential_id: Option<String>,
    color: Option<String>,
    notes: String,
    favorite: bool,
    tags: Vec<String>,
}

fn validate_input(conn: &Connection, input: &SessionInput) -> AppResult<Valid> {
    let folder_id = validate::optional_id(input.folder_id.as_deref())?;
    if let Some(f) = folder_id.as_deref() {
        folders::get(conn, f)?;
    }
    let credential_id = validate::optional_id(input.credential_id.as_deref())?;
    if let Some(cid) = credential_id.as_deref() {
        folders::ensure_credential(conn, cid)?;
    }
    Ok(Valid {
        folder_id,
        name: validate::session_name(&input.name)?,
        protocol: input.protocol,
        host: validate::host(&input.host)?,
        port: validate::port(input.port)?,
        username: validate::optional_username(&input.username)?,
        auth_method: validate::auth_method(&input.auth_method)?,
        credential_id,
        color: validate::color(input.color.as_deref())?,
        notes: validate::notes(&input.notes)?,
        favorite: input.favorite,
        tags: validate::tags(&input.tags)?,
    })
}

fn protocol_to_db(p: ProtocolKind) -> &'static str {
    match p {
        ProtocolKind::Ssh => "ssh",
        ProtocolKind::Ftp => "ftp",
        ProtocolKind::Ftps => "ftps",
    }
}

fn protocol_from_db(raw: &str) -> rusqlite::Result<ProtocolKind> {
    match raw {
        "ssh" => Ok(ProtocolKind::Ssh),
        "ftp" => Ok(ProtocolKind::Ftp),
        "ftps" => Ok(ProtocolKind::Ftps),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            format!("unknown protocol {other:?}").into(),
        )),
    }
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedSession> {
    let protocol: String = row.get("protocol")?;
    let port: i64 = row.get("port")?;
    let favorite: i64 = row.get("favorite")?;
    Ok(SavedSession {
        id: row.get("id")?,
        folder_id: row.get("folder_id")?,
        name: row.get("name")?,
        protocol: protocol_from_db(&protocol)?,
        host: row.get("host")?,
        port: u16::try_from(port).unwrap_or(0),
        username: row.get("username")?,
        auth_method: row.get("auth_method")?,
        credential_id: row.get("credential_id")?,
        jump_host_id: row.get("jump_host_id")?,
        color: row.get("color")?,
        notes: row.get("notes")?,
        favorite: favorite != 0,
        tags: Vec::new(),
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        last_connected_at: row.get("last_connected_at")?,
    })
}

fn tags_for(conn: &Connection, id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT tag FROM session_tags WHERE session_id = ?1 ORDER BY rowid")?;
    let tags = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(tags)
}

fn write_tags(tx: &Transaction<'_>, id: &str, tags: &[String]) -> AppResult<()> {
    tx.execute("DELETE FROM session_tags WHERE session_id = ?1", [id])?;
    let mut insert = tx.prepare("INSERT INTO session_tags (session_id, tag) VALUES (?1, ?2)")?;
    for tag in tags {
        insert.execute(params![id, tag])?;
    }
    Ok(())
}

fn next_sort_order(conn: &Connection, folder_id: Option<&str>) -> AppResult<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM sessions WHERE folder_id IS ?1",
        [folder_id],
        |r| r.get(0),
    )?)
}

/// All sessions with their tags (two queries, no N+1).
pub fn list(conn: &Connection) -> AppResult<Vec<SavedSession>> {
    let mut tag_map: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT session_id, tag FROM session_tags ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, tag) = row?;
            tag_map.entry(id).or_default().push(tag);
        }
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM sessions ORDER BY sort_order, name COLLATE NOCASE"
    ))?;
    let sessions = stmt
        .query_map([], row_to_session)?
        .map(|r| {
            r.map(|mut s| {
                s.tags = tag_map.remove(&s.id).unwrap_or_default();
                s
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(sessions)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<SavedSession> {
    let id = validate::id(id)?;
    let mut session = conn
        .query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM sessions WHERE id = ?1"),
            [&id],
            row_to_session,
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound("Session".into()))?;
    session.tags = tags_for(conn, &id)?;
    Ok(session)
}

pub fn create(conn: &mut Connection, input: &SessionInput) -> AppResult<SavedSession> {
    let v = validate_input(conn, input)?;
    let id = Uuid::new_v4().to_string();
    let now = now_ms();
    let tx = conn.transaction()?;
    let order = next_sort_order(&tx, v.folder_id.as_deref())?;
    tx.execute(
        "INSERT INTO sessions (id, folder_id, name, protocol, host, port, username, auth_method,
                               credential_id, color, notes, favorite, sort_order, created_at,
                               updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
        params![
            id,
            v.folder_id,
            v.name,
            protocol_to_db(v.protocol),
            v.host,
            v.port,
            v.username,
            v.auth_method,
            v.credential_id,
            v.color,
            v.notes,
            v.favorite,
            order,
            now
        ],
    )?;
    write_tags(&tx, &id, &v.tags)?;
    tx.commit()?;
    get(conn, &id)
}

pub fn update(conn: &mut Connection, id: &str, input: &SessionInput) -> AppResult<SavedSession> {
    let existing = get(conn, id)?;
    let v = validate_input(conn, input)?;
    let tx = conn.transaction()?;
    let order = if existing.folder_id == v.folder_id {
        existing.sort_order
    } else {
        next_sort_order(&tx, v.folder_id.as_deref())?
    };
    tx.execute(
        "UPDATE sessions SET folder_id = ?2, name = ?3, protocol = ?4, host = ?5, port = ?6,
                username = ?7, auth_method = ?8, credential_id = ?9, color = ?10, notes = ?11,
                favorite = ?12, sort_order = ?13, updated_at = ?14
         WHERE id = ?1",
        params![
            existing.id,
            v.folder_id,
            v.name,
            protocol_to_db(v.protocol),
            v.host,
            v.port,
            v.username,
            v.auth_method,
            v.credential_id,
            v.color,
            v.notes,
            v.favorite,
            order,
            now_ms()
        ],
    )?;
    write_tags(&tx, &existing.id, &v.tags)?;
    tx.commit()?;
    get(conn, &existing.id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let id = validate::id(id)?;
    if conn.execute("DELETE FROM sessions WHERE id = ?1", [&id])? == 0 {
        return Err(AppError::NotFound("Session".into()));
    }
    Ok(())
}

/// Copy a session ("name (copy)"), placed right after the original.
pub fn duplicate(conn: &mut Connection, id: &str) -> AppResult<SavedSession> {
    let src = get(conn, id)?;
    let mut name = format!("{} (copy)", src.name);
    if name.chars().count() > validate::MAX_NAME_LEN {
        name = name.chars().take(validate::MAX_NAME_LEN).collect();
    }
    let input = SessionInput {
        folder_id: src.folder_id.clone(),
        name,
        protocol: src.protocol,
        host: src.host.clone(),
        port: u32::from(src.port),
        username: src.username.clone(),
        auth_method: src.auth_method.clone(),
        credential_id: src.credential_id.clone(),
        color: src.color.clone(),
        notes: src.notes.clone(),
        favorite: false,
        tags: src.tags.clone(),
    };
    let copy = create(conn, &input)?;
    // Place it right after the source: before the source's next sibling.
    let next: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions WHERE folder_id IS ?1 AND sort_order > ?2 AND id != ?3
             ORDER BY sort_order LIMIT 1",
            params![src.folder_id, src.sort_order, copy.id],
            |r| r.get(0),
        )
        .optional()?;
    move_to(conn, &copy.id, src.folder_id.as_deref(), next.as_deref())?;
    get(conn, &copy.id)
}

/// Move a session into `folder_id` (None = root), before `before_id` (None = last).
pub fn move_to(
    conn: &mut Connection,
    id: &str,
    folder_id: Option<&str>,
    before_id: Option<&str>,
) -> AppResult<()> {
    let id = validate::id(id)?;
    let folder_id = validate::optional_id(folder_id)?;
    let before_id = validate::optional_id(before_id)?;
    get(conn, &id)?;
    if let Some(f) = folder_id.as_deref() {
        folders::get(conn, f)?;
    }
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE sessions SET folder_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, folder_id, now_ms()],
    )?;
    reorder(
        &tx,
        "sessions",
        "folder_id",
        folder_id.as_deref(),
        &id,
        before_id.as_deref(),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn set_favorite(conn: &Connection, id: &str, favorite: bool) -> AppResult<()> {
    let id = validate::id(id)?;
    let changed = conn.execute(
        "UPDATE sessions SET favorite = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, favorite, now_ms()],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Session".into()));
    }
    Ok(())
}

pub fn mark_connected(conn: &Connection, id: &str) -> AppResult<()> {
    let id = validate::id(id)?;
    conn.execute(
        "UPDATE sessions SET last_connected_at = ?2 WHERE id = ?1",
        params![id, now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    fn input(name: &str, folder: Option<&str>) -> SessionInput {
        SessionInput {
            folder_id: folder.map(str::to_string),
            name: name.into(),
            protocol: ProtocolKind::Ssh,
            host: "10.0.0.1".into(),
            port: 22,
            username: "root".into(),
            auth_method: "password".into(),
            credential_id: None,
            color: Some("#14B8A6".into()),
            notes: "primary db".into(),
            favorite: false,
            tags: vec!["prod".into(), "db".into()],
        }
    }

    fn order(conn: &Connection, folder: Option<&str>) -> Vec<String> {
        list(conn)
            .unwrap()
            .into_iter()
            .filter(|s| s.folder_id.as_deref() == folder)
            .map(|s| s.name)
            .collect()
    }

    #[test]
    fn crud_roundtrip_with_tags() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let s = create(c, &input("db-01", None))?;
            assert_eq!(s.color.as_deref(), Some("#14b8a6"));
            assert_eq!(s.tags, ["prod", "db"]);
            assert_eq!(s.protocol, ProtocolKind::Ssh);

            let mut changed = input("db-01-renamed", None);
            changed.tags = vec!["staging".into()];
            changed.port = 2222;
            let u = update(c, &s.id, &changed)?;
            assert_eq!(u.name, "db-01-renamed");
            assert_eq!(u.port, 2222);
            assert_eq!(u.tags, ["staging"]);
            assert_eq!(list(c)?.len(), 1);

            set_favorite(c, &s.id, true)?;
            mark_connected(c, &s.id)?;
            let g = get(c, &s.id)?;
            assert!(g.favorite);
            assert!(g.last_connected_at.is_some());

            delete(c, &s.id)?;
            assert!(list(c)?.is_empty());
            let tags: i64 = c.query_row("SELECT COUNT(*) FROM session_tags", [], |r| r.get(0))?;
            assert_eq!(tags, 0, "tags are deleted with the session");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn validation_rejects_bad_input() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let mut bad = input("x", None);
            bad.host = "bad host".into();
            assert_eq!(create(c, &bad).unwrap_err().code(), "VALIDATION");
            let mut bad = input("x", None);
            bad.port = 70000;
            assert_eq!(create(c, &bad).unwrap_err().code(), "VALIDATION");
            let mut bad = input("x", None);
            bad.color = Some("javascript:alert(1)".into());
            assert_eq!(create(c, &bad).unwrap_err().code(), "VALIDATION");
            let missing = Uuid::new_v4().to_string();
            assert_eq!(
                create(c, &input("x", Some(&missing))).unwrap_err().code(),
                "NOT_FOUND"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn folder_delete_cascades_to_sessions() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let f = folders::create(c, None, "Prod")?;
            let sub = folders::create(c, Some(&f.id), "DB")?;
            create(c, &input("a", Some(&f.id)))?;
            create(c, &input("b", Some(&sub.id)))?;
            create(c, &input("keep", None))?;
            assert_eq!(folders::delete(c, &f.id)?, 2);
            assert_eq!(order(c, None), ["keep"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_goes_right_after_source() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let a = create(c, &input("a", None))?;
            create(c, &input("b", None))?;
            let copy = duplicate(c, &a.id)?;
            assert_eq!(copy.name, "a (copy)");
            assert_eq!(copy.tags, ["prod", "db"]);
            assert!(!copy.favorite);
            assert_eq!(order(c, None), ["a", "a (copy)", "b"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn move_between_folders_and_reorder() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let f = folders::create(c, None, "F")?;
            let a = create(c, &input("a", None))?;
            let b = create(c, &input("b", None))?;
            move_to(c, &a.id, Some(&f.id), None)?;
            assert_eq!(order(c, None), ["b"]);
            assert_eq!(order(c, Some(&f.id)), ["a"]);
            move_to(c, &b.id, Some(&f.id), Some(&a.id))?;
            assert_eq!(order(c, Some(&f.id)), ["b", "a"]);
            Ok(())
        })
        .unwrap();
    }

    /// Phase 3 acceptance: 500+ sessions list quickly.
    #[test]
    fn list_scales_to_thousands() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let f = folders::create(c, None, "Bulk")?;
            for i in 0..2000 {
                create(c, &input(&format!("srv-{i:04}"), Some(&f.id)))?;
            }
            let start = std::time::Instant::now();
            let all = list(c)?;
            let elapsed = start.elapsed();
            assert_eq!(all.len(), 2000);
            assert!(all.iter().all(|s| s.tags.len() == 2));
            assert!(
                elapsed.as_millis() < 500,
                "listing 2000 sessions took {elapsed:?}"
            );
            Ok(())
        })
        .unwrap();
    }
}
