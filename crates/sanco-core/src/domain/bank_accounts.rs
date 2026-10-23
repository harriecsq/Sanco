//! The company's bank accounts. Check deposits and bank transfers received
//! (payments) and TT payments to suppliers name the account the money went
//! into or came out of. No balance is kept: withdrawals and bank charges
//! aren't recorded here, so an account shows only what the app moved.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::auth::{Field, Perms};
use crate::domain::payments::CheckStatus;
use crate::domain::pricing::check_currency;
use crate::domain::purchasing;
use crate::domain::{require_text, text_opt};
use crate::error::{Error, Result};
use crate::repo::WriteTx;

pub const BANK_ACCOUNTS: &str = "bank_accounts";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BankAccountInfo {
    pub id: String,
    pub name: String,
    pub bank: Option<String>,
    pub account_no: Option<String>,
    pub currency: String,
    pub notes: Option<String>,
    pub active: bool,
    /// Checks deposited here that haven't cleared or bounced yet.
    pub checks_pending_c: i64,
    pub checks_pending: i64,
    pub checks_cleared_c: i64,
    pub checks_bounced: i64,
    /// Posted bank transfers from clients received here.
    pub transfers_in_c: i64,
    /// TT payments made from here, in PHP where the FX is known. Stripped
    /// without the costs permission.
    pub tt_out_php_c: Option<i64>,
    pub tt_count: Option<i64>,
    /// Latest date of anything above.
    pub last_date: Option<String>,
}

impl BankAccountInfo {
    pub fn strip(&mut self, perms: &Perms) {
        if !perms.sees(Field::Costs) {
            self.tt_out_php_c = None;
            self.tt_count = None;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BankAccountInput {
    /// `None` adds an account.
    pub id: Option<String>,
    /// What staff pick from, e.g. "BDO Savings – Main".
    pub name: String,
    pub bank: String,
    pub account_no: String,
    pub currency: String,
    pub notes: String,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum BankMoveKind {
    /// A client's check deposited here.
    CheckDeposit,
    /// A client's bank transfer received here.
    TransferIn,
    /// A TT payment to a supplier made from here.
    TtOut,
}

/// Something that went into or out of an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BankMove {
    pub kind: BankMoveKind,
    /// Payment id, or TT payment id.
    pub id: String,
    /// Deposit date for checks, payment date otherwise.
    pub date: String,
    /// PAY number, or the PO's number for a TT.
    pub no: Option<String>,
    /// The PO of a TT.
    pub po_id: Option<String>,
    /// The paying client, or the supplier paid by a TT.
    pub party_id: Option<String>,
    pub party_name: Option<String>,
    /// Check bank and number, or the transfer's reference.
    pub detail: Option<String>,
    /// PHP; for a TT, the PHP amount if the FX is known.
    pub amount_c: Option<i64>,
    pub currency: String,
    pub amount_minor: i64,
    pub check_status: Option<CheckStatus>,
    pub cleared_date: Option<String>,
    pub bounced_date: Option<String>,
}

/// Every account, with what went through it. Sorted active first, by name.
pub fn list(conn: &Connection) -> Result<Vec<BankAccountInfo>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.bank, a.account_no, COALESCE(a.currency, 'PHP'), a.notes,
                COALESCE(a.active, 1),
                COALESCE(c.pending_c, 0), COALESCE(c.pending, 0), COALESCE(c.cleared_c, 0),
                COALESCE(c.bounced, 0), COALESCE(t.in_c, 0),
                MAX(COALESCE(c.last, ''), COALESCE(t.last, ''))
         FROM bank_accounts a
         LEFT JOIN (
             SELECT bank_account_id AS id,
                    SUM(CASE WHEN check_status = 'deposited' THEN COALESCE(amount_c, 0) END) AS pending_c,
                    SUM(check_status = 'deposited') AS pending,
                    SUM(CASE WHEN check_status = 'cleared' THEN COALESCE(amount_c, 0) END) AS cleared_c,
                    SUM(check_status = 'bounced') AS bounced,
                    MAX(deposited_date) AS last
             FROM payments
             WHERE _deleted = 0 AND status = 'posted' AND method = 'check'
               AND check_status IN ('deposited', 'cleared', 'bounced')
             GROUP BY bank_account_id
         ) c ON c.id = a.id
         LEFT JOIN (
             SELECT bank_account_id AS id, SUM(COALESCE(amount_c, 0)) AS in_c, MAX(date) AS last
             FROM payments
             WHERE _deleted = 0 AND status = 'posted' AND method = 'bank_transfer'
             GROUP BY bank_account_id
         ) t ON t.id = a.id
         WHERE a._deleted = 0
         ORDER BY COALESCE(a.active, 1) DESC, a.name COLLATE NOCASE, a.id",
    )?;
    let rows = stmt.query_map([], |r| {
        let last: String = r.get(12)?;
        Ok(BankAccountInfo {
            id: r.get(0)?,
            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            bank: r.get(2)?,
            account_no: r.get(3)?,
            currency: r.get(4)?,
            notes: r.get(5)?,
            active: r.get::<_, i64>(6)? != 0,
            checks_pending_c: r.get(7)?,
            checks_pending: r.get(8)?,
            checks_cleared_c: r.get(9)?,
            checks_bounced: r.get(10)?,
            transfers_in_c: r.get(11)?,
            tt_out_php_c: Some(0),
            tt_count: Some(0),
            last_date: (!last.is_empty()).then_some(last),
        })
    })?;
    let mut out: Vec<BankAccountInfo> = rows.collect::<Result<_, _>>()?;
    // TT amounts are in the PO's currency; convert each with its FX.
    for t in tt_moves(conn, None)? {
        let Some(a) = out.iter_mut().find(|a| Some(&a.id) == t.0.as_ref()) else {
            continue;
        };
        a.tt_count = a.tt_count.map(|n| n + 1);
        a.tt_out_php_c = match (a.tt_out_php_c, t.1.amount_c) {
            (Some(sum), Some(php)) => Some(sum + php),
            _ => None,
        };
        if a.last_date.as_deref().is_none_or(|d| d < t.1.date.as_str()) {
            a.last_date = Some(t.1.date.clone());
        }
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<BankAccountInfo>> {
    Ok(list(conn)?.into_iter().find(|a| a.id == id))
}

/// What went into and out of an account, newest first. TT payments only
/// with `costs`.
pub fn moves(conn: &Connection, id: &str, costs: bool) -> Result<Vec<BankMove>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.method, COALESCE(CASE WHEN p.method = 'check' THEN p.deposited_date END, p.date),
                n.no, p.client_id, c.name, p.check_bank, p.check_no, p.reference,
                COALESCE(p.amount_c, 0), p.check_status, p.cleared_date, p.bounced_date
         FROM payments p
         LEFT JOIN clients c ON c.id = p.client_id
         LEFT JOIN doc_numbers n ON n.ref_id = p.id AND n.series = 'PAY' AND n._deleted = 0
         WHERE p._deleted = 0 AND p.status = 'posted' AND p.bank_account_id = ?1
           AND (p.method = 'bank_transfer'
                OR (p.method = 'check' AND p.check_status IN ('deposited', 'cleared', 'bounced')))",
    )?;
    let rows = stmt.query_map([id], |r| {
        let check = r.get::<_, Option<String>>(1)?.as_deref() == Some("check");
        let amount: i64 = r.get(9)?;
        let detail = if check {
            let parts: Vec<String> = [r.get::<_, Option<String>>(6)?, r.get(7)?]
                .into_iter()
                .flatten()
                .collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        } else {
            r.get(8)?
        };
        Ok(BankMove {
            kind: if check {
                BankMoveKind::CheckDeposit
            } else {
                BankMoveKind::TransferIn
            },
            id: r.get(0)?,
            date: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            no: r.get(3)?,
            po_id: None,
            party_id: r.get(4)?,
            party_name: r.get(5)?,
            detail,
            amount_c: Some(amount),
            currency: "PHP".into(),
            amount_minor: amount,
            check_status: check
                .then(|| r.get::<_, Option<String>>(10))
                .transpose()?
                .map(|s| CheckStatus::parse(s.as_deref())),
            cleared_date: r.get(11)?,
            bounced_date: r.get(12)?,
        })
    })?;
    let mut out: Vec<BankMove> = rows.collect::<Result<_, _>>()?;
    if costs {
        out.extend(tt_moves(conn, Some(id))?.into_iter().map(|(_, m)| m));
    }
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.id.cmp(&a.id)));
    Ok(out)
}

/// TT payments that name an account (`account`, or any), with that account.
fn tt_moves(conn: &Connection, account: Option<&str>) -> Result<Vec<(Option<String>, BankMove)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.bank_account_id, t.id, t.date, n.no, t.po_id, p.supplier_id, s.name,
                t.reference, COALESCE(t.currency, 'PHP'), COALESCE(t.amount_minor, 0), t.fx_rate
         FROM supplier_payments t
         JOIN purchase_orders p ON p.id = t.po_id AND p._deleted = 0
         LEFT JOIN suppliers s ON s.id = p.supplier_id
         LEFT JOIN doc_numbers n ON n.ref_id = t.po_id AND n.series = 'PO' AND n._deleted = 0
         WHERE t._deleted = 0 AND t.bank_account_id IS NOT NULL
           AND (?1 IS NULL OR t.bank_account_id = ?1)",
    )?;
    let rows = stmt.query_map([account], |r| {
        let currency: String = r.get(8)?;
        let minor: i64 = r.get(9)?;
        let fx: Option<String> = r.get(10)?;
        Ok((
            r.get(0)?,
            BankMove {
                kind: BankMoveKind::TtOut,
                id: r.get(1)?,
                date: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                no: r.get(3)?,
                po_id: r.get(4)?,
                party_id: r.get(5)?,
                party_name: r.get(6)?,
                detail: r.get(7)?,
                amount_c: purchasing::php_of(&currency, minor, fx.as_deref()),
                currency,
                amount_minor: minor,
                check_status: None,
                cleared_date: None,
                bounced_date: None,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// An account a payment or TT may name: it exists and is active, unless it
/// is the one already stored (`current`). Empty means none.
pub(crate) fn usable(
    conn: &Connection,
    id: Option<&str>,
    current: Option<&str>,
) -> Result<Option<String>> {
    let Some(id) = id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let active: Option<i64> = conn
        .query_row(
            "SELECT COALESCE(active, 1) FROM bank_accounts WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    match active {
        None => Err(Error::Invalid("that bank account no longer exists".into())),
        Some(0) if current != Some(id) => Err(Error::Invalid(
            "that bank account is inactive; choose another".into(),
        )),
        Some(_) => Ok(Some(id.to_string())),
    }
}

pub fn save(tx: &mut WriteTx, input: &BankAccountInput) -> Result<String> {
    let name = require_text(&input.name, "a name for the account")?;
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM bank_accounts WHERE _deleted = 0
                        AND name = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        (&name, &input.id),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!(
            "a bank account named {name} already exists"
        )));
    }
    let data = json!({
        "name": name,
        "bank": text_opt(&input.bank),
        "account_no": text_opt(&input.account_no),
        "currency": check_currency(&input.currency)?,
        "notes": text_opt(&input.notes),
        "active": input.active,
    });
    match &input.id {
        Some(id) => {
            if tx.get(BANK_ACCOUNTS, id)?.is_none() {
                return Err(Error::Invalid("that bank account no longer exists".into()));
            }
            tx.update(BANK_ACCOUNTS, id, data)?;
            Ok(id.clone())
        }
        None => tx.insert(BANK_ACCOUNTS, data),
    }
}

/// Deletes an account no payment or TT names, even a voided one.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(BANK_ACCOUNTS, id)?.is_none() {
        return Err(Error::Invalid("that bank account no longer exists".into()));
    }
    let used: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM payments WHERE bank_account_id = ?1 AND _deleted = 0)
             OR EXISTS (SELECT 1 FROM supplier_payments WHERE bank_account_id = ?1 AND _deleted = 0)",
        [id],
        |r| r.get(0),
    )?;
    if used {
        return Err(Error::Invalid(
            "payments name this account; mark it inactive instead".into(),
        ));
    }
    tx.delete(BANK_ACCOUNTS, id)
}
