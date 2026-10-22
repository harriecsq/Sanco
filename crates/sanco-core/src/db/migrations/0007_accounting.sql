-- M8: accounting. Payments received from clients (cash, check, bank
-- transfer), their allocations to delivered DRs, and receivables.
--
-- A check is a payment with method 'check' and the check_* columns. Its
-- status is on_hand | deposited | cleared | bounced; "replaced" is not
-- stored: a check is replaced while a posted payment names it in
-- replaces_id. Numbers live in doc_numbers (series PAY).

-- Replicated ---------------------------------------------------------------

-- status: posted | void. or_no is the official/collection receipt number
-- from the booklet, if one was issued.
CREATE TABLE payments (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    client_id       TEXT,
    date            TEXT,
    method          TEXT,
    amount_c        INTEGER,
    reference       TEXT,
    or_no           TEXT,
    notes           TEXT,
    check_bank      TEXT,
    check_no        TEXT,
    check_date      TEXT,
    check_status    TEXT,
    deposited_date  TEXT,
    deposit_account TEXT,
    cleared_date    TEXT,
    bounced_date    TEXT,
    bounce_reason   TEXT,
    replaces_id     TEXT,
    status          TEXT,
    voided_at       INTEGER,
    voided_by       TEXT,
    void_reason     TEXT
);
CREATE INDEX payments_client ON payments (client_id);
CREATE INDEX payments_replaces ON payments (replaces_id);

-- id = derived_id(payment_id, 'allocation/<dr_id>'). A removed allocation
-- has amount_c NULL (rows with derived ids are never deleted).
CREATE TABLE payment_allocations (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    payment_id TEXT,
    dr_id      TEXT,
    amount_c   INTEGER
);
CREATE INDEX payment_allocations_payment ON payment_allocations (payment_id);
CREATE INDEX payment_allocations_dr ON payment_allocations (dr_id);

-- Views --------------------------------------------------------------------

-- Net, VAT and gross of every live DR, from its lines. VAT is rounded half
-- away from zero, like pricing::with_vat.
CREATE VIEW v_dr_totals AS
SELECT t.dr_id, t.client_id, t.status, t.delivered_date, t.net_c, t.vat_c,
       t.net_c + t.vat_c AS gross_c
FROM (
    SELECT r.id AS dr_id, r.client_id, r.status, r.delivered_date, n.net_c,
           CASE WHEN COALESCE(r.vat, 0) = 0 THEN 0
                WHEN n.net_c >= 0 THEN (n.net_c * COALESCE(r.vat_bp, 0) + 5000) / 10000
                ELSE -((-n.net_c * COALESCE(r.vat_bp, 0) + 5000) / 10000)
           END AS vat_c
    FROM delivery_receipts r
    JOIN (
        SELECT d.id, COALESCE(SUM(COALESCE(l.qty, 0) * COALESCE(l.price_c, 0)), 0) AS net_c
        FROM delivery_receipts d
        LEFT JOIN dr_lines l ON l.dr_id = d.id AND l._deleted = 0
        WHERE d._deleted = 0
        GROUP BY d.id
    ) n ON n.id = r.id
    WHERE r._deleted = 0
) t;

-- Payments and whether they count: posted, and for a check, neither
-- bounced nor replaced by a posted payment.
CREATE VIEW v_payment_counts AS
SELECT p.id AS payment_id, p.client_id, COALESCE(p.amount_c, 0) AS amount_c,
       CASE WHEN p.status <> 'posted' THEN 0
            WHEN p.method <> 'check' THEN 1
            WHEN p.check_status = 'bounced' THEN 0
            WHEN EXISTS (SELECT 1 FROM payments n
                         WHERE n.replaces_id = p.id AND n._deleted = 0 AND n.status = 'posted') THEN 0
            ELSE 1
       END AS counts
FROM payments p
WHERE p._deleted = 0;

-- Allocations that count: of a counting payment, to a delivered DR.
CREATE VIEW v_allocations AS
SELECT a.id, a.payment_id, a.dr_id, a.amount_c
FROM payment_allocations a
JOIN v_payment_counts p ON p.payment_id = a.payment_id AND p.counts = 1
JOIN delivery_receipts r ON r.id = a.dr_id AND r._deleted = 0 AND r.status = 'delivered'
WHERE a._deleted = 0 AND a.amount_c IS NOT NULL AND a.amount_c <> 0;

-- Delivered DRs: gross, paid and balance. Balance can be negative if
-- payments entered on PCs that were offline overshoot.
CREATE VIEW v_receivables AS
SELECT t.dr_id, t.client_id, t.delivered_date, t.gross_c,
       COALESCE(a.paid_c, 0) AS paid_c,
       t.gross_c - COALESCE(a.paid_c, 0) AS balance_c
FROM v_dr_totals t
LEFT JOIN (SELECT dr_id, SUM(amount_c) AS paid_c FROM v_allocations GROUP BY dr_id) a
       ON a.dr_id = t.dr_id
WHERE t.status = 'delivered';
