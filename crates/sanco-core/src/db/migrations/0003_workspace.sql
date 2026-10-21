-- M3: workspace identity for networking.

-- Local only ---------------------------------------------------------------

-- The workspace this database belongs to. `sync_key` (hex) is the shared
-- secret every PC derives its bearer token from. It is not replicated, but a
-- join snapshot carries it to the new PC, which is how that PC learns it.
CREATE TABLE workspace (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    sync_key     TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);
