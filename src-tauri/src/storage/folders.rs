//! Folder repository. Folders nest via `parent_id`; deleting a folder deletes
//! its subfolders and sessions (FK `ON DELETE CASCADE`). The UI confirms first.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::models::Folder;
use super::now_ms;
use super::validate::{self, MAX_FOLDER_DEPTH};
use crate::error::{AppError, AppResult};

fn row_to_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get("id")?,
        parent_id: row.get("parent_id")?,
        name: row.get("name")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, name, sort_order, created_at, updated_at
         FROM folders ORDER BY sort_order, name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], row_to_folder)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Folder> {
    conn.query_row(
        "SELECT id, parent_id, name, sort_order, created_at, updated_at FROM folders WHERE id = ?1",
        [id],
        row_to_folder,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound("Folder".into()))
}

/// Ids from `id` up to the root, starting with `id` itself.
fn ancestors(conn: &Connection, id: &str) -> AppResult<Vec<String>> {
    let mut chain = vec![id.to_string()];
    let mut current = id.to_string();
    // Bounded loop: a corrupted cyclic chain cannot hang the app.
    for _ in 0..=MAX_FOLDER_DEPTH * 4 {
        let parent: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM folders WHERE id = ?1",
                [&current],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        match parent {
            Some(p) => {
                chain.push(p.clone());
                current = p;
            }
            None => return Ok(chain),
        }
    }
    Err(AppError::Internal(
        "folder hierarchy is corrupted (cycle)".into(),
    ))
}

/// Height of the subtree rooted at `id` (a leaf folder has height 1).
fn subtree_height(conn: &Connection, id: &str) -> AppResult<usize> {
    let height: i64 = conn.query_row(
        "WITH RECURSIVE sub(id, depth) AS (
             SELECT id, 1 FROM folders WHERE id = ?1
             UNION ALL
             SELECT f.id, sub.depth + 1 FROM folders f JOIN sub ON f.parent_id = sub.id
             WHERE sub.depth < 64
         )
         SELECT MAX(depth) FROM sub",
        [id],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(height).unwrap_or(usize::MAX))
}

fn next_sort_order(conn: &Connection, parent_id: Option<&str>) -> AppResult<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM folders WHERE parent_id IS ?1",
        [parent_id],
        |r| r.get(0),
    )?)
}

pub fn create(conn: &Connection, parent_id: Option<&str>, name: &str) -> AppResult<Folder> {
    let parent_id = validate::optional_id(parent_id)?;
    let name = validate::folder_name(name)?;
    if let Some(parent) = parent_id.as_deref() {
        get(conn, parent)?;
        if ancestors(conn, parent)?.len() >= MAX_FOLDER_DEPTH {
            return Err(AppError::Validation(format!(
                "Folders can be nested at most {MAX_FOLDER_DEPTH} levels deep."
            )));
        }
    }
    let now = now_ms();
    let folder = Folder {
        id: Uuid::new_v4().to_string(),
        parent_id: parent_id.clone(),
        name,
        sort_order: next_sort_order(conn, parent_id.as_deref())?,
        created_at: now,
        updated_at: now,
    };
    conn.execute(
        "INSERT INTO folders (id, parent_id, name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            folder.id,
            folder.parent_id,
            folder.name,
            folder.sort_order,
            folder.created_at,
            folder.updated_at
        ],
    )?;
    Ok(folder)
}

pub fn rename(conn: &Connection, id: &str, name: &str) -> AppResult<Folder> {
    let id = validate::id(id)?;
    let name = validate::folder_name(name)?;
    let changed = conn.execute(
        "UPDATE folders SET name = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, name, now_ms()],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Folder".into()));
    }
    get(conn, &id)
}

/// Move a folder under `new_parent` (None = root), placed before `before_id`
/// (None = at the end). Rejects moving a folder into itself or a descendant.
pub fn move_to(
    conn: &mut Connection,
    id: &str,
    new_parent: Option<&str>,
    before_id: Option<&str>,
) -> AppResult<()> {
    let id = validate::id(id)?;
    let new_parent = validate::optional_id(new_parent)?;
    let before_id = validate::optional_id(before_id)?;
    get(conn, &id)?;

    if let Some(parent) = new_parent.as_deref() {
        get(conn, parent)?;
        let chain = ancestors(conn, parent)?;
        if chain.contains(&id) {
            return Err(AppError::Validation(
                "A folder can't be moved into itself or one of its subfolders.".into(),
            ));
        }
        if chain.len() + subtree_height(conn, &id)? > MAX_FOLDER_DEPTH {
            return Err(AppError::Validation(format!(
                "Folders can be nested at most {MAX_FOLDER_DEPTH} levels deep."
            )));
        }
    }

    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE folders SET parent_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, new_parent, now_ms()],
    )?;
    reorder(
        &tx,
        "folders",
        "parent_id",
        new_parent.as_deref(),
        &id,
        before_id.as_deref(),
    )?;
    tx.commit()?;
    Ok(())
}

/// Delete a folder and everything inside it. Returns the number of sessions removed.
pub fn delete(conn: &Connection, id: &str) -> AppResult<usize> {
    let id = validate::id(id)?;
    let sessions: i64 = conn.query_row(
        "WITH RECURSIVE sub(id) AS (
             SELECT id FROM folders WHERE id = ?1
             UNION ALL
             SELECT f.id FROM folders f JOIN sub ON f.parent_id = sub.id
         )
         SELECT COUNT(*) FROM sessions WHERE folder_id IN (SELECT id FROM sub)",
        [&id],
        |r| r.get(0),
    )?;
    let changed = conn.execute("DELETE FROM folders WHERE id = ?1", [&id])?;
    if changed == 0 {
        return Err(AppError::NotFound("Folder".into()));
    }
    Ok(usize::try_from(sessions).unwrap_or(0))
}

/// Renumber the siblings of `moved_id` so it sits before `before_id` (or last).
/// `table` and `parent_col` are compile-time constants, never user input.
pub(super) fn reorder(
    conn: &Connection,
    table: &'static str,
    parent_col: &'static str,
    parent: Option<&str>,
    moved_id: &str,
    before_id: Option<&str>,
) -> AppResult<()> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM {table} WHERE {parent_col} IS ?1 AND id != ?2 ORDER BY sort_order, rowid"
    ))?;
    let mut ids: Vec<String> = stmt
        .query_map(params![parent, moved_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let position = before_id
        .and_then(|b| ids.iter().position(|i| i == b))
        .unwrap_or(ids.len());
    ids.insert(position, moved_id.to_string());

    let mut update = conn.prepare(&format!("UPDATE {table} SET sort_order = ?2 WHERE id = ?1"))?;
    for (order, id) in ids.iter().enumerate() {
        update.execute(params![id, i64::try_from(order).unwrap_or(i64::MAX)])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    fn names(conn: &Connection, parent: Option<&str>) -> Vec<String> {
        list(conn)
            .unwrap()
            .into_iter()
            .filter(|f| f.parent_id.as_deref() == parent)
            .map(|f| f.name)
            .collect()
    }

    #[test]
    fn create_nested_rename_delete() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let prod = create(c, None, "Prod")?;
            let web = create(c, Some(&prod.id), "Web")?;
            assert_eq!(web.parent_id.as_deref(), Some(prod.id.as_str()));
            assert_eq!(rename(c, &web.id, " Frontend ")?.name, "Frontend");
            delete(c, &prod.id)?;
            assert!(list(c)?.is_empty(), "children are deleted with the parent");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rejects_unknown_parent_and_bad_ids() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let missing = Uuid::new_v4().to_string();
            assert_eq!(
                create(c, Some(&missing), "x").unwrap_err().code(),
                "NOT_FOUND"
            );
            assert_eq!(
                create(c, Some("../etc"), "x").unwrap_err().code(),
                "VALIDATION"
            );
            assert_eq!(rename(c, &missing, "x").unwrap_err().code(), "NOT_FOUND");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn move_prevents_cycles() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let a = create(c, None, "A")?;
            let b = create(c, Some(&a.id), "B")?;
            let cc = create(c, Some(&b.id), "C")?;
            assert_eq!(
                move_to(c, &a.id, Some(&cc.id), None).unwrap_err().code(),
                "VALIDATION"
            );
            assert_eq!(
                move_to(c, &a.id, Some(&a.id), None).unwrap_err().code(),
                "VALIDATION"
            );
            // Legal move: C to root.
            move_to(c, &cc.id, None, None)?;
            assert_eq!(get(c, &cc.id)?.parent_id, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn depth_limit() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let mut parent: Option<String> = None;
            for i in 0..MAX_FOLDER_DEPTH {
                parent = Some(create(c, parent.as_deref(), &format!("L{i}"))?.id);
            }
            assert_eq!(
                create(c, parent.as_deref(), "too deep").unwrap_err().code(),
                "VALIDATION"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reorder_within_parent() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let a = create(c, None, "A")?;
            let _b = create(c, None, "B")?;
            let cc = create(c, None, "C")?;
            assert_eq!(names(c, None), ["A", "B", "C"]);
            move_to(c, &cc.id, None, Some(&a.id))?;
            assert_eq!(names(c, None), ["C", "A", "B"]);
            move_to(c, &cc.id, None, None)?;
            assert_eq!(names(c, None), ["A", "B", "C"]);
            Ok(())
        })
        .unwrap();
    }
}
