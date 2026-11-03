-- M15: a DR or invoice is the client's order (the Booking module is now the
-- DR and invoice screen). Receipts get the client's PO/reference; their
-- lines get the price rule and a note, which the booking lines used to hold.
-- Existing receipts keep working: status 'requested'/'approved' both read as
-- "new". Bookings and their tables stay (older peers still sync them).
ALTER TABLE delivery_receipts ADD COLUMN client_ref TEXT;
ALTER TABLE dr_lines ADD COLUMN price_rule TEXT;
ALTER TABLE dr_lines ADD COLUMN notes TEXT;
