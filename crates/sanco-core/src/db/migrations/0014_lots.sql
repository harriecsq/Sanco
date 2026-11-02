-- M14: lots replace boxes.
--
-- A lot is one item's stock that arrived in one month. Stock is tracked per
-- item, location and lot; every stock line names its lot. The expiry and the
-- serial range live on the lot.
--
-- One-time exception to "migrations are additive only" (user decision,
-- 2026-11-02): boxes are removed outright. Peers on another schema version
-- are refused, so nothing can send box rows back, and no real data had boxes.

DROP VIEW v_box_received;
DROP VIEW v_stock;
DROP VIEW v_stock_moves;
-- Dropped around the column drops: SQLite re-reads every view on DROP COLUMN
-- and some versions can't resolve this view's correlated ORDER BY.
DROP VIEW v_receipt_no;

DROP TABLE dr_line_boxes;
DROP TABLE boxes;

-- Box serials and sync bookkeeping of what was dropped, the same on every PC.
DELETE FROM row_versions WHERE tbl IN ('boxes', 'dr_line_boxes');
DELETE FROM row_versions
WHERE tbl = 'doc_numbers'
  AND id IN (SELECT id FROM doc_numbers WHERE series LIKE 'BOX:%');
DELETE FROM doc_numbers WHERE series LIKE 'BOX:%';
DELETE FROM conflicts WHERE tbl IN ('boxes', 'dr_line_boxes');

DROP INDEX stock_doc_lines_box;
DROP INDEX po_receipt_lines_box;
ALTER TABLE stock_doc_lines DROP COLUMN box_id;
ALTER TABLE po_receipt_lines DROP COLUMN box_id;
ALTER TABLE stock_doc_lines DROP COLUMN expiry;
ALTER TABLE po_receipt_lines DROP COLUMN expiry;
ALTER TABLE dr_lines DROP COLUMN expiry;

ALTER TABLE items DROP COLUMN serial_code;
ALTER TABLE items DROP COLUMN serial_digits;
ALTER TABLE items DROP COLUMN pcs_per_box;
ALTER TABLE po_lines DROP COLUMN pcs_per_box;
ALTER TABLE purchase_orders DROP COLUMN serial_year;

-- Replicated ---------------------------------------------------------------

-- id = derived_id(item_id, "lot/<month>"), so PCs receiving offline get the
-- same row. month is YYYY-MM, the arrival month. A lot is never deleted (a
-- derived-id row couldn't come back). serial_from and serial_to are free
-- text, a visual reference only: nothing reads meaning into them.
CREATE TABLE lots (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    item_id     TEXT,
    month       TEXT,
    expiry      TEXT,
    serial_from TEXT,
    serial_to   TEXT
);
CREATE INDEX lots_item ON lots (item_id);

ALTER TABLE stock_doc_lines ADD COLUMN lot_id TEXT;
ALTER TABLE po_receipt_lines ADD COLUMN lot_id TEXT;
ALTER TABLE dr_lines ADD COLUMN lot_id TEXT;
CREATE INDEX stock_doc_lines_lot ON stock_doc_lines (lot_id);
CREATE INDEX po_receipt_lines_lot ON po_receipt_lines (lot_id);
CREATE INDEX dr_lines_lot ON dr_lines (lot_id);

-- Views --------------------------------------------------------------------

-- As in 0012, unchanged.
CREATE VIEW v_receipt_no AS
SELECT r.id AS dr_id, n.id AS number_id, n.series, n.period, n.seq, n.no,
       COALESCE(n.confirmed, 1) AS confirmed, COALESCE(n.manual, 0) AS manual
FROM delivery_receipts r
JOIN doc_numbers n ON n.id = (
    SELECT x.id FROM doc_numbers x
    WHERE x.ref_id = r.id AND x._deleted = 0 AND x.series IN ('DR', 'SI')
      AND x.seq IS NOT NULL AND x.no IS NOT NULL
    ORDER BY x.series = (CASE WHEN COALESCE(r.vat, 0) <> 0 THEN 'SI' ELSE 'DR' END) DESC, x.id
    LIMIT 1);

-- Every posted stock movement, one row per line and location: stock
-- documents (a transfer line gives an out row and an in row), delivered DR
-- lines (out, on the delivery date) and posted PO receiving reports (in).
CREATE VIEW v_stock_moves AS
SELECT d.id AS doc_id, d.kind, d.date, l.id AS line_id, l.item_id,
       d.location_id, l.lot_id, l.qty
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind IN ('receipt', 'adjustment')
UNION ALL
SELECT d.id, d.kind, d.date, l.id, l.item_id, d.location_id, l.lot_id, -l.qty
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer'
UNION ALL
SELECT d.id, d.kind, d.date, l.id, l.item_id, d.to_location_id, l.lot_id, l.qty
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer'
UNION ALL
SELECT r.id, 'delivery', r.delivered_date, l.id, l.item_id, r.from_location_id, l.lot_id, -l.qty
FROM delivery_receipts r JOIN dr_lines l ON l.dr_id = r.id AND l._deleted = 0
WHERE r._deleted = 0 AND r.status = 'delivered'
UNION ALL
SELECT r.id, 'po_receipt', r.date, l.id, l.item_id, r.location_id, l.lot_id, l.qty
FROM po_receipts r JOIN po_receipt_lines l ON l.receipt_id = r.id AND l._deleted = 0
WHERE r._deleted = 0 AND r.status = 'posted';

-- On hand by item, location and lot. Rows can be zero or negative.
CREATE VIEW v_stock AS
SELECT item_id, location_id, lot_id, SUM(qty) AS qty
FROM v_stock_moves
GROUP BY item_id, location_id, lot_id;
