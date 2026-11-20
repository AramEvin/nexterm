//! Create the sessions of an import in the library.
//!
//! Everything lands in a top-level folder "Imported from <program>" (reused
//! on a second import), keeping the program's folder tree. Jump hosts become
//! their own SSH sessions in a "Jump hosts" subfolder, shared by every session
//! that uses them. Each session goes through `sessions::create`, so the same
//! validation as the editor applies.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;

use super::{ImportSource, ImportedHop, ImportedSession, MAX_SESSIONS};
use crate::error::{AppError, AppResult};
use crate::session::protocol::ProtocolKind;
use crate::storage::models::SessionInput;
use crate::storage::{folders, sessions};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub folder_id: String,
    pub created: usize,
    pub jump_hosts: usize,
    /// Already in the library (same protocol, host, port and user).
    pub existing: usize,
    pub failed: usize,
}

type Key = (ProtocolKind, String, u16, String);

fn key(protocol: ProtocolKind, host: &str, port: u16, user: &str) -> Key {
    (protocol, host.to_lowercase(), port, user.to_lowercase())
}

struct Folders {
    /// (parent, name) → id
    known: HashMap<(Option<String>, String), String>,
}

impl Folders {
    fn load(conn: &Connection) -> AppResult<Self> {
        let known = folders::list(conn)?
            .into_iter()
            .map(|f| ((f.parent_id, f.name), f.id))
            .collect();
        Ok(Self { known })
    }

    fn ensure(&mut self, conn: &Connection, parent: Option<&str>, name: &str) -> AppResult<String> {
        let k = (parent.map(str::to_owned), name.to_owned());
        if let Some(id) = self.known.get(&k) {
            return Ok(id.clone());
        }
        let f = folders::create(conn, parent, name)?;
        self.known.insert(k, f.id.clone());
        Ok(f.id)
    }

    fn path(&mut self, conn: &Connection, root: &str, path: &[String]) -> AppResult<String> {
        let mut parent = root.to_owned();
        for part in path {
            parent = self.ensure(conn, Some(&parent), part)?;
        }
        Ok(parent)
    }
}

pub fn apply(
    conn: &mut Connection,
    source: ImportSource,
    items: &[ImportedSession],
    skip_existing: bool,
) -> AppResult<ImportResult> {
    if items.len() > MAX_SESSIONS {
        return Err(AppError::Validation(format!(
            "Too many sessions at once (max {MAX_SESSIONS})."
        )));
    }
    let mut dirs = Folders::load(conn)?;
    let root = dirs.ensure(conn, None, &format!("Imported from {}", source.label()))?;

    let mut existing: HashMap<Key, String> = HashMap::new();
    for s in sessions::list(conn)? {
        existing
            .entry(key(s.protocol, &s.host, s.port, &s.username))
            .or_insert(s.id);
    }

    let mut result = ImportResult {
        folder_id: root.clone(),
        created: 0,
        jump_hosts: 0,
        existing: 0,
        failed: 0,
    };
    // (host, port, user, previous hop) → session id
    let mut hops: HashMap<(String, u16, String, Option<String>), String> = HashMap::new();
    let mut done: HashSet<Key> = HashSet::new();

    for item in items {
        let k = key(item.protocol, &item.host, item.port, &item.username);
        if skip_existing && (existing.contains_key(&k) || done.contains(&k)) {
            result.existing += 1;
            continue;
        }
        let outcome = (|| -> AppResult<()> {
            let jump = jump_chain(
                conn,
                &mut dirs,
                &root,
                &item.jump,
                &mut hops,
                &existing,
                &mut result,
            )?;
            let folder = dirs.path(conn, &root, &item.folder)?;
            let mut notes = format!("Imported from {}.", source.label());
            if let Some(path) = &item.key_path {
                notes.push_str(&format!(
                    "\nPrivate key used there: {path}\nAdd it as a key credential and link it to log in."
                ));
            }
            sessions::create(conn, &session_input(item, Some(folder), jump, notes))?;
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                result.created += 1;
                done.insert(k);
            }
            Err(AppError::Validation(msg)) => {
                tracing::debug!(%msg, "import: session skipped");
                result.failed += 1;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(result)
}

/// Find or create the jump-host sessions; returns the last hop's id.
fn jump_chain(
    conn: &mut Connection,
    dirs: &mut Folders,
    root: &str,
    chain: &[ImportedHop],
    hops: &mut HashMap<(String, u16, String, Option<String>), String>,
    existing: &HashMap<Key, String>,
    result: &mut ImportResult,
) -> AppResult<Option<String>> {
    let mut prev: Option<String> = None;
    for hop in chain {
        let k = (
            hop.host.to_lowercase(),
            hop.port,
            hop.username.to_lowercase(),
            prev.clone(),
        );
        if let Some(id) = hops.get(&k) {
            prev = Some(id.clone());
            continue;
        }
        // A first hop that's already a saved session is reused as is.
        if prev.is_none()
            && let Some(id) =
                existing.get(&key(ProtocolKind::Ssh, &hop.host, hop.port, &hop.username))
        {
            hops.insert(k, id.clone());
            prev = Some(id.clone());
            continue;
        }
        let folder = dirs.ensure(conn, Some(root), "Jump hosts")?;
        let name = if hop.username.is_empty() {
            hop.host.clone()
        } else {
            format!("{}@{}", hop.username, hop.host)
        };
        let input = session_input(
            &ImportedSession {
                name,
                folder: Vec::new(),
                protocol: ProtocolKind::Ssh,
                host: hop.host.clone(),
                port: hop.port,
                username: hop.username.clone(),
                auth_method: "password".into(),
                key_path: None,
                jump: Vec::new(),
                options: Default::default(),
            },
            Some(folder),
            prev.clone(),
            "Jump host created by the import. Link a credential so sessions behind it can connect."
                .into(),
        );
        let created = sessions::create(conn, &input)?;
        result.jump_hosts += 1;
        hops.insert(k, created.id.clone());
        prev = Some(created.id);
    }
    Ok(prev)
}

fn session_input(
    s: &ImportedSession,
    folder_id: Option<String>,
    jump_host_id: Option<String>,
    notes: String,
) -> SessionInput {
    SessionInput {
        folder_id,
        name: s.name.clone(),
        protocol: s.protocol,
        host: s.host.clone(),
        port: u32::from(s.port),
        username: s.username.clone(),
        auth_method: if s.protocol == ProtocolKind::Ssh {
            s.auth_method.clone()
        } else {
            "password".into()
        },
        credential_id: None,
        color: None,
        notes,
        favorite: false,
        resource_monitor: true,
        jump_host_id,
        options: s.options.clone(),
        tags: vec!["imported".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    fn item(name: &str, host: &str, folder: &[&str], jump: &[&str]) -> ImportedSession {
        ImportedSession {
            name: name.into(),
            folder: folder.iter().map(|s| (*s).to_owned()).collect(),
            protocol: ProtocolKind::Ssh,
            host: host.into(),
            port: 22,
            username: "deploy".into(),
            auth_method: "password".into(),
            key_path: None,
            options: Default::default(),
            jump: jump
                .iter()
                .map(|h| ImportedHop {
                    host: (*h).into(),
                    port: 22,
                    username: "jump".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn creates_folders_shared_jump_hosts_and_skips_existing() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let mut keyed = item("db", "db.internal", &["Prod", "DB"], &["bastion"]);
            keyed.auth_method = "publickey".into();
            keyed.key_path = Some(r"C:\keys\db.ppk".into());
            let items = vec![
                item("web", "web.internal", &["Prod"], &["bastion"]),
                keyed,
                item("deep", "deep.internal", &[], &["gw1", "gw2"]),
            ];
            let r = apply(c, ImportSource::Mobaxterm, &items, true)?;
            assert_eq!(
                (r.created, r.jump_hosts, r.existing, r.failed),
                (3, 3, 0, 0)
            );

            let all = sessions::list(c)?;
            let by_name = |n: &str| all.iter().find(|s| s.name == n).unwrap();
            let bastion = by_name("jump@bastion");
            assert_eq!(
                by_name("web").jump_host_id.as_deref(),
                Some(bastion.id.as_str())
            );
            assert_eq!(
                by_name("db").jump_host_id.as_deref(),
                Some(bastion.id.as_str())
            );
            assert!(by_name("db").notes.contains(r"C:\keys\db.ppk"));
            assert_eq!(by_name("db").auth_method, "publickey");
            // gw1 is dialled first, gw2 jumps through it, deep through gw2.
            let gw1 = by_name("jump@gw1");
            let gw2 = by_name("jump@gw2");
            assert_eq!(gw2.jump_host_id.as_deref(), Some(gw1.id.as_str()));
            assert_eq!(
                by_name("deep").jump_host_id.as_deref(),
                Some(gw2.id.as_str())
            );

            let dirs = folders::list(c)?;
            let names: HashSet<&str> = dirs.iter().map(|f| f.name.as_str()).collect();
            for n in ["Imported from MobaXterm", "Prod", "DB", "Jump hosts"] {
                assert!(names.contains(n), "{n}");
            }

            // A second import reuses folders and skips what's there.
            let again = apply(c, ImportSource::Mobaxterm, &items, true)?;
            assert_eq!((again.created, again.existing), (0, 3));
            assert_eq!(folders::list(c)?.len(), dirs.len());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn invalid_items_are_counted_not_fatal() {
        let db = Database::open_in_memory().unwrap();
        db.with(|c| {
            let mut bad = item("bad", "ok.host", &[], &[]);
            bad.auth_method = "telepathy".into();
            let r = apply(
                c,
                ImportSource::Putty,
                &[bad, item("good", "g.host", &[], &[])],
                false,
            )?;
            assert_eq!((r.created, r.failed), (1, 1));
            Ok(())
        })
        .unwrap();
    }
}
