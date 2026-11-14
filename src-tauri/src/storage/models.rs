//! Storage DTOs. Field names are camelCase on the wire (mirrored in `src/types/library.ts`).

use serde::{Deserialize, Serialize};

use crate::session::protocol::ProtocolKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSession {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub protocol: ProtocolKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub credential_id: Option<String>,
    pub jump_host_id: Option<String>,
    pub color: Option<String>,
    pub notes: String,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_connected_at: Option<i64>,
}

/// Editable fields of a saved session (create and update).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionInput {
    pub folder_id: Option<String>,
    pub name: String,
    pub protocol: ProtocolKind,
    pub host: String,
    pub port: u32,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Everything the sidebar needs, in one round trip.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub folders: Vec<Folder>,
    pub sessions: Vec<SavedSession>,
}
