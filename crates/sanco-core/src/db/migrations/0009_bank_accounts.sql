-- Bank accounts of the company. Check deposits and bank transfers received
-- (payments.bank_account_id) and TT payments to suppliers
-- (supplier_payments.bank_account_id) name one. payments.deposit_account,
-- typed text before this, stays for checks deposited earlier.

-- Replicated ---------------------------------------------------------------

CREATE TABLE bank_accounts (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name       TEXT,
    bank       TEXT,
    account_no TEXT,
    currency   TEXT,
    notes      TEXT,
    active     INTEGER
);

ALTER TABLE payments ADD COLUMN bank_account_id TEXT;
ALTER TABLE supplier_payments ADD COLUMN bank_account_id TEXT;
CREATE INDEX payments_bank_account ON payments (bank_account_id);
CREATE INDEX supplier_payments_bank_account ON supplier_payments (bank_account_id);
