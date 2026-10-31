//! HR: employees, weekly attendance, debts (loans and vale) and the weekly
//! payroll.
//!
//! A payroll week runs Sunday to Saturday and is keyed by its Saturday (the
//! payday); attendance is kept Monday to Saturday. Pay is the daily rate ×
//! paid days: days present plus paid holidays. An employee with no paid day
//! gets nothing that week: no pay, no contributions, no debt charges. An
//! unpaid holiday is like an absence, but neither kind of holiday is counted
//! as one.
//!
//! Debts are paid in weekly charges from the first Saturday on or after the
//! start date. The weekly amount is typed, or the amount ÷ weeks rounded
//! down to ₱0.05; the last week takes the rest. Nothing about the schedule
//! is stored: each week's charge comes from the balance (see
//! [`scheduled_charge`]), so what a skipped or reduced week left behind is
//! spread over the weeks still to come. Balances are what posted payrolls
//! deducted (`v_debt_balance`).
//!
//! A payroll is computed live until it is posted; posting stores a snapshot
//! and locks that week's attendance. A posted payroll is voided to change it.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::{date_opt, peso, require_date, require_text, text_opt};
use crate::error::{Error, Result};
use crate::ids::derived_id;
use crate::repo::WriteTx;
use crate::time::{add_days, days_between, manila_day_start_ms, parse_date};

pub const EMPLOYEES: &str = "employees";
pub const ATTENDANCE: &str = "attendance";
pub const DEBTS: &str = "employee_debts";
pub const DEBT_OVERRIDES: &str = "debt_overrides";
pub const PAYROLLS: &str = "payrolls";
pub const PAYROLL_LINES: &str = "payroll_lines";
pub const PAYROLL_DEBT_LINES: &str = "payroll_debt_lines";

/// Debt amounts and charges are multiples of ₱0.05.
pub const STEP_C: i64 = 5;
/// Longest schedule a debt may have.
pub const MAX_WEEKS: i64 = 520;

// Weeks ----------------------------------------------------------------------

/// Day of the week, Monday = 0 … Sunday = 6.
pub fn weekday(date: &str) -> Option<i64> {
    let days = (manila_day_start_ms(date)? + crate::time::MANILA_OFFSET_MS).div_euclid(86_400_000);
    // 1970-01-01 was a Thursday.
    Some((days + 3).rem_euclid(7))
}

/// The payroll week a date falls in: the Saturday on or after it.
pub fn week_of(date: &str) -> Option<String> {
    let date = parse_date(date)?;
    add_days(&date, (5 - weekday(&date)?).rem_euclid(7))
}

/// A valid Saturday, normalized.
pub fn require_week(week: &str) -> Result<String> {
    parse_date(week)
        .filter(|d| weekday(d) == Some(5))
        .ok_or_else(|| Error::Invalid("a payroll week is given by its Saturday".into()))
}

/// Monday to Saturday of a week.
pub fn week_days(week: &str) -> Vec<String> {
    (0..6).filter_map(|i| add_days(week, i - 5)).collect()
}

/// Whole weeks from one Saturday to another.
fn weeks_between(from: &str, to: &str) -> i64 {
    days_between(from, to).unwrap_or(0).div_euclid(7)
}

fn floor_step(c: i64) -> i64 {
    c - c.rem_euclid(STEP_C)
}

fn check_step(c: i64, what: &str) -> Result<()> {
    if c % STEP_C != 0 {
        return Err(Error::Invalid(format!(
            "{what} must be in steps of ₱0.05 (e.g. ₱478.50 or ₱288.25)"
        )));
    }
    Ok(())
}

// Kinds ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum AttendanceStatus {
    Present,
    Absent,
    /// An unpaid holiday.
    Holiday,
    /// A holiday paid at the daily rate, like a day present.
    PaidHoliday,
}

impl AttendanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Holiday => "holiday",
            Self::PaidHoliday => "paid_holiday",
        }
    }

    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s? {
            "present" => Some(Self::Present),
            "absent" => Some(Self::Absent),
            "holiday" => Some(Self::Holiday),
            "paid_holiday" => Some(Self::PaidHoliday),
            _ => None,
        }
    }
}

/// Each kind is its own payroll column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum DebtKind {
    PiLoan,
    PiStl,
    SssEl,
    SssSl,
    Vale,
}

impl DebtKind {
    pub const ALL: [DebtKind; 5] = [
        DebtKind::PiLoan,
        DebtKind::PiStl,
        DebtKind::SssEl,
        DebtKind::SssSl,
        DebtKind::Vale,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PiLoan => "pi_loan",
            Self::PiStl => "pi_stl",
            Self::SssEl => "sss_el",
            Self::SssSl => "sss_sl",
            Self::Vale => "vale",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PiLoan => "PI Loan",
            Self::PiStl => "PI STL",
            Self::SssEl => "SSS EL",
            Self::SssSl => "SSS SL",
            Self::Vale => "Vale",
        }
    }

    pub fn parse(s: Option<&str>) -> Option<Self> {
        Self::ALL.into_iter().find(|k| Some(k.as_str()) == s)
    }
}

// Employees ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Employee {
    pub id: String,
    pub name: String,
    pub position: Option<String>,
    pub daily_rate_c: i64,
    /// Weekly contributions.
    pub sss_c: i64,
    pub philhealth_c: i64,
    pub pagibig_c: i64,
    pub start_date: Option<String>,
    pub active: bool,
    pub notes: Option<String>,
    /// Balance left on all their debts.
    pub debt_balance_c: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct EmployeeInput {
    /// `None` adds an employee.
    pub id: Option<String>,
    pub name: String,
    pub position: String,
    pub daily_rate_c: i64,
    pub sss_c: i64,
    pub philhealth_c: i64,
    pub pagibig_c: i64,
    pub start_date: Option<String>,
    pub active: bool,
    pub notes: String,
}

/// Every employee, active first, by name.
pub fn list_employees(conn: &Connection) -> Result<Vec<Employee>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.name, e.position, COALESCE(e.daily_rate_c, 0), COALESCE(e.sss_c, 0),
                COALESCE(e.philhealth_c, 0), COALESCE(e.pagibig_c, 0), e.start_date,
                COALESCE(e.active, 1), e.notes,
                COALESCE((SELECT SUM(b.balance_c) FROM v_debt_balance b
                          JOIN employee_debts d ON d.id = b.debt_id
                          WHERE d.employee_id = e.id), 0)
         FROM employees e WHERE e._deleted = 0
         ORDER BY COALESCE(e.active, 1) DESC, e.name COLLATE NOCASE, e.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Employee {
            id: r.get(0)?,
            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            position: r.get(2)?,
            daily_rate_c: r.get(3)?,
            sss_c: r.get(4)?,
            philhealth_c: r.get(5)?,
            pagibig_c: r.get(6)?,
            start_date: r.get(7)?,
            active: r.get::<_, i64>(8)? != 0,
            notes: r.get(9)?,
            debt_balance_c: r.get(10)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get_employee(conn: &Connection, id: &str) -> Result<Option<Employee>> {
    Ok(list_employees(conn)?.into_iter().find(|e| e.id == id))
}

fn require_employee(conn: &Connection, id: &str) -> Result<String> {
    conn.query_row(
        "SELECT COALESCE(name, '') FROM employees WHERE id = ?1 AND _deleted = 0",
        [id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::Invalid("that employee no longer exists".into()))
}

pub fn save_employee(tx: &mut WriteTx, input: &EmployeeInput) -> Result<String> {
    let name = require_text(&input.name, "the employee's name")?;
    for (c, what) in [
        (input.daily_rate_c, "the daily rate"),
        (input.sss_c, "SSS"),
        (input.philhealth_c, "PhilHealth"),
        (input.pagibig_c, "Pag-IBIG"),
    ] {
        if c < 0 {
            return Err(Error::Invalid(format!("{what} can't be negative")));
        }
    }
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM employees WHERE _deleted = 0
                        AND name = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        (&name, &input.id),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!(
            "an employee named {name} already exists"
        )));
    }
    let data = json!({
        "name": name,
        "position": text_opt(&input.position),
        "daily_rate_c": input.daily_rate_c,
        "sss_c": input.sss_c,
        "philhealth_c": input.philhealth_c,
        "pagibig_c": input.pagibig_c,
        "start_date": date_opt(input.start_date.as_deref(), "start date")?,
        "active": input.active,
        "notes": text_opt(&input.notes),
    });
    match &input.id {
        Some(id) => {
            require_employee(tx.conn(), id)?;
            tx.update(EMPLOYEES, id, data)?;
            Ok(id.clone())
        }
        None => tx.insert(EMPLOYEES, data),
    }
}

/// Deletes an employee with no attendance, debts or payroll. Others are
/// made inactive instead.
pub fn delete_employee(tx: &mut WriteTx, id: &str) -> Result<()> {
    require_employee(tx.conn(), id)?;
    let used: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM attendance WHERE employee_id = ?1 AND _deleted = 0
                        AND status IS NOT NULL)
             OR EXISTS (SELECT 1 FROM employee_debts WHERE employee_id = ?1 AND _deleted = 0)
             OR EXISTS (SELECT 1 FROM payroll_lines WHERE employee_id = ?1 AND _deleted = 0)",
        [id],
        |r| r.get(0),
    )?;
    if used {
        return Err(Error::Invalid(
            "this employee has attendance, debts or payroll; mark them inactive instead".into(),
        ));
    }
    tx.delete(EMPLOYEES, id)
}

// Debt schedule --------------------------------------------------------------

/// What a debt's amount, start and weeks (or typed weekly amount) work out to.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DebtPlanInput {
    pub amount_c: i64,
    pub start_date: String,
    /// Number of weeks, when the weekly amount isn't typed.
    pub weeks: Option<i64>,
    /// A typed weekly amount; the number of weeks follows from it.
    pub weekly_c: Option<i64>,
    pub remit_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DebtPlan {
    pub weeks: i64,
    /// Charged each week but the last.
    pub weekly_c: i64,
    /// The last week's charge (the rest).
    pub last_c: i64,
    /// Saturday of the first and last charge.
    pub first_week: String,
    pub last_week: String,
    /// The last week falls after the remit date.
    pub remit_warning: bool,
}

pub fn debt_plan(input: &DebtPlanInput) -> Result<DebtPlan> {
    let amount = input.amount_c;
    if amount <= 0 {
        return Err(Error::Invalid("enter the amount".into()));
    }
    check_step(amount, "the amount")?;
    let start = require_date(&input.start_date, "start date")?;
    let first_week = week_of(&start).ok_or_else(|| Error::Invalid("bad start date".into()))?;
    let (weeks, weekly) = match (input.weekly_c.filter(|w| *w > 0), input.weeks) {
        (Some(w), _) => {
            check_step(w, "the weekly amount")?;
            if w > amount {
                return Err(Error::Invalid(
                    "the weekly amount is more than the whole amount".into(),
                ));
            }
            ((amount + w - 1) / w, w)
        }
        (None, Some(n)) if (1..=MAX_WEEKS).contains(&n) => {
            let w = floor_step(amount / n);
            if w <= 0 {
                return Err(Error::Invalid(
                    "that's less than ₱0.05 a week; use fewer weeks".into(),
                ));
            }
            (n, w)
        }
        (None, Some(_)) => {
            return Err(Error::Invalid(format!(
                "the number of weeks must be 1 to {MAX_WEEKS}"
            )));
        }
        (None, None) => {
            return Err(Error::Invalid(
                "enter the number of weeks or the weekly amount".into(),
            ));
        }
    };
    if weeks > MAX_WEEKS {
        return Err(Error::Invalid(format!(
            "that takes more than {MAX_WEEKS} weeks; raise the weekly amount"
        )));
    }
    let last_week = add_days(&first_week, 7 * (weeks - 1)).unwrap_or_else(|| first_week.clone());
    let remit = date_opt(input.remit_date.as_deref(), "remit date")?;
    Ok(DebtPlan {
        weeks,
        weekly_c: weekly,
        last_c: amount - weekly * (weeks - 1),
        remit_warning: remit.is_some_and(|r| last_week > r),
        first_week,
        last_week,
    })
}

/// A debt's schedule, as stored.
#[derive(Debug, Clone)]
struct Terms {
    amount_c: i64,
    weekly_c: i64,
    first_week: String,
    last_week: String,
}

/// This week's charge on a debt with `balance` left before it.
///
/// - Before the first week, or once paid: nothing.
/// - From the last week on: the whole balance.
/// - On schedule (or ahead): the weekly amount.
/// - Behind (a week skipped or reduced): the weekly amount plus the
///   shortfall spread over the weeks left, rounded down to ₱0.05; the last
///   week takes what rounding left.
fn scheduled_charge(t: &Terms, balance: i64, week: &str) -> i64 {
    if balance <= 0 || week < t.first_week.as_str() {
        return 0;
    }
    if week >= t.last_week.as_str() {
        return balance;
    }
    let k = weeks_between(&t.first_week, week);
    let on_schedule = t.amount_c - k * t.weekly_c;
    let charge = if balance <= on_schedule {
        t.weekly_c
    } else {
        let left = weeks_between(week, &t.last_week) + 1;
        t.weekly_c + floor_step((balance - on_schedule) / left)
    };
    charge.min(balance)
}

// Debts ----------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DebtInput {
    /// `None` adds a debt.
    pub id: Option<String>,
    pub employee_id: String,
    pub kind: DebtKind,
    pub amount_c: i64,
    pub start_date: String,
    pub weeks: Option<i64>,
    pub weekly_c: Option<i64>,
    pub remit_date: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum DebtStatus {
    Active,
    Paid,
    /// Balance left after its last week.
    OverdueSchedule,
    /// Balance left after the remit date.
    PastRemit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DebtInfo {
    pub id: String,
    pub employee_id: String,
    pub employee_name: String,
    pub kind: DebtKind,
    pub amount_c: i64,
    pub paid_c: i64,
    pub balance_c: i64,
    pub start_date: String,
    pub weeks: i64,
    /// The weekly amount (typed or computed).
    pub weekly_c: i64,
    /// The weekly amount was typed.
    pub weekly_typed: bool,
    pub last_c: i64,
    pub first_week: String,
    pub last_week: String,
    pub remit_date: Option<String>,
    /// The last week falls after the remit date.
    pub remit_warning: bool,
    pub status: DebtStatus,
    /// This week's charge: posted, or what the draft payroll would take if
    /// the employee works.
    pub this_week_c: i64,
    pub notes: Option<String>,
}

struct DebtRow {
    id: String,
    employee_id: String,
    employee_name: String,
    kind: DebtKind,
    terms: Terms,
    weeks: i64,
    weekly_typed: bool,
    start_date: String,
    remit_date: Option<String>,
    notes: Option<String>,
}

fn load_debts(conn: &Connection, employee: Option<&str>) -> Result<Vec<DebtRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT d.id, d.employee_id, COALESCE(e.name, ''), d.kind, COALESCE(d.amount_c, 0),
                COALESCE(d.start_date, ''), COALESCE(d.weeks, 1), d.weekly_c, d.remit_date, d.notes
         FROM employee_debts d
         LEFT JOIN employees e ON e.id = d.employee_id
         WHERE d._deleted = 0 AND (?1 IS NULL OR d.employee_id = ?1)
         ORDER BY e.name COLLATE NOCASE, d.start_date, d.id",
    )?;
    let rows = stmt.query_map([employee], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, Option<i64>>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, employee_id, employee_name, kind, amount, start, weeks, typed, remit, notes) =
            row?;
        let weeks = weeks.max(1);
        let first_week = week_of(&start).unwrap_or_default();
        let weekly = typed
            .filter(|w| *w > 0)
            .unwrap_or_else(|| floor_step(amount / weeks));
        out.push(DebtRow {
            id,
            employee_id,
            employee_name,
            kind: DebtKind::parse(kind.as_deref()).unwrap_or(DebtKind::Vale),
            terms: Terms {
                amount_c: amount,
                weekly_c: weekly,
                last_week: add_days(&first_week, 7 * (weeks - 1)).unwrap_or_default(),
                first_week,
            },
            weeks,
            weekly_typed: typed.is_some_and(|w| w > 0),
            start_date: start,
            remit_date: remit,
            notes,
        });
    }
    Ok(out)
}

/// Deducted by posted payrolls: debt → week → amount.
fn paid_by_week(conn: &Connection) -> Result<BTreeMap<String, BTreeMap<String, i64>>> {
    let mut stmt = conn
        .prepare_cached("SELECT debt_id, week, amount_c FROM v_debt_paid WHERE week IS NOT NULL")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut out: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for row in rows {
        let (debt, week, c) = row?;
        *out.entry(debt).or_default().entry(week).or_default() += c;
    }
    Ok(out)
}

/// Charges changed on draft payrolls: (debt, week) → amount.
fn overrides(conn: &Connection, week: Option<&str>) -> Result<BTreeMap<(String, String), i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT debt_id, week, amount_c FROM debt_overrides
         WHERE _deleted = 0 AND amount_c IS NOT NULL AND (?1 IS NULL OR week = ?1)",
    )?;
    let rows = stmt.query_map([week], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn posted(conn: &Connection, week: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM payrolls WHERE id = ?1 AND _deleted = 0
                        AND status = 'posted')",
        [payroll_id(week)],
        |r| r.get(0),
    )?)
}

/// Balance before `week`: everything not deducted in other weeks.
fn balance_outside(d: &DebtRow, paid: &BTreeMap<String, BTreeMap<String, i64>>, week: &str) -> i64 {
    let other: i64 = paid
        .get(&d.id)
        .map(|m| {
            m.iter()
                .filter(|(w, _)| w.as_str() != week)
                .map(|(_, c)| c)
                .sum()
        })
        .unwrap_or(0);
    d.terms.amount_c - other
}

/// What a draft payroll for `week` charges a debt, before the "did they
/// work" check: the changed amount if any, else the schedule.
fn draft_charge(
    d: &DebtRow,
    paid: &BTreeMap<String, BTreeMap<String, i64>>,
    over: &BTreeMap<(String, String), i64>,
    week: &str,
) -> (i64, i64, Option<i64>) {
    let balance = balance_outside(d, paid, week);
    let scheduled = scheduled_charge(&d.terms, balance, week);
    let changed = over
        .get(&(d.id.clone(), week.to_string()))
        .map(|c| (*c).clamp(0, balance.max(0)));
    (balance, scheduled, changed)
}

fn debt_info(
    d: DebtRow,
    paid: &BTreeMap<String, BTreeMap<String, i64>>,
    over: &BTreeMap<(String, String), i64>,
    today: &str,
    this_week: &str,
    this_week_posted: bool,
) -> DebtInfo {
    let paid_c: i64 = paid.get(&d.id).map(|m| m.values().sum()).unwrap_or(0);
    let balance = d.terms.amount_c - paid_c;
    let this_week_c = if this_week_posted {
        paid.get(&d.id)
            .and_then(|m| m.get(this_week))
            .copied()
            .unwrap_or(0)
    } else {
        let (_, scheduled, changed) = draft_charge(&d, paid, over, this_week);
        changed.unwrap_or(scheduled)
    };
    let status = if balance <= 0 {
        DebtStatus::Paid
    } else if d.remit_date.as_deref().is_some_and(|r| today > r) {
        DebtStatus::PastRemit
    } else if this_week > d.terms.last_week.as_str() {
        DebtStatus::OverdueSchedule
    } else {
        DebtStatus::Active
    };
    DebtInfo {
        remit_warning: d
            .remit_date
            .as_deref()
            .is_some_and(|r| d.terms.last_week.as_str() > r),
        last_c: d.terms.amount_c - d.terms.weekly_c * (d.weeks - 1),
        id: d.id,
        employee_id: d.employee_id,
        employee_name: d.employee_name,
        kind: d.kind,
        amount_c: d.terms.amount_c,
        paid_c,
        balance_c: balance,
        start_date: d.start_date,
        weeks: d.weeks,
        weekly_c: d.terms.weekly_c,
        weekly_typed: d.weekly_typed,
        first_week: d.terms.first_week,
        last_week: d.terms.last_week,
        remit_date: d.remit_date,
        status,
        this_week_c,
        notes: d.notes,
    }
}

/// Debts (of one employee, or everyone's), with balances as of `today`.
pub fn list_debts(conn: &Connection, employee: Option<&str>, today: &str) -> Result<Vec<DebtInfo>> {
    let this_week = week_of(today).unwrap_or_default();
    let is_posted = posted(conn, &this_week)?;
    let paid = paid_by_week(conn)?;
    let over = overrides(conn, Some(&this_week))?;
    Ok(load_debts(conn, employee)?
        .into_iter()
        .map(|d| debt_info(d, &paid, &over, today, &this_week, is_posted))
        .collect())
}

pub fn save_debt(tx: &mut WriteTx, input: &DebtInput) -> Result<String> {
    require_employee(tx.conn(), &input.employee_id)?;
    let plan = debt_plan(&DebtPlanInput {
        amount_c: input.amount_c,
        start_date: input.start_date.clone(),
        weeks: input.weeks,
        weekly_c: input.weekly_c,
        remit_date: input.remit_date.clone(),
    })?;
    let typed = input.weekly_c.filter(|w| *w > 0);
    let data = json!({
        "employee_id": input.employee_id,
        "kind": input.kind.as_str(),
        "amount_c": input.amount_c,
        "start_date": require_date(&input.start_date, "start date")?,
        "weeks": plan.weeks,
        "weekly_c": typed,
        "remit_date": date_opt(input.remit_date.as_deref(), "remit date")?,
        "notes": text_opt(&input.notes),
    });
    match &input.id {
        Some(id) => {
            let old = tx
                .get(DEBTS, id)?
                .ok_or_else(|| Error::Invalid("that debt no longer exists".into()))?;
            let paid = debt_paid(tx.conn(), id)?;
            if paid > 0 {
                if input.amount_c < paid {
                    return Err(Error::Invalid(format!(
                        "{} has already been deducted; the amount can't be less",
                        peso(paid)
                    )));
                }
                if old.get("employee_id").and_then(|v| v.as_str()) != Some(&input.employee_id) {
                    return Err(Error::Invalid(
                        "deductions were already made; the employee can't change".into(),
                    ));
                }
            }
            tx.update(DEBTS, id, data)?;
            Ok(id.clone())
        }
        None => tx.insert(DEBTS, data),
    }
}

fn debt_paid(conn: &Connection, id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(amount_c), 0) FROM v_debt_paid WHERE debt_id = ?1",
        [id],
        |r| r.get(0),
    )?)
}

/// Deletes a debt nothing has been deducted from yet.
pub fn delete_debt(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(DEBTS, id)?.is_none() {
        return Err(Error::Invalid("that debt no longer exists".into()));
    }
    if debt_paid(tx.conn(), id)? > 0 {
        return Err(Error::Invalid(
            "deductions were already made from this debt; it can't be deleted".into(),
        ));
    }
    tx.delete(DEBTS, id)
}

/// Changes what a draft payroll charges a debt (`None` = back to the
/// schedule). What's left over is spread over the following weeks.
pub fn set_debt_charge(
    tx: &mut WriteTx,
    debt_id: &str,
    week: &str,
    amount_c: Option<i64>,
) -> Result<()> {
    let week = require_week(week)?;
    if posted(tx.conn(), &week)? {
        return Err(Error::Invalid(
            "this week's payroll is posted; void it to change deductions".into(),
        ));
    }
    let debt = load_debts(tx.conn(), None)?
        .into_iter()
        .find(|d| d.id == debt_id)
        .ok_or_else(|| Error::Invalid("that debt no longer exists".into()))?;
    if let Some(c) = amount_c {
        check_step(c, "the deduction")?;
        let balance = balance_outside(&debt, &paid_by_week(tx.conn())?, &week);
        if c < 0 || c > balance {
            return Err(Error::Invalid(format!(
                "the deduction must be between ₱0.00 and the balance, {}",
                peso(balance)
            )));
        }
    }
    let id = derived_id(debt_id, &format!("charge:{week}"));
    tx.upsert(
        DEBT_OVERRIDES,
        &id,
        json!({ "debt_id": debt_id, "week": week, "amount_c": amount_c }),
    )
}

// Attendance -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AttendanceCell {
    pub date: String,
    pub status: Option<AttendanceStatus>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AttendanceRow {
    pub employee_id: String,
    pub name: String,
    pub active: bool,
    /// Monday to Saturday.
    pub cells: Vec<AttendanceCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AttendanceWeek {
    pub week: String,
    pub days: Vec<String>,
    /// The week's payroll is posted: attendance is locked.
    pub posted: bool,
    pub rows: Vec<AttendanceRow>,
}

type Marks = BTreeMap<(String, String), (Option<AttendanceStatus>, Option<String>)>;

fn marks(conn: &Connection, from: &str, to: &str) -> Result<Marks> {
    let mut stmt = conn.prepare_cached(
        "SELECT employee_id, date, status, note FROM attendance
         WHERE _deleted = 0 AND date BETWEEN ?1 AND ?2",
    )?;
    let rows = stmt.query_map([from, to], |r| {
        Ok((
            (
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ),
            (
                AttendanceStatus::parse(r.get::<_, Option<String>>(2)?.as_deref()),
                r.get(3)?,
            ),
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Active employees, plus inactive ones with a mark that week.
fn week_employees(conn: &Connection, marks: &Marks) -> Result<Vec<Employee>> {
    let marked: BTreeSet<&str> = marks
        .iter()
        .filter(|(_, (s, _))| s.is_some())
        .map(|((e, _), _)| e.as_str())
        .collect();
    let mut list: Vec<Employee> = list_employees(conn)?
        .into_iter()
        .filter(|e| e.active || marked.contains(e.id.as_str()))
        .collect();
    list.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(list)
}

pub fn attendance_week(conn: &Connection, week: &str) -> Result<AttendanceWeek> {
    let week = require_week(week)?;
    let days = week_days(&week);
    let marks = marks(conn, &days[0], &week)?;
    let rows = week_employees(conn, &marks)?
        .into_iter()
        .map(|e| AttendanceRow {
            cells: days
                .iter()
                .map(|d| {
                    let (status, note) = marks
                        .get(&(e.id.clone(), d.clone()))
                        .cloned()
                        .unwrap_or((None, None));
                    AttendanceCell {
                        date: d.clone(),
                        status,
                        note,
                    }
                })
                .collect(),
            employee_id: e.id,
            name: e.name,
            active: e.active,
        })
        .collect();
    Ok(AttendanceWeek {
        posted: posted(conn, &week)?,
        week,
        days,
        rows,
    })
}

fn check_day(conn: &Connection, date: &str) -> Result<String> {
    let date = require_date(date, "date")?;
    if weekday(&date) == Some(6) {
        return Err(Error::Invalid("attendance is Monday to Saturday".into()));
    }
    if posted(conn, &week_of(&date).unwrap_or_default())? {
        return Err(Error::Invalid(
            "that week's payroll is posted; void it to change attendance".into(),
        ));
    }
    Ok(date)
}

fn put_mark(
    tx: &mut WriteTx,
    employee_id: &str,
    date: &str,
    status: Option<AttendanceStatus>,
    note: Option<String>,
) -> Result<()> {
    // A note only goes with an absence or a holiday.
    let note = note.filter(|_| {
        matches!(
            status,
            Some(
                AttendanceStatus::Absent
                    | AttendanceStatus::Holiday
                    | AttendanceStatus::PaidHoliday
            )
        )
    });
    tx.upsert(
        ATTENDANCE,
        &derived_id(employee_id, &format!("attendance:{date}")),
        json!({
            "employee_id": employee_id,
            "date": date,
            "status": status.map(AttendanceStatus::as_str),
            "note": note,
        }),
    )
}

/// Sets one box. `None` = not marked.
pub fn set_attendance(
    tx: &mut WriteTx,
    employee_id: &str,
    date: &str,
    status: Option<AttendanceStatus>,
    note: &str,
) -> Result<()> {
    let date = check_day(tx.conn(), date)?;
    require_employee(tx.conn(), employee_id)?;
    put_mark(tx, employee_id, &date, status, text_opt(note))
}

/// Marks a day for every active employee: all of them (a holiday), or only
/// the boxes not marked yet (`only_unmarked`, e.g. everyone present).
pub fn mark_day(
    tx: &mut WriteTx,
    date: &str,
    status: AttendanceStatus,
    note: &str,
    only_unmarked: bool,
) -> Result<()> {
    let date = check_day(tx.conn(), date)?;
    let marks = marks(tx.conn(), &date, &date)?;
    let employees: Vec<String> = list_employees(tx.conn())?
        .into_iter()
        .filter(|e| e.active)
        .map(|e| e.id)
        .collect();
    for e in employees {
        let current = marks.get(&(e.clone(), date.clone())).and_then(|m| m.0);
        if only_unmarked && current.is_some() {
            continue;
        }
        put_mark(tx, &e, &date, Some(status), text_opt(note))?;
    }
    Ok(())
}

// Payroll --------------------------------------------------------------------

pub fn payroll_id(week: &str) -> String {
    derived_id("payroll", week)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PayrollStatus {
    Draft,
    Posted,
    Voided,
}

/// One debt's charge on a payroll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PayrollDebt {
    pub debt_id: String,
    pub kind: DebtKind,
    /// Charged.
    pub amount_c: i64,
    /// What the schedule asks (drafts).
    pub scheduled_c: i64,
    /// The charge was changed by hand (drafts).
    pub changed: bool,
    /// Balance before this week.
    pub balance_c: i64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PayrollRow {
    pub employee_id: String,
    pub name: String,
    pub daily_rate_c: i64,
    /// Paid days: present plus paid holidays.
    pub days: i64,
    pub absent: i64,
    /// Unpaid holidays.
    pub holidays: i64,
    pub paid_holidays: i64,
    /// Boxes not marked.
    pub unmarked: i64,
    /// At least one paid day. Rows without are paid nothing.
    pub worked: bool,
    pub gross_c: i64,
    pub sss_c: i64,
    pub philhealth_c: i64,
    pub pagibig_c: i64,
    pub contrib_c: i64,
    pub debts: Vec<PayrollDebt>,
    pub pi_loan_c: i64,
    pub pi_stl_c: i64,
    pub sss_el_c: i64,
    pub sss_sl_c: i64,
    pub vale_c: i64,
    pub deductions_c: i64,
    pub net_c: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PayrollWeek {
    /// Saturday (payday).
    pub week: String,
    pub monday: String,
    pub status: PayrollStatus,
    pub posted_at: Option<i64>,
    pub posted_by: Option<String>,
    pub rows: Vec<PayrollRow>,
    pub total_gross_c: i64,
    /// Cash needed for payday.
    pub total_net_c: i64,
    /// Boxes not marked, across everyone.
    pub unmarked: i64,
    /// Employees whose deductions exceed their pay.
    pub over: Vec<String>,
    /// A week after this one: it can't be posted yet.
    pub future: bool,
    pub can_post: bool,
}

impl PayrollRow {
    fn finish(&mut self) {
        let by = |k: DebtKind| -> i64 {
            self.debts
                .iter()
                .filter(|d| d.kind == k)
                .map(|d| d.amount_c)
                .sum()
        };
        self.pi_loan_c = by(DebtKind::PiLoan);
        self.pi_stl_c = by(DebtKind::PiStl);
        self.sss_el_c = by(DebtKind::SssEl);
        self.sss_sl_c = by(DebtKind::SssSl);
        self.vale_c = by(DebtKind::Vale);
        self.contrib_c = self.sss_c + self.philhealth_c + self.pagibig_c;
        self.deductions_c = self.contrib_c + self.debts.iter().map(|d| d.amount_c).sum::<i64>();
        self.net_c = self.gross_c - self.deductions_c;
    }
}

/// Status, posted at, posted by.
type PayrollHead = (String, Option<i64>, Option<String>);

fn payroll_head(conn: &Connection, week: &str) -> Result<Option<PayrollHead>> {
    Ok(conn
        .query_row(
            "SELECT p.status, p.posted_at, COALESCE(u.display_name, u.username)
             FROM payrolls p LEFT JOIN users u ON u.id = p.posted_by
             WHERE p.id = ?1 AND p._deleted = 0",
            [payroll_id(week)],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get(1)?,
                    r.get(2)?,
                ))
            },
        )
        .optional()?)
}

/// The payroll of a week: the posted snapshot, or computed live.
pub fn payroll_week(conn: &Connection, week: &str, today: &str) -> Result<PayrollWeek> {
    let week = require_week(week)?;
    let days = week_days(&week);
    let marks = marks(conn, &days[0], &week)?;
    let head = payroll_head(conn, &week)?;
    let status = match head.as_ref().map(|h| h.0.as_str()) {
        Some("posted") => PayrollStatus::Posted,
        Some("voided") => PayrollStatus::Voided,
        _ => PayrollStatus::Draft,
    };
    let count = |employee: &str, s: Option<AttendanceStatus>| -> i64 {
        days.iter()
            .filter(|d| {
                marks
                    .get(&(employee.to_string(), (*d).clone()))
                    .and_then(|m| m.0)
                    == s
            })
            .count() as i64
    };

    let mut rows = Vec::new();
    if status == PayrollStatus::Posted {
        rows = posted_rows(conn, &week)?;
        for r in &mut rows {
            r.absent = count(&r.employee_id, Some(AttendanceStatus::Absent));
            r.holidays = count(&r.employee_id, Some(AttendanceStatus::Holiday));
            r.paid_holidays = count(&r.employee_id, Some(AttendanceStatus::PaidHoliday));
            r.unmarked = count(&r.employee_id, None);
        }
    } else {
        let paid = paid_by_week(conn)?;
        let over = overrides(conn, Some(&week))?;
        let mut debts: BTreeMap<String, Vec<DebtRow>> = BTreeMap::new();
        for d in load_debts(conn, None)? {
            debts.entry(d.employee_id.clone()).or_default().push(d);
        }
        for e in week_employees(conn, &marks)? {
            let paid_holidays = count(&e.id, Some(AttendanceStatus::PaidHoliday));
            let present = count(&e.id, Some(AttendanceStatus::Present)) + paid_holidays;
            let worked = present > 0;
            let mut row = PayrollRow {
                daily_rate_c: e.daily_rate_c,
                days: present,
                absent: count(&e.id, Some(AttendanceStatus::Absent)),
                holidays: count(&e.id, Some(AttendanceStatus::Holiday)),
                paid_holidays,
                unmarked: count(&e.id, None),
                worked,
                gross_c: e.daily_rate_c * present,
                sss_c: if worked { e.sss_c } else { 0 },
                philhealth_c: if worked { e.philhealth_c } else { 0 },
                pagibig_c: if worked { e.pagibig_c } else { 0 },
                contrib_c: 0,
                debts: Vec::new(),
                pi_loan_c: 0,
                pi_stl_c: 0,
                sss_el_c: 0,
                sss_sl_c: 0,
                vale_c: 0,
                deductions_c: 0,
                net_c: 0,
                employee_id: e.id.clone(),
                name: e.name,
            };
            if worked {
                for d in debts.get(&e.id).into_iter().flatten() {
                    let (balance, scheduled, changed) = draft_charge(d, &paid, &over, &week);
                    if balance <= 0 || week.as_str() < d.terms.first_week.as_str() {
                        continue;
                    }
                    row.debts.push(PayrollDebt {
                        debt_id: d.id.clone(),
                        kind: d.kind,
                        amount_c: changed.unwrap_or(scheduled),
                        scheduled_c: scheduled,
                        changed: changed.is_some(),
                        balance_c: balance,
                        notes: d.notes.clone(),
                    });
                }
            }
            row.finish();
            rows.push(row);
        }
    }

    let this_week = week_of(today).unwrap_or_default();
    let future = week > this_week;
    let over: Vec<String> = rows
        .iter()
        .filter(|r| r.net_c < 0)
        .map(|r| r.name.clone())
        .collect();
    let (posted_at, posted_by) = head
        .filter(|_| status == PayrollStatus::Posted)
        .map(|h| (h.1, h.2))
        .unwrap_or((None, None));
    Ok(PayrollWeek {
        monday: days[0].clone(),
        status,
        posted_at,
        posted_by,
        total_gross_c: rows.iter().filter(|r| r.worked).map(|r| r.gross_c).sum(),
        total_net_c: rows.iter().filter(|r| r.worked).map(|r| r.net_c).sum(),
        unmarked: rows.iter().map(|r| r.unmarked).sum(),
        can_post: status != PayrollStatus::Posted
            && !future
            && over.is_empty()
            && rows.iter().any(|r| r.worked),
        over,
        future,
        rows,
        week,
    })
}

fn posted_rows(conn: &Connection, week: &str) -> Result<Vec<PayrollRow>> {
    let pid = payroll_id(week);
    let mut debts: BTreeMap<String, Vec<PayrollDebt>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare_cached(
            "SELECT x.payroll_line_id, x.debt_id, x.kind, COALESCE(x.amount_c, 0), d.notes
             FROM payroll_debt_lines x
             LEFT JOIN employee_debts d ON d.id = x.debt_id
             WHERE x.payroll_id = ?1 AND x._deleted = 0 AND COALESCE(x.amount_c, 0) <> 0
             ORDER BY d.start_date, x.debt_id",
        )?;
        let rows = stmt.query_map([&pid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                PayrollDebt {
                    debt_id: r.get(1)?,
                    kind: DebtKind::parse(r.get::<_, Option<String>>(2)?.as_deref())
                        .unwrap_or(DebtKind::Vale),
                    amount_c: r.get(3)?,
                    scheduled_c: r.get(3)?,
                    changed: false,
                    balance_c: 0,
                    notes: r.get(4)?,
                },
            ))
        })?;
        for row in rows {
            let (line, d) = row?;
            debts.entry(line).or_default().push(d);
        }
    }
    let mut stmt = conn.prepare_cached(
        "SELECT id, employee_id, COALESCE(name, ''), COALESCE(daily_rate_c, 0), COALESCE(days, 0),
                COALESCE(gross_c, 0), COALESCE(sss_c, 0), COALESCE(philhealth_c, 0),
                COALESCE(pagibig_c, 0)
         FROM payroll_lines
         WHERE payroll_id = ?1 AND _deleted = 0 AND COALESCE(included, 1) <> 0
         ORDER BY name COLLATE NOCASE, employee_id",
    )?;
    let rows = stmt.query_map([&pid], |r| {
        let line: String = r.get(0)?;
        Ok(PayrollRow {
            debts: debts.get(&line).cloned().unwrap_or_default(),
            employee_id: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            name: r.get(2)?,
            daily_rate_c: r.get(3)?,
            days: r.get(4)?,
            absent: 0,
            holidays: 0,
            paid_holidays: 0,
            unmarked: 0,
            worked: true,
            gross_c: r.get(5)?,
            sss_c: r.get(6)?,
            philhealth_c: r.get(7)?,
            pagibig_c: r.get(8)?,
            contrib_c: 0,
            pi_loan_c: 0,
            pi_stl_c: 0,
            sss_el_c: 0,
            sss_sl_c: 0,
            vale_c: 0,
            deductions_c: 0,
            net_c: 0,
        })
    })?;
    let mut out: Vec<PayrollRow> = rows.collect::<Result<_, _>>()?;
    out.iter_mut().for_each(PayrollRow::finish);
    Ok(out)
}

/// Posts a week's payroll: stores what it pays and deducts, and locks the
/// week's attendance.
pub fn post_payroll(tx: &mut WriteTx, week: &str, today: &str) -> Result<()> {
    let p = payroll_week(tx.conn(), week, today)?;
    if p.status == PayrollStatus::Posted {
        return Err(Error::Invalid(
            "this week's payroll is already posted".into(),
        ));
    }
    if p.future {
        return Err(Error::Invalid(
            "this week's payroll can't be posted yet".into(),
        ));
    }
    if !p.over.is_empty() {
        return Err(Error::Invalid(format!(
            "deductions exceed pay for {}; reduce a loan first",
            p.over.join(", ")
        )));
    }
    if !p.rows.iter().any(|r| r.worked) {
        return Err(Error::Invalid("nobody worked this week".into()));
    }
    let pid = payroll_id(&p.week);
    let now = tx.now_ms();
    let by = tx.actor().user_id.clone();
    tx.upsert(
        PAYROLLS,
        &pid,
        json!({ "week": p.week, "status": "posted", "posted_at": now, "posted_by": by }),
    )?;

    let mut keep_lines = BTreeSet::new();
    let mut keep_debts = BTreeSet::new();
    for r in p.rows.iter().filter(|r| r.worked) {
        let line = derived_id(&pid, &r.employee_id);
        keep_lines.insert(line.clone());
        tx.upsert(
            PAYROLL_LINES,
            &line,
            json!({
                "payroll_id": pid,
                "employee_id": r.employee_id,
                "name": r.name,
                "daily_rate_c": r.daily_rate_c,
                "days": r.days,
                "gross_c": r.gross_c,
                "sss_c": r.sss_c,
                "philhealth_c": r.philhealth_c,
                "pagibig_c": r.pagibig_c,
                "net_c": r.net_c,
                "included": 1,
            }),
        )?;
        for d in &r.debts {
            let id = derived_id(&line, &d.debt_id);
            keep_debts.insert(id.clone());
            tx.upsert(
                PAYROLL_DEBT_LINES,
                &id,
                json!({
                    "payroll_id": pid,
                    "payroll_line_id": line,
                    "debt_id": d.debt_id,
                    "kind": d.kind.as_str(),
                    "amount_c": d.amount_c,
                }),
            )?;
        }
    }
    // Rows from an earlier post of this week that this one leaves out.
    for id in crate::domain::line_ids(tx, PAYROLL_LINES, "payroll_id", &pid)? {
        if !keep_lines.contains(&id) {
            tx.update(PAYROLL_LINES, &id, json!({ "included": 0 }))?;
        }
    }
    for id in crate::domain::line_ids(tx, PAYROLL_DEBT_LINES, "payroll_id", &pid)? {
        if !keep_debts.contains(&id) {
            tx.update(PAYROLL_DEBT_LINES, &id, json!({ "amount_c": 0 }))?;
        }
    }
    Ok(())
}

/// Voids a posted payroll: its deductions return to the debts and the week's
/// attendance can be changed again.
pub fn void_payroll(tx: &mut WriteTx, week: &str) -> Result<()> {
    let week = require_week(week)?;
    if !posted(tx.conn(), &week)? {
        return Err(Error::Invalid("this week's payroll isn't posted".into()));
    }
    tx.update(PAYROLLS, &payroll_id(&week), json!({ "status": "voided" }))
        .map(drop)
}

// Employee profile -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PayslipSummary {
    pub week: String,
    pub days: i64,
    pub gross_c: i64,
    pub deductions_c: i64,
    pub net_c: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct EmployeeProfile {
    pub employee: Employee,
    /// Every debt, newest last; paid ones included.
    pub debts: Vec<DebtInfo>,
    pub year: i32,
    /// This year's days present, absences, unpaid and paid holidays.
    pub present: i64,
    pub absent: i64,
    pub holidays: i64,
    pub paid_holidays: i64,
    /// Posted payrolls, newest first.
    pub payslips: Vec<PayslipSummary>,
}

pub fn employee_profile(
    conn: &Connection,
    id: &str,
    today: &str,
) -> Result<Option<EmployeeProfile>> {
    let Some(employee) = get_employee(conn, id)? else {
        return Ok(None);
    };
    let year: i32 = today.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0);
    let (present, absent, holidays, paid_holidays): (i64, i64, i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(status = 'present'), 0), COALESCE(SUM(status = 'absent'), 0),
                COALESCE(SUM(status = 'holiday'), 0), COALESCE(SUM(status = 'paid_holiday'), 0)
         FROM attendance
         WHERE employee_id = ?1 AND _deleted = 0 AND substr(date, 1, 4) = ?2",
        (id, format!("{year:04}")),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let mut stmt = conn.prepare(
        "SELECT p.week, COALESCE(l.days, 0), COALESCE(l.gross_c, 0), COALESCE(l.net_c, 0)
         FROM payroll_lines l
         JOIN payrolls p ON p.id = l.payroll_id AND p._deleted = 0 AND p.status = 'posted'
         WHERE l.employee_id = ?1 AND l._deleted = 0 AND COALESCE(l.included, 1) <> 0
         ORDER BY p.week DESC",
    )?;
    let payslips = stmt
        .query_map([id], |r| {
            let gross: i64 = r.get(2)?;
            let net: i64 = r.get(3)?;
            Ok(PayslipSummary {
                week: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                days: r.get(1)?,
                gross_c: gross,
                deductions_c: gross - net,
                net_c: net,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Some(EmployeeProfile {
        debts: list_debts(conn, Some(id), today)?,
        employee,
        year,
        present,
        absent,
        holidays,
        paid_holidays,
        payslips,
    }))
}

// Dashboard ------------------------------------------------------------------

/// Past weeks with paid days but no posted payroll, oldest first.
pub fn unposted_weeks(conn: &Connection, today: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT date FROM attendance
         WHERE _deleted = 0 AND status IN ('present', 'paid_holiday') AND date < ?1",
    )?;
    let dates: Vec<String> = stmt
        .query_map([today], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut weeks = BTreeSet::new();
    for d in dates {
        if let Some(w) = week_of(&d).filter(|w| w.as_str() < today) {
            weeks.insert(w);
        }
    }
    let mut out = Vec::new();
    for w in weeks {
        if !posted(conn, &w)? {
            out.push(w);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(amount: i64, weekly: i64, first: &str, weeks: i64) -> Terms {
        Terms {
            amount_c: amount,
            weekly_c: weekly,
            first_week: first.into(),
            last_week: add_days(first, 7 * (weeks - 1)).unwrap(),
        }
    }

    #[test]
    fn weeks_end_on_saturday() {
        // 2026-10-31 is a Saturday.
        assert_eq!(weekday("2026-10-31"), Some(5));
        assert_eq!(weekday("2026-10-26"), Some(0));
        assert_eq!(week_of("2026-10-30").as_deref(), Some("2026-10-31"));
        assert_eq!(week_of("2026-10-31").as_deref(), Some("2026-10-31"));
        assert_eq!(week_of("2026-11-01").as_deref(), Some("2026-11-07"));
        assert_eq!(week_of("2026-10-25").as_deref(), Some("2026-10-31"));
        assert_eq!(
            week_days("2026-10-31"),
            [
                "2026-10-26",
                "2026-10-27",
                "2026-10-28",
                "2026-10-29",
                "2026-10-30",
                "2026-10-31"
            ]
        );
        assert!(require_week("2026-10-30").is_err());
    }

    #[test]
    fn schedule_follows_the_balance() {
        // ₱1,000 over 4 weeks from Nov 7: ₱250 a week.
        let t = terms(100_000, 25_000, "2026-11-07", 4);
        assert_eq!(scheduled_charge(&t, 100_000, "2026-10-31"), 0);
        assert_eq!(scheduled_charge(&t, 100_000, "2026-11-07"), 25_000);
        // Nov 14 skipped: Nov 21 takes ₱375.
        assert_eq!(scheduled_charge(&t, 75_000, "2026-11-21"), 37_500);
        // Last week, and after: the whole balance.
        assert_eq!(scheduled_charge(&t, 37_500, "2026-11-28"), 37_500);
        assert_eq!(scheduled_charge(&t, 17_500, "2026-12-05"), 17_500);
        // Ahead of schedule: the weekly amount, no more than the balance.
        assert_eq!(scheduled_charge(&t, 30_000, "2026-11-14"), 25_000);
        assert_eq!(scheduled_charge(&t, 10_000, "2026-11-14"), 10_000);
        assert_eq!(scheduled_charge(&t, 0, "2026-11-14"), 0);
    }
}
