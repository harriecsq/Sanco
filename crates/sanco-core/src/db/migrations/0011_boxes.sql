-- M11: boxes and box serials.
--
-- Items with a serial code come in boxes. Each box has one serial, e.g.
-- MT26-0001: the code, the 2-digit year, then a sequence that restarts
-- yearly and is shared by items with the same code. Serials live in
-- doc_numbers (series 'BOX:<code>', period = the year, ref_id = the box).
-- A purchase order gets its boxes when it is marked ordered; receiving,
-- stock documents and DRs then say which box each piece moves in or out of.

-- Item serial settings: serial_code (NULL = not boxed), serial_digits
-- (3, 4 or 5), pcs_per_box (the default box size).
ALTER TABLE items ADD COLUMN serial_code TEXT;
ALTER TABLE items ADD COLUMN serial_digits INTEGER;
ALTER TABLE items ADD COLUMN pcs_per_box INTEGER;

-- The year printed in the serials of the PO's boxes (YYYY).
ALTER TABLE purchase_orders ADD COLUMN serial_year TEXT;
-- Box size on this line; NULL = the item's.
ALTER TABLE po_lines ADD COLUMN pcs_per_box INTEGER;

-- The box a line moves stock in or out of (NULL for items without boxes).
ALTER TABLE stock_doc_lines ADD COLUMN box_id TEXT;
ALTER TABLE po_receipt_lines ADD COLUMN box_id TEXT;
CREATE INDEX stock_doc_lines_box ON stock_doc_lines (box_id);
CREATE INDEX po_receipt_lines_box ON po_receipt_lines (box_id);

-- Replicated ---------------------------------------------------------------

-- One box. It belongs to a PO line (po_id, po_line_id) or was made by a
-- stock receipt (doc_id). pcs is what it holds when full; the last box of a
-- line holds the rest. What is left in it is computed from v_stock.
CREATE TABLE boxes (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    item_id    TEXT,
    po_id      TEXT,
    po_line_id TEXT,
    doc_id     TEXT,
    pcs        INTEGER
);
CREATE INDEX boxes_item ON boxes (item_id);
CREATE INDEX boxes_po ON boxes (po_id);
CREATE INDEX boxes_po_line ON boxes (po_line_id);
CREATE INDEX boxes_doc ON boxes (doc_id);

-- How much of a DR line comes out of each box. id = derived_id(dr_line_id,
-- "box/{box_id}"); a draw that is taken off is cleared (qty NULL), never
-- deleted. expiry is the box's lot.
CREATE TABLE dr_line_boxes (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    dr_id      TEXT,
    dr_line_id TEXT,
    box_id     TEXT,
    expiry     TEXT,
    qty        INTEGER
);
CREATE INDEX dr_line_boxes_dr ON dr_line_boxes (dr_id);
CREATE INDEX dr_line_boxes_line ON dr_line_boxes (dr_line_id);
CREATE INDEX dr_line_boxes_box ON dr_line_boxes (box_id);

-- Views --------------------------------------------------------------------

DROP VIEW v_stock;
DROP VIEW v_stock_moves;

-- Every posted stock movement, one row per line, location and box: stock
-- documents (a transfer line gives an out row and an in row), delivered DR
-- lines (out, on the delivery date; per box when the line draws from
-- boxes) and posted PO receiving reports (in).
CREATE VIEW v_stock_moves AS
SELECT d.id AS doc_id, d.kind, d.date, l.id AS line_id, l.item_id,
       d.location_id, l.expiry, l.qty, l.box_id
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind IN ('receipt', 'adjustment')
UNION ALL
SELECT d.id, d.kind, d.date, l.id, l.item_id, d.location_id, l.expiry, -l.qty, l.box_id
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer'
UNION ALL
SELECT d.id, d.kind, d.date, l.id, l.item_id, d.to_location_id, l.expiry, l.qty, l.box_id
FROM stock_docs d JOIN stock_doc_lines l ON l.doc_id = d.id AND l._deleted = 0
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer'
UNION ALL
SELECT r.id, 'delivery', r.delivered_date, l.id, l.item_id, r.from_location_id, l.expiry, -l.qty,
       NULL
FROM delivery_receipts r JOIN dr_lines l ON l.dr_id = r.id AND l._deleted = 0
WHERE r._deleted = 0 AND r.status = 'delivered'
  AND NOT EXISTS (SELECT 1 FROM dr_line_boxes b
                  WHERE b.dr_line_id = l.id AND b._deleted = 0 AND b.qty IS NOT NULL)
UNION ALL
SELECT r.id, 'delivery', r.delivered_date, l.id, l.item_id, r.from_location_id, b.expiry, -b.qty,
       b.box_id
FROM delivery_receipts r JOIN dr_lines l ON l.dr_id = r.id AND l._deleted = 0
JOIN dr_line_boxes b ON b.dr_line_id = l.id AND b._deleted = 0 AND b.qty IS NOT NULL
WHERE r._deleted = 0 AND r.status = 'delivered'
UNION ALL
SELECT r.id, 'po_receipt', r.date, l.id, l.item_id, r.location_id, l.expiry, l.qty, l.box_id
FROM po_receipts r JOIN po_receipt_lines l ON l.receipt_id = r.id AND l._deleted = 0
WHERE r._deleted = 0 AND r.status = 'posted';

-- On hand by item, location, expiry and box. Rows can be zero or negative.
CREATE VIEW v_stock AS
SELECT item_id, location_id, expiry, box_id, SUM(qty) AS qty
FROM v_stock_moves
GROUP BY item_id, location_id, expiry, box_id;

-- Quantity received per box, from posted receiving reports.
CREATE VIEW v_box_received AS
SELECT l.box_id, SUM(COALESCE(l.qty, 0)) AS qty
FROM po_receipt_lines l
JOIN po_receipts r ON r.id = l.receipt_id AND r._deleted = 0 AND r.status = 'posted'
WHERE l._deleted = 0 AND l.box_id IS NOT NULL
GROUP BY l.box_id;
