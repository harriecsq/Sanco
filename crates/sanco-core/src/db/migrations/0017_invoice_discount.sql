-- 1.9: a discount on the whole invoice (Sales Invoices only, not DRs).
--
-- Either a percentage (discount_bp, basis points: 5% = 500) or a set peso
-- amount (discount_c, centavos); at most one is set. Both NULL = none.
-- The discount comes off the lines' subtotal; the client owes the rest.
-- A DR ignores both (switching an invoice to a DR clears them).
ALTER TABLE delivery_receipts ADD COLUMN discount_bp INTEGER;
ALTER TABLE delivery_receipts ADD COLUMN discount_c INTEGER;

-- v_dr_totals gains subtotal_c (the lines) and discount_c (what comes off
-- it); net_c is now after the discount. Same rounding as
-- pricing::discount_of: half away from zero, never more than the subtotal.
DROP VIEW v_receivables;
DROP VIEW v_dr_totals;

CREATE VIEW v_dr_totals AS
SELECT t.dr_id, t.client_id, t.status, t.delivered_date, t.subtotal_c, t.discount_c, t.net_c,
       t.vat_c, t.net_c + t.vat_c AS gross_c
FROM (
    SELECT d.dr_id, d.client_id, d.status, d.delivered_date, d.subtotal_c, d.discount_c, d.net_c,
           CASE WHEN COALESCE(d.vat, 0) = 0 THEN 0
                WHEN d.net_c >= 0 THEN (d.net_c * COALESCE(d.vat_bp, 0) + 5000) / 10000
                ELSE -((-d.net_c * COALESCE(d.vat_bp, 0) + 5000) / 10000)
           END AS vat_c
    FROM (
        SELECT s.*, s.subtotal_c - s.discount_c AS net_c
        FROM (
            SELECT r.id AS dr_id, r.client_id, r.status, r.delivered_date, r.vat, r.vat_bp,
                   n.subtotal_c,
                   CASE WHEN COALESCE(r.vat, 0) = 0 OR n.subtotal_c <= 0 THEN 0
                        WHEN r.discount_bp IS NOT NULL
                            THEN MIN(n.subtotal_c, MAX(0, (n.subtotal_c * r.discount_bp + 5000) / 10000))
                        WHEN r.discount_c IS NOT NULL
                            THEN MIN(n.subtotal_c, MAX(0, r.discount_c))
                        ELSE 0
                   END AS discount_c
            FROM delivery_receipts r
            JOIN (
                SELECT d.id, COALESCE(SUM(COALESCE(l.qty, 0) * COALESCE(l.price_c, 0)), 0) AS subtotal_c
                FROM delivery_receipts d
                LEFT JOIN dr_lines l ON l.dr_id = d.id AND l._deleted = 0
                WHERE d._deleted = 0
                GROUP BY d.id
            ) n ON n.id = r.id
            WHERE r._deleted = 0
        ) s
    ) d
) t;

CREATE VIEW v_receivables AS
SELECT t.dr_id, t.client_id, t.delivered_date, t.gross_c,
       COALESCE(a.paid_c, 0) AS paid_c,
       t.gross_c - COALESCE(a.paid_c, 0) AS balance_c
FROM v_dr_totals t
LEFT JOIN (SELECT dr_id, SUM(amount_c) AS paid_c FROM v_allocations GROUP BY dr_id) a
       ON a.dr_id = t.dr_id
WHERE t.status = 'delivered';
