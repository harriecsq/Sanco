//! Business Development: client and prospect requests and inquiries.
//!
//! open → quoted → won or lost. A request is for a client or for a prospect
//! (a name only). Lines are catalog items, or free text for something not
//! in the catalog, with an optional quoted net price. A request is won by
//! converting it into a draft booking, which needs a client.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::bookings::{self, BookingInput, BookingLineInput};
use crate::domain::{
    MAX_LINES, date_opt, line_amount, line_summary, replace_lines, require_date, text_opt,
};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};

pub const BD_REQUESTS: &str = "bd_requests";
pub const BD_LINES: &str = "bd_lines";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum BdStatus {
    Open,
    Quoted,
    /// Converted into a booking.
    Won,
    Lost,
}

impl BdStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BdStatus::Open => "open",
            BdStatus::Quoted => "quoted",
            BdStatus::Won => "won",
            BdStatus::Lost => "lost",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("quoted") => BdStatus::Quoted,
            Some("won") => BdStatus::Won,
            Some("lost") => BdStatus::Lost,
            _ => BdStatus::Open,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BdLineInput {
    /// The line being edited; `None` for a new line.
    pub id: Option<String>,
    /// A catalog item, or `None` with a description.
    pub item_id: Option<String>,
    pub description: String,
    pub qty: i64,
    /// Quoted net unit price.
    pub price_c: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BdInput {
    /// `None` creates a request.
    pub id: Option<String>,
    /// A client, or `None` with a prospect name.
    pub client_id: Option<String>,
    pub prospect_name: String,
    pub contact: String,
    pub date: String,
    /// Open, quoted or lost. Won is set by converting into a booking; a won
    /// request may keep it when edited.
    pub status: BdStatus,
    pub follow_up: Option<String>,
    pub vat: bool,
    pub notes: String,
    pub lost_reason: String,
    pub lines: Vec<BdLineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BdInfo {
    pub id: String,
    pub no: Option<String>,
    pub no_confirmed: bool,
    pub client_id: Option<String>,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    pub prospect_name: Option<String>,
    pub contact: Option<String>,
    pub date: String,
    pub status: BdStatus,
    pub follow_up: Option<String>,
    pub vat: bool,
    pub notes: Option<String>,
    pub lost_reason: Option<String>,
    pub summary: String,
    pub item_ids: Vec<String>,
    pub line_count: i64,
    /// Sum of the quoted lines, before VAT.
    pub quoted_net_c: i64,
    /// The booking it was converted into.
    pub booking_id: Option<String>,
    pub booking_no: Option<String>,
    pub created_at: i64,
    pub created_by_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BdLine {
    pub id: String,
    pub item_id: Option<String>,
    pub item_code: Option<String>,
    pub item_name: Option<String>,
    pub unit: Option<String>,
    pub description: Option<String>,
    pub qty: i64,
    pub price_c: Option<i64>,
    pub amount_c: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BdDetail {
    pub request: BdInfo,
    pub lines: Vec<BdLine>,
}

// Writing -------------------------------------------------------------------

/// `was_won`: the request is already won, so it may stay won.
fn data(conn: &Connection, input: &BdInput, was_won: bool) -> Result<serde_json::Value> {
    let client = input.client_id.as_deref().filter(|s| !s.is_empty());
    let prospect = text_opt(&input.prospect_name);
    match client {
        Some(c) => {
            let found: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM clients WHERE id = ?1 AND _deleted = 0)",
                [c],
                |r| r.get(0),
            )?;
            if !found {
                return Err(Error::Invalid("that client no longer exists".into()));
            }
        }
        None if prospect.is_none() => {
            return Err(Error::Invalid(
                "choose a client, or enter the prospect's name".into(),
            ));
        }
        None => {}
    }
    if input.status == BdStatus::Won && !was_won {
        return Err(Error::Invalid(
            "a request is won by converting it into a booking".into(),
        ));
    }
    let lost = input.status == BdStatus::Lost;
    Ok(json!({
        "client_id": client,
        "prospect_name": if client.is_some() { None } else { prospect },
        "contact": text_opt(&input.contact),
        "date": require_date(&input.date, "request date")?,
        "status": input.status.as_str(),
        "follow_up": date_opt(input.follow_up.as_deref(), "follow-up date")?,
        "vat": input.vat,
        "notes": text_opt(&input.notes),
        "lost_reason": if lost { text_opt(&input.lost_reason) } else { None },
    }))
}

fn line_rows(
    conn: &Connection,
    input: &BdInput,
) -> Result<Vec<(Option<String>, serde_json::Value)>> {
    if input.lines.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a request can have at most {MAX_LINES} lines"
        )));
    }
    let mut out = Vec::with_capacity(input.lines.len());
    for (i, line) in input.lines.iter().enumerate() {
        let item = line.item_id.as_deref().filter(|s| !s.is_empty());
        let description = text_opt(&line.description);
        let what = match item {
            Some(id) => conn
                .query_row(
                    "SELECT COALESCE(code, '') FROM items WHERE id = ?1 AND _deleted = 0",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .ok_or_else(|| Error::Invalid("one of the items no longer exists".into()))?,
            None => description.clone().ok_or_else(|| {
                Error::Invalid("choose an item or describe it on every line".into())
            })?,
        };
        if line.qty <= 0 {
            return Err(Error::Invalid(format!("enter a quantity for {what}")));
        }
        if line.price_c.is_some_and(|p| p < 0) {
            return Err(Error::Invalid(format!(
                "the price of {what} can't be negative"
            )));
        }
        out.push((
            line.id.clone(),
            json!({
                "line_no": i as i64 + 1,
                "item_id": item,
                "description": description,
                "qty": line.qty,
                "price_c": line.price_c,
            }),
        ));
    }
    Ok(out)
}

/// Checks a new request as [`create`] would, without writing.
pub fn validate(conn: &Connection, input: &BdInput) -> Result<()> {
    data(conn, input, false)?;
    line_rows(conn, input).map(drop)
}

/// Creates a request with the number reserved for it (`res.ref_id` is its
/// id). Returns the id.
pub fn create(tx: &mut WriteTx, input: &BdInput, res: &Reservation) -> Result<String> {
    if res.series != series::BD {
        return Err(Error::Invalid("wrong number series for a request".into()));
    }
    let mut v = data(tx.conn(), input, false)?;
    let lines = line_rows(tx.conn(), input)?;
    v["id"] = json!(res.ref_id);
    let id = tx.insert(BD_REQUESTS, v)?;
    replace_lines(tx, BD_LINES, "request_id", &id, lines)?;
    numbering::assign(tx, res, BD_REQUESTS)?;
    Ok(id)
}

fn status_of(tx: &WriteTx, id: &str) -> Result<BdStatus> {
    let row = tx
        .get(BD_REQUESTS, id)?
        .ok_or_else(|| Error::Invalid("that request no longer exists".into()))?;
    Ok(BdStatus::parse(row["status"].as_str()))
}

pub fn update(tx: &mut WriteTx, input: &BdInput) -> Result<()> {
    let id = input.id.clone().unwrap_or_default();
    let was_won = status_of(tx, &id)? == BdStatus::Won;
    let v = data(tx.conn(), input, was_won)?;
    let lines = line_rows(tx.conn(), input)?;
    tx.update(BD_REQUESTS, &id, v)?;
    replace_lines(tx, BD_LINES, "request_id", &id, lines)
}

/// Deletes a request that wasn't won. Its number is not given out again.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if status_of(tx, id)? == BdStatus::Won {
        return Err(Error::Invalid(
            "this request was converted into a booking and can't be deleted".into(),
        ));
    }
    for line in crate::domain::line_ids(tx, BD_LINES, "request_id", id)? {
        tx.delete(BD_LINES, &line)?;
    }
    tx.delete(BD_REQUESTS, id)
}

/// The booking a request converts into, with which lines' prices are the
/// quote.
fn booking_input(
    conn: &Connection,
    id: &str,
    client_id: Option<&str>,
    today: &str,
) -> Result<(BookingInput, Vec<bool>)> {
    let row: Option<(Option<String>, Option<String>, i64)> = conn
        .query_row(
            "SELECT client_id, status, COALESCE(vat, 0) FROM bd_requests
             WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (own_client, status, vat) =
        row.ok_or_else(|| Error::Invalid("that request no longer exists".into()))?;
    match BdStatus::parse(status.as_deref()) {
        BdStatus::Open | BdStatus::Quoted => {}
        s => {
            return Err(Error::Invalid(format!(
                "this request is already {}",
                s.as_str()
            )));
        }
    }
    let client = client_id
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or(own_client)
        .ok_or_else(|| {
            Error::Invalid("choose the client (add the prospect as a client first)".into())
        })?;
    let mut lines = Vec::new();
    let mut quoted = Vec::new();
    for (_, l) in lines_of(conn, Some(id))? {
        let Some(item_id) = l.item_id else { continue };
        quoted.push(l.price_c.is_some());
        lines.push(BookingLineInput {
            id: None,
            item_id,
            qty: l.qty,
            price_c: l.price_c,
            notes: String::new(),
        });
    }
    if lines.is_empty() {
        return Err(Error::Invalid(
            "add catalog items to the request before converting it".into(),
        ));
    }
    Ok((
        BookingInput {
            id: None,
            client_id: client,
            date: today.to_string(),
            vat: vat != 0,
            client_ref: String::new(),
            deliver_to: String::new(),
            notes: String::new(),
            lines,
        },
        quoted,
    ))
}

/// Checks a conversion as [`convert`] would, without writing.
pub fn check_convert(
    conn: &Connection,
    id: &str,
    client_id: Option<&str>,
    today: &str,
) -> Result<()> {
    let (input, _) = booking_input(conn, id, client_id, today)?;
    bookings::validate(conn, &input)
}

/// Converts a request into a draft booking (with the booking number
/// reserved for it) and marks it won. Lines without a catalog item are
/// left out; quoted prices carry over. Returns the booking's id.
pub fn convert(
    tx: &mut WriteTx,
    id: &str,
    client_id: Option<&str>,
    res: &Reservation,
) -> Result<String> {
    let today = crate::time::manila_date_string(tx.now_ms());
    let (input, quoted) = booking_input(tx.conn(), id, client_id, &today)?;
    let booking = bookings::insert(tx, &input, res, Some(id), &quoted)?;
    tx.update(
        BD_REQUESTS,
        id,
        json!({
            "status": BdStatus::Won.as_str(),
            "client_id": input.client_id,
            "prospect_name": null,
            "lost_reason": null,
        }),
    )?;
    Ok(booking)
}

// Reading -------------------------------------------------------------------

const SELECT: &str = "
    SELECT d.id, n.no, COALESCE(n.confirmed, 1), d.client_id, cn.no, c.name, d.prospect_name,
           d.contact, d.date, d.status, d.follow_up, COALESCE(d.vat, 0), d.notes, d.lost_reason,
           (SELECT b.id FROM bookings b WHERE b.bd_request_id = d.id AND b._deleted = 0
            ORDER BY b.created_at, b.id LIMIT 1),
           d.created_at, uc.display_name
    FROM bd_requests d
    LEFT JOIN doc_numbers n ON n.ref_id = d.id AND n.series = 'BD' AND n._deleted = 0
    LEFT JOIN clients c ON c.id = d.client_id
    LEFT JOIN doc_numbers cn ON cn.ref_id = d.client_id AND cn.series = 'CL' AND cn._deleted = 0
    LEFT JOIN users uc ON uc.id = d.created_by
    WHERE d._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<BdInfo> {
    Ok(BdInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        client_id: r.get(3)?,
        client_code: r.get(4)?,
        client_name: r.get(5)?,
        prospect_name: r.get(6)?,
        contact: r.get(7)?,
        date: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        status: BdStatus::parse(r.get::<_, Option<String>>(9)?.as_deref()),
        follow_up: r.get(10)?,
        vat: r.get::<_, i64>(11)? != 0,
        notes: r.get(12)?,
        lost_reason: r.get(13)?,
        booking_id: r.get(14)?,
        created_at: r.get(15)?,
        created_by_name: r.get(16)?,
        booking_no: None,
        summary: String::new(),
        item_ids: Vec::new(),
        line_count: 0,
        quoted_net_c: 0,
    })
}

fn lines_of(conn: &Connection, request: Option<&str>) -> Result<Vec<(String, BdLine)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.request_id, l.id, l.item_id, i.code, i.name, i.unit, l.description,
                COALESCE(l.qty, 0), l.price_c
         FROM bd_lines l LEFT JOIN items i ON i.id = l.item_id
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.request_id = ?1)
         ORDER BY l.request_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([request], |r| {
        let qty: i64 = r.get(7)?;
        let price: Option<i64> = r.get(8)?;
        Ok((
            r.get(0)?,
            BdLine {
                id: r.get(1)?,
                item_id: r.get(2)?,
                item_code: r.get(3)?,
                item_name: r.get(4)?,
                unit: r.get(5)?,
                description: r.get(6)?,
                qty,
                price_c: price,
                amount_c: price.map(|p| line_amount(qty, p)),
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn fill(conn: &Connection, d: &mut BdInfo, lines: &[BdLine]) -> Result<()> {
    d.summary = line_summary(
        lines
            .iter()
            .map(|l| (l.item_code.as_deref().or(l.description.as_deref()), l.qty)),
    );
    d.item_ids = lines.iter().filter_map(|l| l.item_id.clone()).collect();
    d.item_ids.sort();
    d.item_ids.dedup();
    d.line_count = lines.len() as i64;
    d.quoted_net_c = lines.iter().filter_map(|l| l.amount_c).sum();
    if let Some(b) = &d.booking_id {
        d.booking_no = numbering::number_of(conn, series::BOOKING, b)?;
    }
    Ok(())
}

/// Every request, newest first.
pub fn list(conn: &Connection) -> Result<Vec<BdInfo>> {
    let mut by_request: HashMap<String, Vec<BdLine>> = HashMap::new();
    for (req, line) in lines_of(conn, None)? {
        by_request.entry(req).or_default().push(line);
    }
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY d.date DESC, d.created_at DESC, d.id DESC"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let mut d = row?;
        let lines = by_request.remove(&d.id).unwrap_or_default();
        fill(conn, &mut d, &lines)?;
        out.push(d);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<BdDetail>> {
    let Some(mut request) = conn
        .query_row(&format!("{SELECT} AND d.id = ?1"), [id], from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let lines: Vec<BdLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    fill(conn, &mut request, &lines)?;
    Ok(Some(BdDetail { request, lines }))
}
