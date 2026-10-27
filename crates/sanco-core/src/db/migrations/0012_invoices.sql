-- M12: a delivery receipt is a DR (no VAT) or an Invoice (VAT). The type is
-- delivery_receipts.vat, so existing DRs with VAT are Invoices from now on.
-- Invoice numbers are series SI (their own BIR booklet); DR numbers stay DR.
-- Invoices made before M12 keep their DR-series number (it is in the DR
-- booklet). Switching a receipt's type frees its number: the number row
-- stays (its id is derived) with seq and no cleared.

-- The live number of each receipt: the DR or SI row that has a number,
-- preferring the series of its type.
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
