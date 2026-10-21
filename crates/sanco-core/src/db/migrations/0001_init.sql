-- M1: sync bookkeeping and system tables.
--
-- Replicated tables all start with the same meta columns (see registry.rs)
-- and have no foreign keys or unique constraints besides the id, because
-- rows can arrive from peers in any order.

-- Local only ---------------------------------------------------------------

CREATE TABLE meta (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    site_id     TEXT NOT NULL,
    epoch       TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    last_hlc    INTEGER NOT NULL DEFAULT 0,
    ver         INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE row_versions (
    tbl TEXT NOT NULL,
    id  TEXT NOT NULL,
    ver INTEGER NOT NULL,
    PRIMARY KEY (tbl, id)
) WITHOUT ROWID;
CREATE INDEX row_versions_ver ON row_versions (ver);

CREATE TABLE peer_watermarks (
    site_id    TEXT PRIMARY KEY,
    epoch      TEXT NOT NULL,
    ver        INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Replicated ---------------------------------------------------------------

CREATE TABLE devices (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name           TEXT,
    host           TEXT,
    port           INTEGER,
    app_version    TEXT,
    schema_version INTEGER,
    last_seen      INTEGER
);

CREATE TABLE users (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    username     TEXT,
    display_name TEXT,
    pw_hash      TEXT,
    active       INTEGER
);
CREATE INDEX users_username ON users (username);

CREATE TABLE roles (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name       TEXT,
    perms_json TEXT
);

CREATE TABLE user_roles (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    user_id TEXT,
    role_id TEXT
);
CREATE INDEX user_roles_user ON user_roles (user_id);

CREATE TABLE audit_log (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    at           INTEGER,
    user_id      TEXT,
    action       TEXT,
    tbl          TEXT,
    row_id       TEXT,
    changes_json TEXT
);
CREATE INDEX audit_log_row ON audit_log (tbl, row_id);
CREATE INDEX audit_log_at ON audit_log (at);

CREATE TABLE todos (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    title       TEXT,
    notes       TEXT,
    due         TEXT,
    assignee_id TEXT,
    done        INTEGER,
    ref_tbl     TEXT,
    ref_id      TEXT
);
CREATE INDEX todos_assignee ON todos (assignee_id);

CREATE TABLE conflicts (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    tbl         TEXT,
    row_id      TEXT,
    lost_json   TEXT,
    resolved    INTEGER,
    resolved_by TEXT
);
CREATE INDEX conflicts_row ON conflicts (tbl, row_id);

CREATE TABLE settings (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    key   TEXT,
    value TEXT
);
