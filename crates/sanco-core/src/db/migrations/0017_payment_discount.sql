-- 1.9: a discount given when an invoice is paid (Sales Invoices only, not
-- DRs). Each allocation of a payment to an invoice may carry one: a
-- percentage of the invoice's total (discount_bp, basis points: 5% = 500)
-- or a set peso amount (discount_c, centavos); at most one is set. It
-- settles that much of the invoice on top of the money allocated
-- (amount_c), and counts only while the payment counts (posted, not a
-- bounced or replaced check). A removed allocation (amount_c NULL) has
-- neither.
ALTER TABLE payment_allocations ADD COLUMN discount_bp INTEGER;
ALTER TABLE payment_allocations ADD COLUMN discount_c INTEGER;

DROP VIEW v_receivables;
DROP VIEW v_allocations;

-- Allocations that count: of a counting payment, to a delivered DR.
-- amount_c is money; discount_c what the discount settles (invoices only),
-- rounded half away from zero and never more than the invoice's total,
-- like pricing::Discount::of; settled_c is both.
CREATE VIEW v_allocations AS
SELECT d.id, d.payment_id, d.dr_id, d.amount_c, d.discount_c,
       d.amount_c + d.discount_c AS settled_c
FROM (
    SELECT a.id, a.payment_id, a.dr_id, a.amount_c,
           CASE WHEN COALESCE(r.vat, 0) = 0 OR t.gross_c <= 0 THEN 0
                WHEN a.discount_bp IS NOT NULL
                    THEN MIN(t.gross_c, MAX(0, (t.gross_c * a.discount_bp + 5000) / 10000))
                WHEN a.discount_c IS NOT NULL THEN MIN(t.gross_c, MAX(0, a.discount_c))
                ELSE 0
           END AS discount_c
    FROM payment_allocations a
    JOIN v_payment_counts p ON p.payment_id = a.payment_id AND p.counts = 1
    JOIN delivery_receipts r ON r.id = a.dr_id AND r._deleted = 0 AND r.status = 'delivered'
    JOIN v_dr_totals t ON t.dr_id = a.dr_id
    WHERE a._deleted = 0 AND a.amount_c IS NOT NULL
) d
WHERE d.amount_c <> 0 OR d.discount_c <> 0;

-- Delivered DRs: gross, money paid, discount given and balance. Balance
-- can be negative if payments entered on PCs that were offline overshoot.
CREATE VIEW v_receivables AS
SELECT t.dr_id, t.client_id, t.delivered_date, t.gross_c,
       COALESCE(a.paid_c, 0) AS paid_c,
       COALESCE(a.discount_c, 0) AS discount_c,
       t.gross_c - COALESCE(a.paid_c, 0) - COALESCE(a.discount_c, 0) AS balance_c
FROM v_dr_totals t
LEFT JOIN (SELECT dr_id, SUM(amount_c) AS paid_c, SUM(discount_c) AS discount_c
           FROM v_allocations GROUP BY dr_id) a
       ON a.dr_id = t.dr_id
WHERE t.status = 'delivered';
