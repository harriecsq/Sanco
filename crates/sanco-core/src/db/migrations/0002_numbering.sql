-- M2: shared document numbers (DR-2026-0001, client codes, ...).

-- Local only ---------------------------------------------------------------

-- Numbers this PC has promised to a reservation (its own or a peer's).
-- A claim blocks the number for other documents until it expires.
CREATE TABLE number_claims (
    series     TEXT NOT NULL,
    period     TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    ref_id     TEXT NOT NULL,
    site_id    TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (series, period, seq)
) WITHOUT ROWID;
CREATE INDEX number_claims_ref ON number_claims (ref_id);

-- Site ids this database wrote under before it was restored from a backup.
-- This PC renumbers clashing numbers that those sites assigned.
CREATE TABLE previous_sites (
    site_id    TEXT PRIMARY KEY,
    retired_at INTEGER NOT NULL
);

-- Replicated ---------------------------------------------------------------

-- One row per numbered document and series. id = derived_id(ref_id, "number/{series}").
CREATE TABLE doc_numbers (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    series      TEXT,
    period      TEXT,
    seq         INTEGER,
    no          TEXT,
    ref_tbl     TEXT,
    ref_id      TEXT,
    site        TEXT,
    assigned_at INTEGER,
    prev_no     TEXT,
    confirmed   INTEGER
);
CREATE INDEX doc_numbers_seq ON doc_numbers (series, period, seq);
CREATE INDEX doc_numbers_ref ON doc_numbers (ref_id);
