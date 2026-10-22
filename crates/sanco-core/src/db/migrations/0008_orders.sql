-- M9: orders. Purchase orders to suppliers, their TT payments, and
-- receiving reports that bring PO goods into stock.
--
-- Amounts on a PO are in minor units of its currency (USD/CNY/TWD/PHP);
-- fx_rate is PHP per unit, as decimal text. Numbers live in doc_numbers
-- (series PO for orders, RR for receiving reports). A receiving report is
-- posted when saved and never edited: a mistake is voided and entered
-- again, like a stock document.

-- Replicated ---------------------------------------------------------------

-- status: draft | ordered | shipped | arrived | closed | cancelled. How much
-- was received is computed from the receiving reports, never stored.
CREATE TABLE purchase_orders (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    supplier_id   TEXT,
    date          TEXT,
    currency      TEXT,
    fx_rate       TEXT,
    expected_date TEXT,
    status        TEXT,
    supplier_ref  TEXT,
    notes         TEXT,
    ordered_at    INTEGER,
    ordered_by    TEXT,
    shipped_date  TEXT,
    arrived_date  TEXT,
    closed_at     INTEGER,
    closed_by     TEXT,
    cancel_reason TEXT
);
CREATE INDEX purchase_orders_supplier ON purchase_orders (supplier_id);

-- price_minor is the supplier's unit price in the PO's currency (NULL if
-- not entered yet).
CREATE TABLE po_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    po_id       TEXT,
    line_no     INTEGER,
    item_id     TEXT,
    qty         INTEGER,
    price_minor INTEGER,
    notes       TEXT
);
CREATE INDEX po_lines_po ON po_lines (po_id);
CREATE INDEX po_lines_item ON po_lines (item_id);

-- A telegraphic transfer (or other payment) to the supplier for a PO.
-- amount_minor is in `currency`; fx_rate is PHP per unit (NULL for PHP).
CREATE TABLE supplier_payments (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    po_id        TEXT,
    date         TEXT,
    currency     TEXT,
    amount_minor INTEGER,
    fx_rate      TEXT,
    reference    TEXT,
    notes        TEXT
);
CREATE INDEX supplier_payments_po ON supplier_payments (po_id);

-- Receiving report. status: posted | void.
CREATE TABLE po_receipts (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    po_id       TEXT,
    date        TEXT,
    location_id TEXT,
    notes       TEXT,
    status      TEXT,
    voided_at   INTEGER,
    voided_by   TEXT,
    void_reason TEXT
);
CREATE INDEX po_receipts_po ON po_receipts (po_id);
CREATE INDEX po_receipts_location ON po_receipts (location_id);

-- expiry is the lot's expiry date (NULL for items without expiry).
CREATE TABLE po_receipt_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    receipt_id TEXT,
    line_no    INTEGER,
    po_line_id TEXT,
    item_id    TEXT,
    expiry     TEXT,
    qty        INTEGER
);
CREATE INDEX po_receipt_lines_receipt ON po_receipt_lines (receipt_id);
CREATE INDEX po_receipt_lines_po_line ON po_receipt_lines (po_line_id);
CREATE INDEX po_receipt_lines_item ON po_receipt_lines (item_id);

-- Views --------------------------------------------------------------------

DROP VIEW v_stock;
DROP VIEW v_stock_moves;

-- Every posted stock movement, one row per line and location: stock
-- documents (a transfer line gives an out row and an in row), delivered DR
-- lines (out, on the delivery date) and posted PO receiving reports (in).
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
WHERE d._deleted = 0 AND d.status = 'posted' AND d.kind = 'transfer'
UNION ALL
SELECT r.id, 'delivery', r.delivered_date, l.id, l.item_id, r.from_location_id, l.expiry, -l.qty
FROM delivery_receipts r JOIN dr_lines l ON l.dr_id = r.id AND l._deleted = 0
WHERE r._deleted = 0 AND r.status = 'delivered'
UNION ALL
SELECT r.id, 'po_receipt', r.date, l.id, l.item_id, r.location_id, l.expiry, l.qty
FROM po_receipts r JOIN po_receipt_lines l ON l.receipt_id = r.id AND l._deleted = 0
WHERE r._deleted = 0 AND r.status = 'posted';

-- On hand by item, location and expiry. Rows can be zero or negative.
CREATE VIEW v_stock AS
SELECT item_id, location_id, expiry, SUM(qty) AS qty
FROM v_stock_moves
GROUP BY item_id, location_id, expiry;

-- Quantity received per PO line, from posted receiving reports.
CREATE VIEW v_po_received AS
SELECT l.po_line_id, SUM(COALESCE(l.qty, 0)) AS qty
FROM po_receipt_lines l
JOIN po_receipts r ON r.id = l.receipt_id AND r._deleted = 0 AND r.status = 'posted'
WHERE l._deleted = 0 AND l.po_line_id IS NOT NULL
GROUP BY l.po_line_id;
