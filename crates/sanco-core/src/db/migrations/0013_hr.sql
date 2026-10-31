-- HR: employees, weekly attendance, debts (loans and vale) and the weekly
-- payroll paid on Saturday. A payroll week runs Sunday to Saturday and is
-- keyed by its Saturday; attendance is Monday to Saturday.
--
-- Derived ids: attendance = derived_id(employee, date), debt_overrides =
-- derived_id(debt, week), payrolls = derived_id('payroll', week),
-- payroll_lines = derived_id(payroll, employee), payroll_debt_lines =
-- derived_id(line, debt). They are cleared, never deleted.

-- Replicated ---------------------------------------------------------------

CREATE TABLE employees (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name           TEXT,
    position       TEXT,
    daily_rate_c   INTEGER,
    -- Weekly contributions, charged in weeks the employee works.
    sss_c          INTEGER,
    philhealth_c   INTEGER,
    pagibig_c      INTEGER,
    start_date     TEXT,
    active         INTEGER,
    notes          TEXT
);

-- One box of the attendance grid. status: present, absent, holiday (unpaid),
-- paid_holiday, or NULL (not marked).
CREATE TABLE attendance (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    employee_id TEXT,
    date        TEXT,
    status      TEXT,
    note        TEXT
);
CREATE INDEX attendance_date ON attendance (date);
CREATE INDEX attendance_employee ON attendance (employee_id, date);

-- kind: pi_loan, pi_stl, sss_el, sss_sl, vale. weeks is always set;
-- weekly_c only when the weekly amount was typed.
CREATE TABLE employee_debts (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    employee_id TEXT,
    kind        TEXT,
    amount_c    INTEGER,
    start_date  TEXT,
    weeks       INTEGER,
    weekly_c    INTEGER,
    remit_date  TEXT,
    notes       TEXT
);
CREATE INDEX employee_debts_employee ON employee_debts (employee_id);

-- A charge changed on a draft payroll. amount_c NULL = no change.
CREATE TABLE debt_overrides (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    debt_id  TEXT,
    week     TEXT,
    amount_c INTEGER
);
CREATE INDEX debt_overrides_week ON debt_overrides (week);

-- status: posted or voided.
CREATE TABLE payrolls (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    week      TEXT,
    status    TEXT,
    posted_at INTEGER,
    posted_by TEXT
);
CREATE INDEX payrolls_week ON payrolls (week);

-- Snapshot of one employee's pay. included = 0 when a re-post left them out.
CREATE TABLE payroll_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    payroll_id    TEXT,
    employee_id   TEXT,
    name          TEXT,
    daily_rate_c  INTEGER,
    days          INTEGER,
    gross_c       INTEGER,
    sss_c         INTEGER,
    philhealth_c  INTEGER,
    pagibig_c     INTEGER,
    net_c         INTEGER,
    included      INTEGER
);
CREATE INDEX payroll_lines_payroll ON payroll_lines (payroll_id);
CREATE INDEX payroll_lines_employee ON payroll_lines (employee_id);

-- What a posted payroll took from a debt. amount_c 0 when cleared.
CREATE TABLE payroll_debt_lines (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    payroll_id      TEXT,
    payroll_line_id TEXT,
    debt_id         TEXT,
    kind            TEXT,
    amount_c        INTEGER
);
CREATE INDEX payroll_debt_lines_debt ON payroll_debt_lines (debt_id);
CREATE INDEX payroll_debt_lines_line ON payroll_debt_lines (payroll_line_id);

-- Local ------------------------------------------------------------------

-- Deductions from posted payrolls, per debt and week.
CREATE VIEW v_debt_paid AS
SELECT x.debt_id, p.week, SUM(COALESCE(x.amount_c, 0)) AS amount_c
FROM payroll_debt_lines x
JOIN payroll_lines l ON l.id = x.payroll_line_id AND l._deleted = 0 AND COALESCE(l.included, 1) <> 0
JOIN payrolls p ON p.id = x.payroll_id AND p._deleted = 0 AND p.status = 'posted'
WHERE x._deleted = 0
GROUP BY x.debt_id, p.week;

-- Balance left on each debt.
CREATE VIEW v_debt_balance AS
SELECT d.id AS debt_id, COALESCE(d.amount_c, 0) AS amount_c,
       COALESCE((SELECT SUM(v.amount_c) FROM v_debt_paid v WHERE v.debt_id = d.id), 0) AS paid_c,
       COALESCE(d.amount_c, 0)
         - COALESCE((SELECT SUM(v.amount_c) FROM v_debt_paid v WHERE v.debt_id = d.id), 0) AS balance_c
FROM employee_debts d
WHERE d._deleted = 0;
