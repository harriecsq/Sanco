//! The Dashboard: KPIs per module and data issues, all computed on request
//! from the same functions the module screens use. Each KPI or issue only
//! appears for users who may view its module.
//!
//! Issues are what offline PCs can leave behind after a merge (negative
//! stock, a DR number typed twice, an overpaid DR, a PO line received twice,
//! duplicate codes) plus unresolved sync conflicts. They are checked live
//! instead of being written as to-dos, so they disappear once fixed.

use rusqlite::Connection;
use serde::Serialize;

use crate::domain::auth::{Action, Module, Perms};
use crate::domain::bd::{self, BdStatus};
use crate::domain::bookings::{self, BookingStatus, DeliveryProgress};
use crate::domain::delivery::{self, DrStatus};
use crate::domain::payments::{self, CheckStatus, PaymentStatus};
use crate::domain::purchasing::{self, PoStatus};
use crate::domain::{inventory, peso, receivables};
use crate::error::Result;
use crate::time::add_days;

/// Days ahead for "PDCs to deposit".
pub const DEPOSIT_WINDOW_DAYS: i64 = 7;
/// Names listed in an issue's detail before "+N more".
const SHOWN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Normal,
    Warn,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Kpi {
    pub key: String,
    /// The module it belongs to (for grouping).
    pub module: Module,
    pub label: String,
    pub value: i64,
    /// `value` is centavos.
    pub money: bool,
    pub hint: String,
    pub tone: Tone,
    /// Screen with the matching list.
    pub href: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Issue {
    pub key: String,
    pub title: String,
    pub detail: String,
    pub count: i64,
    pub href: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Dashboard {
    pub as_of: String,
    pub kpis: Vec<Kpi>,
    pub issues: Vec<Issue>,
}

struct Out {
    kpis: Vec<Kpi>,
    issues: Vec<Issue>,
}

impl Out {
    #[allow(clippy::too_many_arguments)]
    fn kpi(
        &mut self,
        module: Module,
        key: &str,
        label: &str,
        value: i64,
        money: bool,
        hint: String,
        tone: Tone,
        href: &str,
    ) {
        self.kpis.push(Kpi {
            key: key.into(),
            module,
            label: label.into(),
            value,
            money,
            hint,
            tone,
            href: href.into(),
        });
    }

    /// Adds an issue when `names` isn't empty.
    fn issue(&mut self, key: &str, title: &str, names: Vec<String>, href: &str) {
        if names.is_empty() {
            return;
        }
        self.issues.push(Issue {
            key: key.into(),
            title: title.into(),
            detail: listing(&names),
            count: names.len() as i64,
            href: href.into(),
        });
    }
}

/// "A, B, C +2 more".
fn listing(names: &[String]) -> String {
    let mut s = names
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > SHOWN {
        s.push_str(&format!(" +{} more", names.len() - SHOWN));
    }
    s
}

fn tone_if(warn: bool) -> Tone {
    if warn { Tone::Warn } else { Tone::Normal }
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn summary(conn: &Connection, perms: &Perms, today: &str) -> Result<Dashboard> {
    let sees = |m: Module| perms.can(m, Action::View);
    let mut out = Out {
        kpis: Vec::new(),
        issues: Vec::new(),
    };

    if sees(Module::Bd) {
        let open: Vec<_> = bd::list(conn)?
            .into_iter()
            .filter(|r| matches!(r.status, BdStatus::Open | BdStatus::Quoted))
            .collect();
        let due = open
            .iter()
            .filter(|r| r.follow_up.as_deref().is_some_and(|f| f <= today))
            .count() as i64;
        let quoted = open
            .iter()
            .filter(|r| r.status == BdStatus::Quoted)
            .count() as i64;
        out.kpi(
            Module::Bd,
            "bd_open",
            "Open requests",
            open.len() as i64,
            false,
            if due > 0 {
                format!("{} due", plural(due, "follow-up", "follow-ups"))
            } else {
                format!("{quoted} quoted")
            },
            tone_if(due > 0),
            if due > 0 { "/bd/?status=due" } else { "/bd/" },
        );
    }

    if sees(Module::Bookings) {
        let waiting = bookings::list(conn)?
            .into_iter()
            .filter(|b| b.status == BookingStatus::Confirmed && b.progress != DeliveryProgress::Full)
            .collect::<Vec<_>>();
        let no_dr = waiting
            .iter()
            .filter(|b| b.progress == DeliveryProgress::None && b.open_drs == 0)
            .count() as i64;
        out.kpi(
            Module::Bookings,
            "bookings_to_deliver",
            "Bookings to deliver",
            waiting.len() as i64,
            false,
            format!("{no_dr} without a DR yet"),
            Tone::Normal,
            "/bookings/?status=to_deliver",
        );
    }

    let sees_drs = sees(Module::Accounting) || sees(Module::Bookings);
    if sees_drs {
        let drs = delivery::list(conn)?;
        let requested = drs
            .iter()
            .filter(|d| d.status == DrStatus::Requested)
            .count() as i64;
        let approved = drs
            .iter()
            .filter(|d| d.status == DrStatus::Approved)
            .count() as i64;
        out.kpi(
            Module::Accounting,
            "drs_open",
            "DRs to deliver",
            requested + approved,
            false,
            format!("{requested} to approve · {approved} approved"),
            Tone::Normal,
            if requested > 0 {
                "/accounting/?status=requested"
            } else {
                "/accounting/?status=approved"
            },
        );
        let clashes: Vec<String> = drs
            .iter()
            .filter(|d| d.no_clash)
            .map(|d| d.no.clone().unwrap_or_default())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        out.issue(
            "dr_clash",
            "DR numbers used twice",
            clashes,
            "/accounting/?status=clash",
        );
    }

    if sees(Module::Accounting) {
        let r = receivables::report(conn, today)?;
        let overdue = r.totals.overdue_c();
        let overdue_clients = r.clients.iter().filter(|c| c.overdue_c > 0).count() as i64;
        let over_limit: Vec<String> = r
            .clients
            .iter()
            .filter(|c| c.credit_limit_c.is_some_and(|l| c.balance_c > l))
            .map(|c| c.client_name.clone().unwrap_or_default())
            .collect();
        out.kpi(
            Module::Accounting,
            "receivables",
            "Receivables",
            r.outstanding_c,
            true,
            if overdue > 0 {
                format!("{} overdue", peso(overdue))
            } else {
                "nothing overdue".into()
            },
            Tone::Normal,
            "/accounting/receivables/",
        );
        out.kpi(
            Module::Accounting,
            "overdue",
            "Overdue",
            overdue,
            true,
            if over_limit.is_empty() {
                plural(overdue_clients, "client", "clients")
            } else {
                format!(
                    "{} · {} over limit",
                    plural(overdue_clients, "client", "clients"),
                    over_limit.len()
                )
            },
            if !over_limit.is_empty() {
                Tone::Bad
            } else {
                tone_if(overdue > 0)
            },
            "/accounting/receivables/?bucket=overdue",
        );
        let overpaid: Vec<String> = r
            .drs
            .iter()
            .filter(|d| d.balance_c < 0)
            .map(|d| d.no.clone().unwrap_or_else(|| "DR without no.".into()))
            .collect();
        out.issue(
            "dr_overpaid",
            "DRs paid more than their total",
            overpaid,
            "/accounting/receivables/?bucket=credit",
        );

        let pays = payments::list(conn, today)?;
        let week_end = add_days(today, DEPOSIT_WINDOW_DAYS).unwrap_or_else(|| today.into());
        let live_checks = pays
            .iter()
            .filter(|p| p.status == PaymentStatus::Posted)
            .filter_map(|p| p.check.as_ref().map(|c| (p, c)));
        let (mut to_deposit, mut to_deposit_c, mut bounced, mut bounced_c) = (0, 0, 0, 0);
        for (p, c) in live_checks {
            if c.status == CheckStatus::OnHand
                && c.check_date.as_deref().is_some_and(|d| d <= week_end.as_str())
            {
                to_deposit += 1;
                to_deposit_c += p.amount_c;
            }
            if c.status == CheckStatus::Bounced {
                bounced += 1;
                bounced_c += p.amount_c;
            }
        }
        out.kpi(
            Module::Accounting,
            "checks_to_deposit",
            "Checks to deposit",
            to_deposit,
            false,
            format!("{} within {DEPOSIT_WINDOW_DAYS} days", peso(to_deposit_c)),
            tone_if(to_deposit > 0),
            "/accounting/checks/?status=to_deposit",
        );
        out.kpi(
            Module::Accounting,
            "checks_bounced",
            "Bounced checks",
            bounced,
            false,
            if bounced > 0 {
                format!("{} not replaced", peso(bounced_c))
            } else {
                "none".into()
            },
            if bounced > 0 { Tone::Bad } else { Tone::Normal },
            "/accounting/checks/?status=bounced",
        );
        let over_applied: Vec<String> = pays
            .iter()
            .filter(|p| p.status == PaymentStatus::Posted && p.unapplied_c < 0)
            .map(|p| p.no.clone().unwrap_or_default())
            .collect();
        out.issue(
            "payment_over_applied",
            "Payments applied for more than their amount",
            over_applied,
            "/accounting/payments/",
        );
    }

    if sees(Module::Inventory) {
        let stock = inventory::item_stock(conn, today)?;
        let low = stock.iter().filter(|s| s.active && s.low).count() as i64;
        let out_of_stock = stock
            .iter()
            .filter(|s| s.active && s.low && s.on_hand <= 0)
            .count() as i64;
        out.kpi(
            Module::Inventory,
            "low_stock",
            "Low stock",
            low,
            false,
            format!("{out_of_stock} out of stock"),
            tone_if(low > 0),
            "/inventory/low/",
        );
        let expiring = inventory::expiring(conn, today, inventory::NEAR_EXPIRY_DAYS)?;
        let expired = expiring
            .iter()
            .filter(|l| l.expiry.as_deref().is_some_and(|e| e < today))
            .count() as i64;
        out.kpi(
            Module::Inventory,
            "near_expiry",
            "Near expiry",
            expiring.len() as i64,
            false,
            if expired > 0 {
                format!("{} expired", plural(expired, "lot", "lots"))
            } else {
                format!("lots within {} days", inventory::NEAR_EXPIRY_DAYS)
            },
            if expired > 0 {
                Tone::Bad
            } else {
                tone_if(!expiring.is_empty())
            },
            "/inventory/expiry/",
        );
        let negative = stock
            .iter()
            .filter(|s| s.negative)
            .map(|s| s.code.clone())
            .collect();
        out.issue(
            "negative_stock",
            "Stock below zero",
            negative,
            "/inventory/?status=negative",
        );
    }

    if sees(Module::Orders) {
        let pos = purchasing::list(conn)?;
        let transit: Vec<_> = pos
            .iter()
            .filter(|p| matches!(p.status, PoStatus::Ordered | PoStatus::Shipped))
            .collect();
        let late = transit
            .iter()
            .filter(|p| p.expected_date.as_deref().is_some_and(|e| e < today))
            .count() as i64;
        let shipped = transit
            .iter()
            .filter(|p| p.status == PoStatus::Shipped)
            .count() as i64;
        out.kpi(
            Module::Orders,
            "po_transit",
            "POs in transit",
            transit.len() as i64,
            false,
            if late > 0 {
                format!("{late} late")
            } else {
                format!("{shipped} shipped")
            },
            tone_if(late > 0),
            if late > 0 {
                "/orders/?status=late"
            } else {
                "/orders/?status=to_receive"
            },
        );
        let over: Vec<String> = pos
            .iter()
            .filter(|p| p.status != PoStatus::Cancelled && p.over_received)
            .map(|p| p.no.clone().unwrap_or_default())
            .collect();
        out.issue(
            "po_over_received",
            "PO lines received more than ordered",
            over,
            "/orders/?status=over",
        );
    }

    if sees(Module::Items) {
        let dupes = duplicates(
            conn,
            "SELECT MIN(code) FROM items WHERE _deleted = 0 AND code IS NOT NULL
             GROUP BY code COLLATE NOCASE HAVING COUNT(*) > 1 ORDER BY 1",
        )?;
        out.issue(
            "item_duplicate",
            "Item codes used twice",
            dupes,
            "/items/?status=duplicate",
        );
    }
    if sees(Module::Clients) {
        let dupes = duplicates(
            conn,
            "SELECT MIN(name) FROM clients WHERE _deleted = 0 AND name IS NOT NULL
             GROUP BY TRIM(name) COLLATE NOCASE HAVING COUNT(*) > 1 ORDER BY 1",
        )?;
        out.issue(
            "client_duplicate",
            "Client names used twice",
            dupes,
            "/clients/?status=duplicate",
        );
    }
    if sees(Module::Settings) {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM conflicts WHERE _deleted = 0 AND COALESCE(resolved, 0) = 0",
            [],
            |r| r.get(0),
        )?;
        if n > 0 {
            out.issues.push(Issue {
                key: "conflicts".into(),
                title: "Sync conflicts to review".into(),
                detail: "Edits made on two PCs at once. Keep the current version or restore the other."
                    .into(),
                count: n,
                href: "/settings/conflicts/".into(),
            });
        }
    }

    Ok(Dashboard {
        as_of: today.to_string(),
        kpis: out.kpis,
        issues: out.issues,
    })
}

fn duplicates(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0))?;
    let names: Vec<Option<String>> = rows.collect::<Result<_, _>>()?;
    Ok(names.into_iter().flatten().collect())
}
