//! Receivables: what clients owe on delivered DRs, and aging.
//!
//! Nothing here is stored. A delivered DR's gross (from its lines and the
//! VAT rate stored on it) minus the allocations of counting payments is its
//! balance (`v_receivables`). A payment counts unless it is void, or it is
//! a check that bounced or was replaced (`v_payment_counts`); so a bounced
//! check puts the balance back. What a counting payment doesn't allocate is
//! the client's credit.
//!
//! A DR is due its client's payment terms after delivery; aging buckets
//! count days past that due date.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::Serialize;

use crate::domain::delivery::ReceiptKind;
use crate::error::Result;
use crate::time::{add_days, days_between};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum AgingBucket {
    NotDue,
    Days1To30,
    Days31To60,
    Days61To90,
    Over90,
}

impl AgingBucket {
    pub fn of(days_overdue: i64) -> Self {
        match days_overdue {
            ..=0 => AgingBucket::NotDue,
            1..=30 => AgingBucket::Days1To30,
            31..=60 => AgingBucket::Days31To60,
            61..=90 => AgingBucket::Days61To90,
            _ => AgingBucket::Over90,
        }
    }
}

/// Amounts per aging bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Aging {
    pub not_due_c: i64,
    pub d1_30_c: i64,
    pub d31_60_c: i64,
    pub d61_90_c: i64,
    pub over_90_c: i64,
}

impl Aging {
    fn add(&mut self, bucket: AgingBucket, c: i64) {
        let slot = match bucket {
            AgingBucket::NotDue => &mut self.not_due_c,
            AgingBucket::Days1To30 => &mut self.d1_30_c,
            AgingBucket::Days31To60 => &mut self.d31_60_c,
            AgingBucket::Days61To90 => &mut self.d61_90_c,
            AgingBucket::Over90 => &mut self.over_90_c,
        };
        *slot += c;
    }

    fn merge(&mut self, o: &Aging) {
        self.not_due_c += o.not_due_c;
        self.d1_30_c += o.d1_30_c;
        self.d31_60_c += o.d31_60_c;
        self.d61_90_c += o.d61_90_c;
        self.over_90_c += o.over_90_c;
    }

    pub fn overdue_c(&self) -> i64 {
        self.d1_30_c + self.d31_60_c + self.d61_90_c + self.over_90_c
    }
}

/// A delivered DR or invoice with a balance (positive: owed; negative:
/// overpaid).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceivableDr {
    pub dr_id: String,
    pub kind: ReceiptKind,
    pub no: Option<String>,
    pub client_id: Option<String>,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    pub delivered_date: Option<String>,
    pub due_date: Option<String>,
    /// Days past the due date; zero or negative while not due.
    pub days_overdue: i64,
    pub bucket: AgingBucket,
    pub gross_c: i64,
    pub paid_c: i64,
    pub balance_c: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ClientBalance {
    pub client_id: String,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    pub terms_days: Option<i64>,
    pub credit_limit_c: Option<i64>,
    /// Owed on delivered DRs, by days past due.
    pub aging: Aging,
    /// Owed on delivered DRs (positive balances).
    pub outstanding_c: i64,
    /// Of that, past due.
    pub overdue_c: i64,
    /// Paid but not applied to a DR, plus DRs paid more than their total.
    pub credit_c: i64,
    /// Outstanding minus credit.
    pub balance_c: i64,
    /// Checks on hand or deposited, not cleared yet, that count as paid.
    pub uncleared_c: i64,
    /// Unpaid DRs and invoices.
    pub open_drs: i64,
    pub open_invoices: i64,
    /// DRs and invoices paid more than their total.
    pub overpaid_drs: i64,
    /// Days past due of the oldest unpaid DR, if any is overdue.
    pub oldest_overdue_days: Option<i64>,
}

impl ClientBalance {
    fn new(client_id: &str) -> Self {
        ClientBalance {
            client_id: client_id.to_string(),
            client_code: None,
            client_name: None,
            terms_days: None,
            credit_limit_c: None,
            aging: Aging::default(),
            outstanding_c: 0,
            overdue_c: 0,
            credit_c: 0,
            balance_c: 0,
            uncleared_c: 0,
            open_drs: 0,
            open_invoices: 0,
            overpaid_drs: 0,
            oldest_overdue_days: None,
        }
    }

    fn take(&mut self, d: &ReceivableDr) {
        if d.balance_c > 0 {
            self.aging.add(d.bucket, d.balance_c);
            self.outstanding_c += d.balance_c;
            match d.kind {
                ReceiptKind::Dr => self.open_drs += 1,
                ReceiptKind::Invoice => self.open_invoices += 1,
            }
            if d.days_overdue > 0 {
                self.oldest_overdue_days =
                    Some(self.oldest_overdue_days.unwrap_or(0).max(d.days_overdue));
            }
        } else if d.balance_c < 0 {
            self.credit_c -= d.balance_c;
            self.overpaid_drs += 1;
        }
    }

    fn finish(&mut self) {
        self.overdue_c = self.aging.overdue_c();
        self.balance_c = self.outstanding_c - self.credit_c;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceivablesReport {
    pub as_of: String,
    /// Clients with a balance, credit or uncleared checks.
    pub clients: Vec<ClientBalance>,
    /// Delivered DRs that aren't fully paid (or are overpaid).
    pub drs: Vec<ReceivableDr>,
    pub totals: Aging,
    pub outstanding_c: i64,
    pub credit_c: i64,
    pub uncleared_c: i64,
}

/// A DR's due date: delivery plus the client's terms.
pub fn due_date(delivered: Option<&str>, terms_days: Option<i64>) -> Option<String> {
    add_days(delivered?, terms_days.unwrap_or(0).max(0))
}

/// What counting payments have paid on a delivered DR.
pub fn dr_paid(conn: &Connection, dr_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(amount_c), 0) FROM v_allocations WHERE dr_id = ?1",
        [dr_id],
        |r| r.get(0),
    )?)
}

/// What counting payments have paid per delivered DR.
pub fn paid_by_dr(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt =
        conn.prepare_cached("SELECT dr_id, SUM(amount_c) FROM v_allocations GROUP BY dr_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Delivered DRs with a balance, of one client or all.
fn open_drs(conn: &Connection, client: Option<&str>, today: &str) -> Result<Vec<ReceivableDr>> {
    let mut stmt = conn.prepare_cached(
        "SELECT v.dr_id, n.no, v.client_id, cn.no, c.name, v.delivered_date, c.terms_days,
                v.gross_c, v.paid_c, v.balance_c, COALESCE(r.vat, 0)
         FROM v_receivables v
         JOIN delivery_receipts r ON r.id = v.dr_id
         LEFT JOIN v_receipt_no n ON n.dr_id = v.dr_id
         LEFT JOIN clients c ON c.id = v.client_id
         LEFT JOIN doc_numbers cn ON cn.ref_id = v.client_id AND cn.series = 'CL' AND cn._deleted = 0
         WHERE v.balance_c <> 0 AND (?1 IS NULL OR v.client_id = ?1)
         ORDER BY v.delivered_date, n.no, v.dr_id",
    )?;
    let rows = stmt.query_map([client], |r| {
        let delivered: Option<String> = r.get(5)?;
        let due = due_date(delivered.as_deref(), r.get(6)?);
        let days_overdue = due
            .as_deref()
            .and_then(|d| days_between(d, today))
            .unwrap_or(0);
        Ok(ReceivableDr {
            dr_id: r.get(0)?,
            kind: ReceiptKind::from_vat(r.get::<_, i64>(10)? != 0),
            no: r.get(1)?,
            client_id: r.get(2)?,
            client_code: r.get(3)?,
            client_name: r.get(4)?,
            delivered_date: delivered,
            due_date: due,
            days_overdue,
            bucket: AgingBucket::of(days_overdue),
            gross_c: r.get(7)?,
            paid_c: r.get(8)?,
            balance_c: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Per client: unapplied amounts of counting payments, and checks that
/// count but haven't cleared.
fn payment_sums(conn: &Connection, client: Option<&str>) -> Result<HashMap<String, (i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT p.client_id,
                SUM(p.amount_c - COALESCE(a.applied_c, 0)),
                SUM(CASE WHEN q.method = 'check' AND q.check_status IN ('on_hand', 'deposited')
                         THEN p.amount_c ELSE 0 END)
         FROM v_payment_counts p
         JOIN payments q ON q.id = p.payment_id
         LEFT JOIN (SELECT payment_id, SUM(amount_c) AS applied_c FROM v_allocations
                    GROUP BY payment_id) a ON a.payment_id = p.payment_id
         WHERE p.counts = 1 AND p.client_id IS NOT NULL AND (?1 IS NULL OR p.client_id = ?1)
         GROUP BY p.client_id",
    )?;
    let rows = stmt.query_map([client], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn fill_client(conn: &Connection, b: &mut ClientBalance) -> Result<()> {
    let row = conn.query_row(
        "SELECT n.no, c.name, c.terms_days, c.credit_limit_c FROM clients c
         LEFT JOIN doc_numbers n ON n.ref_id = c.id AND n.series = 'CL' AND n._deleted = 0
         WHERE c.id = ?1",
        [&b.client_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    );
    match row {
        Ok((code, name, terms, limit)) => {
            b.client_code = code;
            b.client_name = name;
            b.terms_days = terms;
            b.credit_limit_c = limit;
            Ok(())
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Every client's balance and aging as of `today`, with the DRs behind it.
pub fn report(conn: &Connection, today: &str) -> Result<ReceivablesReport> {
    let drs = open_drs(conn, None, today)?;
    let mut by_client: BTreeMap<String, ClientBalance> = BTreeMap::new();
    for d in &drs {
        let Some(c) = &d.client_id else { continue };
        by_client
            .entry(c.clone())
            .or_insert_with(|| ClientBalance::new(c))
            .take(d);
    }
    for (c, (unapplied, uncleared)) in payment_sums(conn, None)? {
        let b = by_client
            .entry(c.clone())
            .or_insert_with(|| ClientBalance::new(&c));
        b.credit_c += unapplied;
        b.uncleared_c += uncleared;
    }
    let mut totals = Aging::default();
    let (mut outstanding_c, mut credit_c, mut uncleared_c) = (0, 0, 0);
    let mut clients = Vec::new();
    for (_, mut b) in by_client {
        b.finish();
        if b.outstanding_c == 0 && b.credit_c == 0 && b.uncleared_c == 0 {
            continue;
        }
        fill_client(conn, &mut b)?;
        totals.merge(&b.aging);
        outstanding_c += b.outstanding_c;
        credit_c += b.credit_c;
        uncleared_c += b.uncleared_c;
        clients.push(b);
    }
    clients.sort_by(|a, b| {
        b.balance_c
            .cmp(&a.balance_c)
            .then_with(|| a.client_name.cmp(&b.client_name))
    });
    Ok(ReceivablesReport {
        as_of: today.to_string(),
        clients,
        drs,
        totals,
        outstanding_c,
        credit_c,
        uncleared_c,
    })
}

/// One client's balance as of `today`.
pub fn client_balance(conn: &Connection, client_id: &str, today: &str) -> Result<ClientBalance> {
    let mut b = ClientBalance::new(client_id);
    for d in open_drs(conn, Some(client_id), today)? {
        b.take(&d);
    }
    if let Some((unapplied, uncleared)) = payment_sums(conn, Some(client_id))?.remove(client_id) {
        b.credit_c += unapplied;
        b.uncleared_c += uncleared;
    }
    b.finish();
    fill_client(conn, &mut b)?;
    Ok(b)
}

/// A client's unpaid delivered DRs, oldest first.
pub fn client_drs(conn: &Connection, client_id: &str, today: &str) -> Result<Vec<ReceivableDr>> {
    open_drs(conn, Some(client_id), today)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_count_days_past_due() {
        assert_eq!(AgingBucket::of(-5), AgingBucket::NotDue);
        assert_eq!(AgingBucket::of(0), AgingBucket::NotDue);
        assert_eq!(AgingBucket::of(1), AgingBucket::Days1To30);
        assert_eq!(AgingBucket::of(30), AgingBucket::Days1To30);
        assert_eq!(AgingBucket::of(31), AgingBucket::Days31To60);
        assert_eq!(AgingBucket::of(90), AgingBucket::Days61To90);
        assert_eq!(AgingBucket::of(91), AgingBucket::Over90);
        assert_eq!(
            due_date(Some("2026-10-21"), Some(30)).as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(
            due_date(Some("2026-10-21"), None).as_deref(),
            Some("2026-10-21")
        );
        assert_eq!(due_date(None, Some(30)), None);
    }
}
