//! Payments received from clients: cash, checks and bank transfers, each
//! allocated to the client's delivered DRs.
//!
//! A check is a payment with check details and a check status:
//! on hand → deposited → cleared, or bounced (from deposited or cleared; a
//! bounced check can be deposited again). It is a PDC while its check date
//! is in the future. A check counts as paid from the day it is received
//! until it bounces or is replaced: a payment that names it in
//! `replaces_id` (the client's new check, cash, …) replaces it while that
//! payment is posted.
//!
//! Payments stay editable (amount, details, allocations); a wrong one is
//! voided. Allocations use derived ids per DR and are cleared, never
//! deleted. What a payment doesn't allocate is the client's credit.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::{MAX_LINES, peso, require_date, require_text, text_opt};
use crate::error::{Error, Result};
use crate::ids::derived_id;
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};
use crate::time::manila_date_string;

pub const PAYMENTS: &str = "payments";
pub const PAYMENT_ALLOCATIONS: &str = "payment_allocations";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PayMethod {
    Cash,
    Check,
    BankTransfer,
}

impl PayMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            PayMethod::Cash => "cash",
            PayMethod::Check => "check",
            PayMethod::BankTransfer => "bank_transfer",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("check") => PayMethod::Check,
            Some("bank_transfer") => PayMethod::BankTransfer,
            _ => PayMethod::Cash,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    OnHand,
    Deposited,
    Cleared,
    Bounced,
    /// Not stored: a posted payment replaces it.
    Replaced,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckStatus::OnHand => "on_hand",
            CheckStatus::Deposited => "deposited",
            CheckStatus::Cleared => "cleared",
            CheckStatus::Bounced => "bounced",
            CheckStatus::Replaced => "replaced",
        }
    }

    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("deposited") => CheckStatus::Deposited,
            Some("cleared") => CheckStatus::Cleared,
            Some("bounced") => CheckStatus::Bounced,
            _ => CheckStatus::OnHand,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Posted,
    Void,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CheckInput {
    pub bank: String,
    pub check_no: String,
    pub check_date: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AllocationInput {
    pub dr_id: String,
    pub amount_c: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PaymentInput {
    /// `None` records a new payment.
    pub id: Option<String>,
    pub client_id: String,
    /// When it was received (not in the future).
    pub date: String,
    pub method: PayMethod,
    pub amount_c: i64,
    /// Bank transfer reference, deposit slip, …
    pub reference: String,
    /// Official/collection receipt number from the booklet, if issued.
    pub or_no: String,
    pub notes: String,
    /// Required for checks.
    pub check: Option<CheckInput>,
    /// A check of the client's (bounced or on hand) this payment replaces.
    /// Only when recording a payment.
    pub replaces_id: Option<String>,
    /// Zero amounts are left out.
    pub allocations: Vec<AllocationInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CheckStep {
    /// on hand (or bounced) → deposited: `date`, `account`.
    Deposit,
    /// deposited → cleared: `date`.
    Clear,
    /// deposited or cleared → bounced: `date`, `reason`.
    Bounce,
    /// Back one step: deposited → on hand, cleared → deposited, bounced →
    /// deposited (or on hand if it never was).
    Undo,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CheckStepInput {
    pub step: CheckStep,
    pub date: String,
    pub account: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CheckInfo {
    pub bank: Option<String>,
    pub check_no: Option<String>,
    pub check_date: Option<String>,
    /// Post-dated: the check date is after today.
    pub pdc: bool,
    pub status: CheckStatus,
    pub deposited_date: Option<String>,
    pub deposit_account: Option<String>,
    pub cleared_date: Option<String>,
    pub bounced_date: Option<String>,
    pub bounce_reason: Option<String>,
    /// The posted payment that replaces this check.
    pub replaced_by_id: Option<String>,
    pub replaced_by_no: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PaymentInfo {
    pub id: String,
    pub no: Option<String>,
    /// False while the number was given offline: it may still change.
    pub no_confirmed: bool,
    pub client_id: Option<String>,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    pub date: String,
    pub method: PayMethod,
    pub amount_c: i64,
    /// Allocated to delivered DRs.
    pub applied_c: i64,
    /// Amount minus applied (negative if offline edits overshoot).
    pub unapplied_c: i64,
    /// Reduces the client's balance: posted, and not a bounced or replaced
    /// check.
    pub counts: bool,
    pub reference: Option<String>,
    pub or_no: Option<String>,
    pub notes: Option<String>,
    pub check: Option<CheckInfo>,
    pub replaces_id: Option<String>,
    pub replaces_no: Option<String>,
    pub status: PaymentStatus,
    pub dr_nos: Vec<String>,
    pub created_at: i64,
    pub created_by_name: Option<String>,
    pub voided_at: Option<i64>,
    pub voided_by_name: Option<String>,
    pub void_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AllocationInfo {
    pub dr_id: String,
    pub dr_no: Option<String>,
    pub delivered_date: Option<String>,
    /// False if the DR was voided since: the amount is back as credit.
    pub delivered: bool,
    pub gross_c: i64,
    pub amount_c: i64,
    /// What the DR still owes, after every counting payment.
    pub dr_balance_c: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PaymentDetail {
    pub payment: PaymentInfo,
    pub allocations: Vec<AllocationInfo>,
}

/// A delivered DR a payment can be allocated to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OpenDr {
    pub dr_id: String,
    pub no: Option<String>,
    pub delivered_date: Option<String>,
    pub due_date: Option<String>,
    pub gross_c: i64,
    /// Paid by other counting payments.
    pub paid_other_c: i64,
    /// Gross minus that: the most this payment can allocate.
    pub open_c: i64,
    /// Allocated by this payment now.
    pub allocated_c: i64,
}

/// A payment allocated to a DR, for the DR's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrPayment {
    pub payment_id: String,
    pub no: Option<String>,
    pub date: String,
    pub method: PayMethod,
    pub check_status: Option<CheckStatus>,
    pub amount_c: i64,
    pub counts: bool,
}

pub fn allocation_id(payment_id: &str, dr_id: &str) -> String {
    derived_id(payment_id, &format!("allocation/{dr_id}"))
}

// Writing -------------------------------------------------------------------

struct Prepared {
    date: String,
    check: Option<(String, String, String)>,
    replaces: Option<String>,
    allocations: BTreeMap<String, i64>,
}

/// The payment as stored, for an edit.
struct Current {
    method: PayMethod,
    replaces: Option<String>,
}

fn current(conn: &Connection, id: &str) -> Result<Current> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT method, status, replaces_id FROM payments WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (method, status, replaces) =
        row.ok_or_else(|| Error::Invalid("that payment no longer exists".into()))?;
    if status.as_deref() == Some("void") {
        return Err(Error::Invalid("this payment is void".into()));
    }
    Ok(Current {
        method: PayMethod::parse(method.as_deref()),
        replaces,
    })
}

/// A check `replaces` may name: the same client's, posted, a check, on hand
/// or bounced, and not replaced by another payment.
fn check_replaceable(conn: &Connection, id: &str, client: &str, by: Option<&str>) -> Result<()> {
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Option<Row> = conn
        .query_row(
            "SELECT client_id, method, status, check_status FROM payments
             WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((owner, method, status, check_status)) = row else {
        return Err(Error::Invalid(
            "the check to replace no longer exists".into(),
        ));
    };
    if owner.as_deref() != Some(client) || method.as_deref() != Some("check") {
        return Err(Error::Invalid(
            "only a check of the same client can be replaced".into(),
        ));
    }
    if status.as_deref() == Some("void") {
        return Err(Error::Invalid("the check to replace was voided".into()));
    }
    if !matches!(
        check_status.as_deref(),
        Some("bounced") | Some("on_hand") | None
    ) {
        return Err(Error::Invalid(
            "only a check on hand or a bounced check can be replaced".into(),
        ));
    }
    let taken: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM payments WHERE replaces_id = ?1 AND _deleted = 0
                        AND status = 'posted' AND (?2 IS NULL OR id <> ?2))",
        (id, by),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid("that check was already replaced".into()));
    }
    Ok(())
}

fn prepare(conn: &Connection, input: &PaymentInput, today: &str) -> Result<Prepared> {
    let cur = input
        .id
        .as_deref()
        .map(|id| current(conn, id))
        .transpose()?;
    let client: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM clients WHERE id = ?1 AND _deleted = 0",
            [&input.client_id],
            |r| r.get(0),
        )
        .optional()?;
    if client.is_none() {
        return Err(Error::Invalid("choose the client who paid".into()));
    }
    if let (Some(id), Some(_)) = (&input.id, &cur) {
        let owner: Option<String> =
            conn.query_row("SELECT client_id FROM payments WHERE id = ?1", [id], |r| {
                r.get(0)
            })?;
        if owner.as_deref() != Some(input.client_id.as_str()) {
            return Err(Error::Invalid("a payment stays with its client".into()));
        }
    }
    let date = require_date(&input.date, "payment date")?;
    if date.as_str() > today {
        return Err(Error::Invalid(
            "the payment date can't be in the future".into(),
        ));
    }
    if input.amount_c <= 0 {
        return Err(Error::Invalid("enter the amount received".into()));
    }
    if cur.as_ref().is_some_and(|c| c.method != input.method) {
        return Err(Error::Invalid(
            "the payment method can't change; void the payment and record it again".into(),
        ));
    }
    let check = match (input.method, &input.check) {
        (PayMethod::Check, Some(c)) => Some((
            require_text(&c.bank, "the check's bank")?,
            require_text(&c.check_no, "the check number")?,
            require_date(&c.check_date, "check date")?,
        )),
        (PayMethod::Check, None) => {
            return Err(Error::Invalid("enter the check's details".into()));
        }
        _ => None,
    };
    // The replaced check is fixed when the payment is recorded.
    let replaces = match &cur {
        Some(c) => c.replaces.clone(),
        None => input.replaces_id.clone().filter(|r| !r.is_empty()),
    };
    if let (Some(r), None) = (&replaces, &cur) {
        check_replaceable(conn, r, &input.client_id, input.id.as_deref())?;
    }

    let mut allocations: BTreeMap<String, i64> = BTreeMap::new();
    for a in &input.allocations {
        if a.amount_c < 0 {
            return Err(Error::Invalid("allocations can't be negative".into()));
        }
        if a.amount_c > 0 {
            *allocations.entry(a.dr_id.clone()).or_default() += a.amount_c;
        }
    }
    if allocations.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a payment can be allocated to at most {MAX_LINES} DRs"
        )));
    }
    let total: i64 = allocations.values().sum();
    if total > input.amount_c {
        return Err(Error::Invalid(format!(
            "the allocations ({}) are more than the amount received ({})",
            peso(total),
            peso(input.amount_c)
        )));
    }
    // What other counting payments paid; this payment and the check it
    // replaces don't count against it.
    for (dr, amount) in &allocations {
        type DrRow = (Option<String>, Option<String>, Option<String>, i64);
        let row: Option<DrRow> = conn
            .query_row(
                "SELECT t.client_id, t.status, n.no, t.gross_c FROM v_dr_totals t
                 LEFT JOIN doc_numbers n ON n.ref_id = t.dr_id AND n.series = 'DR' AND n._deleted = 0
                 WHERE t.dr_id = ?1",
                [dr],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((owner, status, no, gross)) = row else {
            return Err(Error::Invalid(
                "a DR to allocate to no longer exists".into(),
            ));
        };
        let name = no.map_or_else(|| "a DR".to_string(), |n| format!("DR {n}"));
        if owner.as_deref() != Some(input.client_id.as_str()) {
            return Err(Error::Invalid(format!("{name} is another client's")));
        }
        if status.as_deref() != Some("delivered") {
            return Err(Error::Invalid(format!(
                "{name} isn't delivered; only delivered DRs can be paid"
            )));
        }
        let paid_other: i64 = conn.query_row(
            "SELECT COALESCE(SUM(amount_c), 0) FROM v_allocations
             WHERE dr_id = ?1 AND payment_id <> ?2 AND (?3 IS NULL OR payment_id <> ?3)",
            (dr, input.id.as_deref().unwrap_or(""), replaces.as_deref()),
            |r| r.get(0),
        )?;
        let open = gross - paid_other;
        if *amount > open {
            return Err(Error::Invalid(format!(
                "{name} has only {} left to pay",
                peso(open.max(0))
            )));
        }
    }
    Ok(Prepared {
        date,
        check,
        replaces,
        allocations,
    })
}

/// Checks a payment as [`create`] or [`update`] would, without writing.
/// `today` is Manila's.
pub fn validate(conn: &Connection, input: &PaymentInput, today: &str) -> Result<()> {
    prepare(conn, input, today).map(drop)
}

fn write_allocations(
    tx: &mut WriteTx,
    payment: &str,
    wanted: &BTreeMap<String, i64>,
) -> Result<()> {
    let existing: Vec<(String, String)> = {
        let mut stmt = tx.conn().prepare(
            "SELECT id, dr_id FROM payment_allocations
             WHERE payment_id = ?1 AND _deleted = 0 AND amount_c IS NOT NULL",
        )?;
        let rows = stmt.query_map([payment], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (dr, amount) in wanted {
        tx.upsert(
            PAYMENT_ALLOCATIONS,
            &allocation_id(payment, dr),
            json!({ "payment_id": payment, "dr_id": dr, "amount_c": amount }),
        )?;
    }
    for (id, dr) in existing {
        if !wanted.contains_key(&dr) {
            tx.update(PAYMENT_ALLOCATIONS, &id, json!({ "amount_c": null }))?;
        }
    }
    Ok(())
}

fn fields(input: &PaymentInput, p: &Prepared) -> serde_json::Value {
    let (bank, no, date) = match &p.check {
        Some((b, n, d)) => (Some(b.clone()), Some(n.clone()), Some(d.clone())),
        None => (None, None, None),
    };
    json!({
        "date": p.date,
        "amount_c": input.amount_c,
        "reference": text_opt(&input.reference),
        "or_no": text_opt(&input.or_no),
        "notes": text_opt(&input.notes),
        "check_bank": bank,
        "check_no": no,
        "check_date": date,
    })
}

/// Records a payment with the number reserved for it (`res.ref_id` is the
/// new payment's id). Returns the id.
pub fn create(tx: &mut WriteTx, input: &PaymentInput, res: &Reservation) -> Result<String> {
    if res.series != series::PAYMENT {
        return Err(Error::Invalid("wrong number series for a payment".into()));
    }
    if input.id.is_some() {
        return Err(Error::Invalid("this payment was already recorded".into()));
    }
    let today = manila_date_string(tx.now_ms());
    let p = prepare(tx.conn(), input, &today)?;
    let mut data = fields(input, &p);
    data["id"] = json!(res.ref_id);
    data["client_id"] = json!(input.client_id);
    data["method"] = json!(input.method.as_str());
    data["status"] = json!("posted");
    data["replaces_id"] = json!(p.replaces);
    if input.method == PayMethod::Check {
        data["check_status"] = json!(CheckStatus::OnHand.as_str());
    }
    let id = tx.insert(PAYMENTS, data)?;
    write_allocations(tx, &id, &p.allocations)?;
    numbering::assign(tx, res, PAYMENTS)?;
    Ok(id)
}

/// Edits a posted payment: date, amount, details and allocations.
pub fn update(tx: &mut WriteTx, input: &PaymentInput) -> Result<()> {
    let id = input
        .id
        .clone()
        .ok_or_else(|| Error::Invalid("which payment?".into()))?;
    let today = manila_date_string(tx.now_ms());
    let p = prepare(tx.conn(), input, &today)?;
    tx.update(PAYMENTS, &id, fields(input, &p))?;
    write_allocations(tx, &id, &p.allocations)
}

/// Voids a payment: it no longer counts, and the DRs it paid owe again.
pub fn void(tx: &mut WriteTx, id: &str, reason: &str) -> Result<()> {
    current(tx.conn(), id)?;
    let reason =
        text_opt(reason).ok_or_else(|| Error::Invalid("enter why the payment is voided".into()))?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        PAYMENTS,
        id,
        json!({
            "status": "void",
            "voided_at": now,
            "voided_by": user,
            "void_reason": reason,
        }),
    )
    .map(drop)
}

/// The patch a check step writes, after checking it is allowed.
fn step_patch(
    conn: &Connection,
    id: &str,
    input: &CheckStepInput,
    today: &str,
) -> Result<serde_json::Value> {
    let cur = current(conn, id)?;
    if cur.method != PayMethod::Check {
        return Err(Error::Invalid("this payment isn't a check".into()));
    }
    type Row = (Option<String>, Option<String>, Option<String>);
    let (status, check_date, deposited): Row = conn.query_row(
        "SELECT check_status, check_date, deposited_date FROM payments WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let status = CheckStatus::parse(status.as_deref());
    let replaced: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM payments WHERE replaces_id = ?1 AND _deleted = 0
                        AND status = 'posted')",
        [id],
        |r| r.get(0),
    )?;
    if replaced {
        return Err(Error::Invalid("this check was replaced".into()));
    }
    let date = || -> Result<String> {
        let d = require_date(&input.date, "date")?;
        if d.as_str() > today {
            return Err(Error::Invalid("the date can't be in the future".into()));
        }
        Ok(d)
    };
    let wrong = |what: &str| {
        Err(Error::Invalid(format!(
            "a check {} can't be {what}",
            match status {
                CheckStatus::OnHand => "on hand",
                CheckStatus::Deposited => "that is deposited",
                CheckStatus::Cleared => "that cleared",
                CheckStatus::Bounced => "that bounced",
                CheckStatus::Replaced => "that was replaced",
            }
        )))
    };
    Ok(match input.step {
        CheckStep::Deposit => {
            if !matches!(status, CheckStatus::OnHand | CheckStatus::Bounced) {
                return wrong("deposited");
            }
            let d = date()?;
            if check_date.as_deref().is_some_and(|c| d.as_str() < c) {
                return Err(Error::Invalid(
                    "a post-dated check can't be deposited before its check date".into(),
                ));
            }
            json!({
                "check_status": CheckStatus::Deposited.as_str(),
                "deposited_date": d,
                "deposit_account": text_opt(&input.account),
                "cleared_date": null,
                "bounced_date": null,
                "bounce_reason": null,
            })
        }
        CheckStep::Clear => {
            if status != CheckStatus::Deposited {
                return wrong("marked cleared");
            }
            let d = date()?;
            if deposited.as_deref().is_some_and(|dep| d.as_str() < dep) {
                return Err(Error::Invalid(
                    "a check can't clear before it was deposited".into(),
                ));
            }
            json!({ "check_status": CheckStatus::Cleared.as_str(), "cleared_date": d })
        }
        CheckStep::Bounce => {
            if !matches!(status, CheckStatus::Deposited | CheckStatus::Cleared) {
                return wrong("marked bounced");
            }
            let reason = text_opt(&input.reason)
                .ok_or_else(|| Error::Invalid("enter why the check bounced".into()))?;
            json!({
                "check_status": CheckStatus::Bounced.as_str(),
                "bounced_date": date()?,
                "bounce_reason": reason,
            })
        }
        CheckStep::Undo => match status {
            CheckStatus::Deposited => json!({
                "check_status": CheckStatus::OnHand.as_str(),
                "deposited_date": null,
                "deposit_account": null,
            }),
            CheckStatus::Cleared => json!({
                "check_status": CheckStatus::Deposited.as_str(),
                "cleared_date": null,
            }),
            CheckStatus::Bounced => json!({
                "check_status": if deposited.is_some() {
                    CheckStatus::Deposited.as_str()
                } else {
                    CheckStatus::OnHand.as_str()
                },
                "bounced_date": null,
                "bounce_reason": null,
            }),
            _ => return wrong("undone"),
        },
    })
}

/// Checks a check step as [`check_step`] would, without writing.
pub fn validate_step(
    conn: &Connection,
    id: &str,
    input: &CheckStepInput,
    today: &str,
) -> Result<()> {
    step_patch(conn, id, input, today).map(drop)
}

/// Moves a check along: deposited, cleared, bounced, or back a step.
pub fn check_step(tx: &mut WriteTx, id: &str, input: &CheckStepInput) -> Result<()> {
    let today = manila_date_string(tx.now_ms());
    let patch = step_patch(tx.conn(), id, input, &today)?;
    tx.update(PAYMENTS, id, patch).map(drop)
}

// Reading -------------------------------------------------------------------

const SELECT: &str = "
    SELECT p.id, n.no, COALESCE(n.confirmed, 1), p.client_id, cn.no, c.name, p.date, p.method,
           COALESCE(p.amount_c, 0), COALESCE(v.counts, 0), p.reference, p.or_no, p.notes,
           p.check_bank, p.check_no, p.check_date, p.check_status, p.deposited_date,
           p.deposit_account, p.cleared_date, p.bounced_date, p.bounce_reason,
           (SELECT r.id FROM payments r WHERE r.replaces_id = p.id AND r._deleted = 0
            AND r.status = 'posted' ORDER BY r.created_at, r.id LIMIT 1),
           p.replaces_id, rn.no, p.status, p.created_at, uc.display_name, p.voided_at,
           uv.display_name, p.void_reason
    FROM payments p
    LEFT JOIN v_payment_counts v ON v.payment_id = p.id
    LEFT JOIN doc_numbers n ON n.ref_id = p.id AND n.series = 'PAY' AND n._deleted = 0
    LEFT JOIN clients c ON c.id = p.client_id
    LEFT JOIN doc_numbers cn ON cn.ref_id = p.client_id AND cn.series = 'CL' AND cn._deleted = 0
    LEFT JOIN doc_numbers rn ON rn.ref_id = p.replaces_id AND rn.series = 'PAY' AND rn._deleted = 0
    LEFT JOIN users uc ON uc.id = p.created_by
    LEFT JOIN users uv ON uv.id = p.voided_by
    WHERE p._deleted = 0";

fn from_row(r: &rusqlite::Row, today: &str) -> rusqlite::Result<PaymentInfo> {
    let method = PayMethod::parse(r.get::<_, Option<String>>(7)?.as_deref());
    let replaced_by: Option<String> = r.get(22)?;
    let check = if method == PayMethod::Check {
        let check_date: Option<String> = r.get(15)?;
        let status = if replaced_by.is_some() {
            CheckStatus::Replaced
        } else {
            CheckStatus::parse(r.get::<_, Option<String>>(16)?.as_deref())
        };
        Some(CheckInfo {
            bank: r.get(13)?,
            check_no: r.get(14)?,
            pdc: check_date.as_deref().is_some_and(|d| d > today),
            check_date,
            status,
            deposited_date: r.get(17)?,
            deposit_account: r.get(18)?,
            cleared_date: r.get(19)?,
            bounced_date: r.get(20)?,
            bounce_reason: r.get(21)?,
            replaced_by_id: replaced_by,
            replaced_by_no: None,
        })
    } else {
        None
    };
    let amount_c: i64 = r.get(8)?;
    Ok(PaymentInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        client_id: r.get(3)?,
        client_code: r.get(4)?,
        client_name: r.get(5)?,
        date: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        method,
        amount_c,
        applied_c: 0,
        unapplied_c: amount_c,
        counts: r.get::<_, i64>(9)? != 0,
        reference: r.get(10)?,
        or_no: r.get(11)?,
        notes: r.get(12)?,
        check,
        replaces_id: r.get(23)?,
        replaces_no: r.get(24)?,
        status: if r.get::<_, Option<String>>(25)?.as_deref() == Some("void") {
            PaymentStatus::Void
        } else {
            PaymentStatus::Posted
        },
        dr_nos: Vec::new(),
        created_at: r.get(26)?,
        created_by_name: r.get(27)?,
        voided_at: r.get(28)?,
        voided_by_name: r.get(29)?,
        void_reason: r.get(30)?,
    })
}

/// Allocations of a payment (or all), to live DRs, as (payment, info).
fn allocations_of(
    conn: &Connection,
    payment: Option<&str>,
) -> Result<Vec<(String, AllocationInfo)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.payment_id, a.dr_id, n.no, t.delivered_date, t.status, t.gross_c, a.amount_c,
                COALESCE(v.balance_c, t.gross_c)
         FROM payment_allocations a
         JOIN v_dr_totals t ON t.dr_id = a.dr_id
         LEFT JOIN v_receivables v ON v.dr_id = a.dr_id
         LEFT JOIN doc_numbers n ON n.ref_id = a.dr_id AND n.series = 'DR' AND n._deleted = 0
         WHERE a._deleted = 0 AND a.amount_c IS NOT NULL AND a.amount_c <> 0
           AND (?1 IS NULL OR a.payment_id = ?1)
         ORDER BY t.delivered_date, n.no, a.dr_id",
    )?;
    let rows = stmt.query_map([payment], |r| {
        Ok((
            r.get(0)?,
            AllocationInfo {
                dr_id: r.get(1)?,
                dr_no: r.get(2)?,
                delivered_date: r.get(3)?,
                delivered: r.get::<_, Option<String>>(4)?.as_deref() == Some("delivered"),
                gross_c: r.get(5)?,
                amount_c: r.get(6)?,
                dr_balance_c: r.get(7)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn fill(p: &mut PaymentInfo, allocs: &[AllocationInfo], numbers: &HashMap<String, Option<String>>) {
    p.applied_c = allocs
        .iter()
        .filter(|a| a.delivered)
        .map(|a| a.amount_c)
        .sum();
    p.unapplied_c = p.amount_c - p.applied_c;
    p.dr_nos = allocs
        .iter()
        .filter(|a| a.delivered)
        .map(|a| a.dr_no.clone().unwrap_or_else(|| "(no DR no.)".into()))
        .collect();
    if let Some(c) = &mut p.check {
        c.replaced_by_no = c
            .replaced_by_id
            .as_ref()
            .and_then(|id| numbers.get(id).cloned().flatten());
    }
}

fn numbers(conn: &Connection) -> Result<HashMap<String, Option<String>>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ref_id, no FROM doc_numbers WHERE series = 'PAY' AND _deleted = 0",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every payment, newest first. `today` (Manila) decides what is a PDC.
pub fn list(conn: &Connection, today: &str) -> Result<Vec<PaymentInfo>> {
    let mut by_payment: HashMap<String, Vec<AllocationInfo>> = HashMap::new();
    for (p, a) in allocations_of(conn, None)? {
        by_payment.entry(p).or_default().push(a);
    }
    let numbers = numbers(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY p.date DESC, p.created_at DESC, p.id DESC"
    ))?;
    let rows = stmt.query_map([], |r| from_row(r, today))?;
    let mut out = Vec::new();
    for row in rows {
        let mut p = row?;
        let allocs = by_payment.remove(&p.id).unwrap_or_default();
        fill(&mut p, &allocs, &numbers);
        out.push(p);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str, today: &str) -> Result<Option<PaymentDetail>> {
    let Some(mut p) = conn
        .query_row(&format!("{SELECT} AND p.id = ?1"), [id], |r| {
            from_row(r, today)
        })
        .optional()?
    else {
        return Ok(None);
    };
    let allocations: Vec<AllocationInfo> = allocations_of(conn, Some(id))?
        .into_iter()
        .map(|(_, a)| a)
        .collect();
    fill(&mut p, &allocations, &numbers(conn)?);
    Ok(Some(PaymentDetail {
        payment: p,
        allocations,
    }))
}

/// The client's delivered DRs a payment can go to: those with something
/// left to pay, plus those `payment` already allocates to. Oldest first.
/// `replaces` is a check being replaced: what it paid is open again.
pub fn open_drs(
    conn: &Connection,
    client_id: &str,
    payment: Option<&str>,
    replaces: Option<&str>,
) -> Result<Vec<OpenDr>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.dr_id, n.no, t.delivered_date, c.terms_days, t.gross_c,
                COALESCE((SELECT SUM(a.amount_c) FROM v_allocations a WHERE a.dr_id = t.dr_id
                          AND a.payment_id <> ?2 AND (?3 IS NULL OR a.payment_id <> ?3)), 0),
                COALESCE((SELECT a.amount_c FROM payment_allocations a WHERE a.payment_id = ?2
                          AND a.dr_id = t.dr_id AND a._deleted = 0), 0)
         FROM v_dr_totals t
         LEFT JOIN clients c ON c.id = t.client_id
         LEFT JOIN doc_numbers n ON n.ref_id = t.dr_id AND n.series = 'DR' AND n._deleted = 0
         WHERE t.status = 'delivered' AND t.client_id = ?1
         ORDER BY t.delivered_date, n.no, t.dr_id",
    )?;
    let rows = stmt.query_map((client_id, payment.unwrap_or(""), replaces), |r| {
        let delivered: Option<String> = r.get(2)?;
        let gross: i64 = r.get(4)?;
        let paid_other: i64 = r.get(5)?;
        Ok(OpenDr {
            dr_id: r.get(0)?,
            no: r.get(1)?,
            due_date: crate::domain::receivables::due_date(delivered.as_deref(), r.get(3)?),
            delivered_date: delivered,
            gross_c: gross,
            paid_other_c: paid_other,
            open_c: gross - paid_other,
            allocated_c: r.get(6)?,
        })
    })?;
    let all: Vec<OpenDr> = rows.collect::<Result<_, _>>()?;
    Ok(all
        .into_iter()
        .filter(|d| d.open_c > 0 || d.allocated_c != 0)
        .collect())
}

/// Payments allocated to a DR, oldest first.
pub fn for_dr(conn: &Connection, dr_id: &str) -> Result<Vec<DrPayment>> {
    let mut stmt = conn.prepare_cached(
        "SELECT p.id, n.no, p.date, p.method, p.check_status, a.amount_c, COALESCE(v.counts, 0),
                EXISTS (SELECT 1 FROM payments r WHERE r.replaces_id = p.id AND r._deleted = 0
                        AND r.status = 'posted')
         FROM payment_allocations a
         JOIN payments p ON p.id = a.payment_id AND p._deleted = 0
         LEFT JOIN v_payment_counts v ON v.payment_id = p.id
         LEFT JOIN doc_numbers n ON n.ref_id = p.id AND n.series = 'PAY' AND n._deleted = 0
         WHERE a.dr_id = ?1 AND a._deleted = 0 AND a.amount_c IS NOT NULL AND a.amount_c <> 0
         ORDER BY p.date, p.created_at, p.id",
    )?;
    let rows = stmt.query_map([dr_id], |r| {
        let method = PayMethod::parse(r.get::<_, Option<String>>(3)?.as_deref());
        let replaced: bool = r.get(7)?;
        Ok(DrPayment {
            payment_id: r.get(0)?,
            no: r.get(1)?,
            date: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            method,
            check_status: (method == PayMethod::Check)
                .then(|| {
                    if replaced {
                        Ok(CheckStatus::Replaced)
                    } else {
                        r.get::<_, Option<String>>(4)
                            .map(|s| CheckStatus::parse(s.as_deref()))
                    }
                })
                .transpose()?,
            amount_c: r.get(5)?,
            counts: r.get::<_, i64>(6)? != 0,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
