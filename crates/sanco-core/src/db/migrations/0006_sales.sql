-- M7: sales flow. Business Development requests, bookings and delivery
-- receipts (DRs). Delivered DR lines take stock out.
--
-- Numbers live in doc_numbers (series BD, BK, DR). DR numbers are typed
-- from the BIR booklet by default (doc_numbers.manual = 1); a manual number
-- is never renumbered automatically, a clash is flagged instead.

-- Replicated ---------------------------------------------------------------

-- A typed (manual) number. NULL or 0 = given automatically.
ALTER TABLE doc_numbers ADD COLUMN manual INTEGER;

-- Client or prospect inquiry. status: open | quoted | won | lost. A won
-- request has a booking (bookings.bd_request_id).
CREATE TABLE bd_requests (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    client_id     TEXT,
    prospect_name TEXT,
    contact       TEXT,
    date          TEXT,
    status        TEXT,
    follow_up     TEXT,
    vat           INTEGER,
    notes         TEXT,
    lost_reason   TEXT
);
CREATE INDEX bd_requests_client ON bd_requests (client_id);

-- item_id may be NULL for something not in the catalog (description only).
-- price_c is the quoted net unit price, if any.
CREATE TABLE bd_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    request_id  TEXT,
    line_no     INTEGER,
    item_id     TEXT,
    description TEXT,
    qty         INTEGER,
    price_c     INTEGER
);
CREATE INDEX bd_lines_request ON bd_lines (request_id);
CREATE INDEX bd_lines_item ON bd_lines (item_id);

-- Confirmed client order. status: draft | confirmed | closed | cancelled.
-- vat_bp is the VAT rate when the booking was last saved.
CREATE TABLE bookings (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    client_id     TEXT,
    bd_request_id TEXT,
    date          TEXT,
    status        TEXT,
    vat           INTEGER,
    vat_bp        INTEGER,
    client_ref    TEXT,
    deliver_to    TEXT,
    notes         TEXT,
    confirmed_at  INTEGER,
    confirmed_by  TEXT,
    closed_at     INTEGER,
    closed_by     TEXT,
    cancel_reason TEXT
);
CREATE INDEX bookings_client ON bookings (client_id);
CREATE INDEX bookings_bd ON bookings (bd_request_id);

-- price_c is the net unit price; price_rule says where it came from
-- (manual | client_tier | item_default | quoted | custom).
CREATE TABLE booking_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    booking_id TEXT,
    line_no    INTEGER,
    item_id    TEXT,
    qty        INTEGER,
    price_c    INTEGER,
    price_rule TEXT,
    notes      TEXT
);
CREATE INDEX booking_lines_booking ON booking_lines (booking_id);
CREATE INDEX booking_lines_item ON booking_lines (item_id);

-- Delivery receipt. status: requested | approved | delivered | cancelled.
-- A cancelled DR with delivered_at set was voided after delivery. Lines
-- are never edited: a wrong DR is cancelled and requested again.
CREATE TABLE delivery_receipts (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    booking_id       TEXT,
    client_id        TEXT,
    date             TEXT,
    from_location_id TEXT,
    status           TEXT,
    vat              INTEGER,
    vat_bp           INTEGER,
    deliver_to       TEXT,
    notes            TEXT,
    approved_at      INTEGER,
    approved_by      TEXT,
    delivered_date   TEXT,
    delivered_at     INTEGER,
    delivered_by     TEXT,
    received_by      TEXT,
    cancelled_at     INTEGER,
    cancelled_by     TEXT,
    cancel_reason    TEXT
);
CREATE INDEX delivery_receipts_booking ON delivery_receipts (booking_id);
CREATE INDEX delivery_receipts_client ON delivery_receipts (client_id);
CREATE INDEX delivery_receipts_location ON delivery_receipts (from_location_id);

-- expiry names the lot the stock comes from (NULL for items without expiry).
CREATE TABLE dr_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    dr_id           TEXT,
    line_no         INTEGER,
    booking_line_id TEXT,
    item_id         TEXT,
    expiry          TEXT,
    qty             INTEGER,
    price_c         INTEGER
);
CREATE INDEX dr_lines_dr ON dr_lines (dr_id);
CREATE INDEX dr_lines_booking_line ON dr_lines (booking_line_id);
CREATE INDEX dr_lines_item ON dr_lines (item_id);

-- Views --------------------------------------------------------------------

DROP VIEW v_stock;
DROP VIEW v_stock_moves;

-- Every posted stock movement, one row per line and location: stock
-- documents (a transfer line gives an out row and an in row) and
-- delivered DR lines (out, on the delivery date).
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
WHERE r._deleted = 0 AND r.status = 'delivered';

-- On hand by item, location and expiry. Rows can be zero or negative.
CREATE VIEW v_stock AS
SELECT item_id, location_id, expiry, SUM(qty) AS qty
FROM v_stock_moves
GROUP BY item_id, location_id, expiry;
