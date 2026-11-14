-- NexTerm schema v1 (Phase 3): folders, saved sessions, tags, settings.
-- No secrets are stored in this database. Passwords and keys live in the
-- encrypted vault (Phase 4); sessions only reference them by credential_id.
-- Timestamps are Unix epoch milliseconds (UTC).

CREATE TABLE folders (
    id          TEXT PRIMARY KEY NOT NULL,          -- UUID v4
    parent_id   TEXT REFERENCES folders(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX idx_folders_parent ON folders(parent_id, sort_order);

CREATE TABLE sessions (
    id                TEXT PRIMARY KEY NOT NULL,    -- UUID v4
    folder_id         TEXT REFERENCES folders(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    protocol          TEXT NOT NULL DEFAULT 'ssh',  -- ssh | telnet | serial | ... (later phases)
    host              TEXT NOT NULL,
    port              INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    username          TEXT NOT NULL DEFAULT '',
    auth_method       TEXT NOT NULL DEFAULT 'password',
    credential_id     TEXT,                         -- vault reference (Phase 4)
    jump_host_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,  -- Phase 7
    color             TEXT,                         -- '#rrggbb' or NULL
    notes             TEXT NOT NULL DEFAULT '',
    favorite          INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    options           TEXT NOT NULL DEFAULT '{}',   -- protocol-specific JSON
    sort_order        INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    last_connected_at INTEGER
);
CREATE INDEX idx_sessions_folder ON sessions(folder_id, sort_order);

CREATE TABLE session_tags (
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tag         TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (session_id, tag)
);
CREATE INDEX idx_session_tags_tag ON session_tags(tag);

CREATE TABLE settings (
    key         TEXT PRIMARY KEY NOT NULL,
    value       TEXT NOT NULL                       -- JSON
);
