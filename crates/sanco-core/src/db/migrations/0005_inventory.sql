-- M6: inventory. Manual stock documents (receipts, adjustments, transfers)
-- and the stock views.
--
-- A stock document is posted when saved and never edited; a mistake is
-- voided (status = 'void') and entered again. Quantities are integers in the
-- item's base unit. Numbers live in doc_numbers (series SR, ADJ, TR).

-- Replicated ---------------------------------------------------------------

-- kind: receipt | adjustment | transfer. status: posted | void.
-- location_id is where stock comes in (receipt), changes (adjustment) or
-- leaves from (transfer); to_location_id is a transfer's destination.
CREATE TABLE stock_docs (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    kind           TEXT,
    date           TEXT,
    location_id    TEXT,
    to_location_id TEXT,
    reason         TEXT,
    notes          TEXT,
    status         TEXT,
    voided_at      INTEGER,
    voided_by      TEXT,
    void_reason    TEXT
);
CREATE INDEX stock_docs_location ON stock_docs (location_id);
CREATE INDEX stock_docs_to_location ON stock_docs (to_location_id);

-- qty is signed for adjustments and positive otherwise. expiry is
-- YYYY-MM-DD, or NULL for items that don't track expiry.
CREATE TABLE stock_doc_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    doc_id  TEXT,
    line_no INTEGER,
    item_id TEXT,
    expiry  TEXT,
    qty     INTEGER
);
CREATE INDEX stock_doc_lines_doc ON stock_doc_lines (doc_id);
CREATE INDEX stock_doc_lines_item ON stock_doc_lines (item_id);

-- Views --------------------------------------------------------------------
-- Views are not replicated, so later migrations may drop and recreate them
-- (M7 adds delivered DR lines, M9 PO receipts).

-- Every posted stock movement, one row per line and location (a transfer
-- line gives an out row and an in row).
CREATE VIEW v_stock_moves AS
SELECT d.id AS doc_id, d.kind, d.date, l.id AS line_id, l.item_id,
       d.location_id, l.expiry, l.qty
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind IN ('receipt', 'adjustment')
UNION ALL
SELECT d.id, d.kind, d.date, l.id, l.item_id, d.location_id, l.expiry, -l.qty
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer'
UNION ALL
SELECT d.id, d.kind, d.date, l.id, l.item_id, d.to_location_id, l.expiry, l.qty
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer';

-- On hand by item, location and expiry. Rows can be zero or negative.
CREATE VIEW v_stock AS
SELECT item_id, location_id, expiry, SUM(qty) AS qty
FROM v_stock_moves
GROUP BY item_id, location_id, expiry;
