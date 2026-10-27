//! Boxes and their serials.
//!
//! An item with a serial code (e.g. `MT`) comes in boxes of a set size,
//! each with one serial: the code, the 2-digit year and a sequence padded to
//! the item's digits (`MT26-0001`). The sequence restarts yearly and items
//! sharing a code share it (series `BOX:MT`, period = the year, in
//! `doc_numbers` like other numbers). Items without a code have no boxes.
//!
//! A purchase order gets its boxes when it is marked ordered, in the PO's
//! serial year: full boxes of the line's size, then one box for the rest.
//! Changing the quantity later adds boxes or removes trailing ones that
//! weren't received. A stock receipt (SR) can make boxes too. Receiving
//! reports and stock documents name the box of each line; a DR line says how
//! many pieces it takes from each box ([`DR_LINE_BOXES`]). What is left in a
//! box is computed from `v_stock`, never stored.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::registry::DOC_NUMBERS;
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Batch};
use crate::time::{days_between, manila_year};

pub const BOXES: &str = "boxes";
pub const DR_LINE_BOXES: &str = "dr_line_boxes";

/// Sequence digits an item may use.
pub const SERIAL_DIGITS: [i64; 3] = [3, 4, 5];
pub const DEFAULT_DIGITS: i64 = 4;
/// Longest serial code, in letters.
pub const MAX_CODE: usize = 6;
/// Boxes one line may need.
pub const MAX_BOXES: usize = 5000;

/// The number series of a serial code.
pub fn series_of(code: &str) -> String {
    format!("BOX:{code}")
}

/// `MT26-0001`: code, last two digits of the year, sequence.
pub fn render(code: &str, year: &str, digits: i64, seq: i64) -> String {
    let yy = &year[year.len().saturating_sub(2)..];
    let width = digits.clamp(1, 12) as usize;
    format!("{code}{yy}-{seq:0width$}")
}

/// A serial code as stored: letters only, upper case. Blank is `None`.
pub(crate) fn check_code(text: &str) -> Result<Option<String>> {
    let code = text.trim().to_ascii_uppercase();
    if code.is_empty() {
        return Ok(None);
    }
    if code.len() > MAX_CODE || !code.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(Error::Invalid(format!(
            "a serial code is 1 to {MAX_CODE} letters, e.g. MT"
        )));
    }
    Ok(Some(code))
}

/// A four-digit serial year. Blank is `None`.
pub(crate) fn check_year(text: &str) -> Result<Option<String>> {
    let year = text.trim();
    if year.is_empty() {
        return Ok(None);
    }
    if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) || year.starts_with('0') {
        return Err(Error::Invalid(format!(
            "“{year}” is not a valid serial year"
        )));
    }
    Ok(Some(year.to_string()))
}

/// This year in Manila, for serials.
pub fn this_year(now_ms: i64) -> String {
    manila_year(now_ms).to_string()
}

/// An item's serial settings, for items that come in boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Serial {
    pub code: String,
    pub digits: i64,
    /// Default box size.
    pub per_box: Option<i64>,
    pub item_code: String,
    pub unit: Option<String>,
}

impl Serial {
    pub fn series(&self) -> String {
        series_of(&self.code)
    }

    /// Box size for a line: the line's, else the item's.
    pub fn size(&self, line: Option<i64>) -> Result<i64> {
        line.or(self.per_box).ok_or_else(|| {
            Error::Invalid(format!(
                "enter how many {} of {} go in one box",
                self.unit.as_deref().unwrap_or("pcs"),
                self.item_code
            ))
        })
    }
}

/// The item's serial settings, or `None` if it doesn't come in boxes.
pub(crate) fn serial_of(conn: &Connection, item_id: &str) -> Result<Option<Serial>> {
    /// Serial code, digits, box size, item code, unit.
    type Row = (
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let row: Option<Row> = conn
        .prepare_cached(
            "SELECT serial_code, serial_digits, pcs_per_box, code, unit FROM items
             WHERE id = ?1 AND _deleted = 0",
        )?
        .query_row([item_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .optional()?;
    Ok(row.and_then(|(code, digits, per_box, item_code, unit)| {
        code.filter(|c| !c.is_empty()).map(|code| Serial {
            code,
            digits: digits.unwrap_or(DEFAULT_DIGITS),
            per_box: per_box.filter(|n| *n > 0),
            item_code: item_code.unwrap_or_default(),
            unit,
        })
    }))
}

/// Box sizes for `qty` pieces at `per` a box: full boxes, then the rest.
pub fn box_sizes(qty: i64, per: i64) -> Vec<i64> {
    if qty <= 0 || per <= 0 {
        return Vec::new();
    }
    let mut sizes = vec![per; (qty / per) as usize];
    if qty % per > 0 {
        sizes.push(qty % per);
    }
    sizes
}

/// Checks a box size typed on a line. `None` = blank.
pub(crate) fn check_size(n: Option<i64>, item_code: &str) -> Result<Option<i64>> {
    match n {
        Some(n) if n <= 0 => Err(Error::Invalid(format!(
            "the box size of {item_code} must be at least 1"
        ))),
        n => Ok(n),
    }
}

// Serial numbers for a write ---------------------------------------------------

/// How many new serials a write needs in one series and year. The caller
/// reserves them ([`numbering::reserve_batch`]) before the write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialNeed {
    pub series: String,
    pub period: String,
    pub count: i64,
}

/// Serials reserved for one write, handed out in order.
#[derive(Debug, Clone, Default)]
pub struct Serials {
    batches: Vec<Batch>,
    used: HashMap<(String, String), i64>,
}

impl Serials {
    /// For writes that make no boxes.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn new(batches: Vec<Batch>) -> Self {
        Self {
            batches,
            used: HashMap::new(),
        }
    }

    fn take(&mut self, series: &str, period: &str) -> Result<(Batch, i64)> {
        let key = (series.to_string(), period.to_string());
        let used = self.used.entry(key).or_default();
        let mut skip = *used;
        for b in self
            .batches
            .iter()
            .filter(|b| b.series == series && b.period == period)
        {
            if skip < b.count {
                *used += 1;
                return Ok((b.clone(), b.first + skip));
            }
            skip -= b.count;
        }
        Err(Error::Invalid(
            "the box serials changed while saving; try again".into(),
        ))
    }
}

/// Sums new boxes per series and year.
fn needs_of<'a>(boxes: impl IntoIterator<Item = (&'a Serial, &'a str)>) -> Vec<SerialNeed> {
    let mut by: BTreeMap<(String, String), i64> = BTreeMap::new();
    for (serial, year) in boxes {
        *by.entry((serial.series(), year.to_string())).or_default() += 1;
    }
    by.into_iter()
        .map(|((series, period), count)| SerialNeed {
            series,
            period,
            count,
        })
        .collect()
}

/// Makes a box with the next reserved serial. Returns its id.
pub(crate) fn create_box(
    tx: &mut WriteTx,
    serials: &mut Serials,
    serial: &Serial,
    year: &str,
    data: serde_json::Value,
) -> Result<String> {
    let id = tx.insert(BOXES, data)?;
    let (batch, seq) = serials.take(&serial.series(), year)?;
    let no = render(&serial.code, year, serial.digits, seq);
    numbering::assign_seq(tx, &batch, seq, &no, BOXES, &id)?;
    Ok(id)
}

/// Deletes a box and its serial (the serial isn't given out again).
pub(crate) fn delete_box(tx: &mut WriteTx, id: &str) -> Result<()> {
    let numbers: Vec<String> = {
        let mut stmt = tx.conn().prepare_cached(
            "SELECT id FROM doc_numbers WHERE ref_id = ?1 AND ref_tbl = 'boxes' AND _deleted = 0",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for n in numbers {
        tx.delete(DOC_NUMBERS, &n)?;
    }
    tx.delete(BOXES, id)
}

/// A box's serial.
pub fn serial_no(conn: &Connection, box_id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT no FROM doc_numbers WHERE ref_id = ?1 AND ref_tbl = 'boxes' AND _deleted = 0
             ORDER BY id LIMIT 1",
        )?
        .query_row([box_id], |r| r.get(0))
        .optional()?
        .flatten())
}

/// A live box's item and size.
pub(crate) fn box_row(conn: &Connection, id: &str) -> Result<Option<(String, i64)>> {
    Ok(conn
        .prepare_cached(
            "SELECT COALESCE(item_id, ''), COALESCE(pcs, 0) FROM boxes
             WHERE id = ?1 AND _deleted = 0",
        )?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

/// The expiry a box came in with (its lot), if any stock came in yet.
pub(crate) fn box_expiry(conn: &Connection, box_id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT expiry FROM v_stock_moves
             WHERE box_id = ?1 AND qty > 0 AND kind IN ('po_receipt', 'receipt')
             ORDER BY date, doc_id LIMIT 1",
        )?
        .query_row([box_id], |r| r.get(0))
        .optional()?
        .flatten())
}

// Purchase orders --------------------------------------------------------------

/// A PO line as it will be saved, for planning its boxes.
#[derive(Debug, Clone)]
pub(crate) struct LineSpec {
    /// `None` for a line not written yet.
    pub id: Option<String>,
    pub item_id: String,
    pub qty: i64,
    pub per_box: Option<i64>,
}

#[derive(Debug, Clone)]
struct BoxRow {
    id: String,
    item_id: String,
    po_line_id: Option<String>,
    pcs: i64,
    received: bool,
}

/// Live boxes of a PO, lowest serial first.
fn po_box_rows(conn: &Connection, po_id: &str) -> Result<Vec<BoxRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT b.id, COALESCE(b.item_id, ''), b.po_line_id, COALESCE(b.pcs, 0),
                EXISTS (SELECT 1 FROM v_box_received v WHERE v.box_id = b.id)
         FROM boxes b
         LEFT JOIN doc_numbers n ON n.ref_id = b.id AND n.ref_tbl = 'boxes' AND n._deleted = 0
         WHERE b.po_id = ?1 AND b._deleted = 0
         ORDER BY n.period, n.seq, b.id",
    )?;
    let rows = stmt.query_map([po_id], |r| {
        Ok(BoxRow {
            id: r.get(0)?,
            item_id: r.get(1)?,
            po_line_id: r.get(2)?,
            pcs: r.get(3)?,
            received: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone)]
struct NewBox {
    /// Index into the PO's lines.
    line: usize,
    item_id: String,
    pcs: i64,
    serial: Serial,
}

/// What saving a PO does to its boxes.
#[derive(Debug, Clone, Default)]
pub(crate) struct PoPlan {
    year: String,
    remove: Vec<String>,
    resize: Vec<(String, i64)>,
    create: Vec<NewBox>,
}

impl PoPlan {
    pub fn needs(&self) -> Vec<SerialNeed> {
        needs_of(self.create.iter().map(|b| (&b.serial, self.year.as_str())))
    }

    /// Whether the PO has (or keeps) boxes the year was used for.
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.resize.is_empty() && self.create.is_empty()
    }
}

/// Plans a PO's boxes. `boxed`: the PO is placed (not a draft, not
/// cancelled), so its lines need boxes; otherwise boxes not received yet go.
/// Received boxes always stay as they are.
pub(crate) fn plan_po(
    conn: &Connection,
    po_id: &str,
    boxed: bool,
    year: &str,
    lines: &[LineSpec],
) -> Result<PoPlan> {
    let mut plan = PoPlan {
        year: year.to_string(),
        ..PoPlan::default()
    };
    let mut by_line: HashMap<String, Vec<BoxRow>> = HashMap::new();
    let mut loose = Vec::new();
    for b in po_box_rows(conn, po_id)? {
        match b.po_line_id.clone() {
            Some(l) => by_line.entry(l).or_default().push(b),
            None => loose.push(b),
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let own = line
            .id
            .as_ref()
            .and_then(|id| by_line.remove(id))
            .unwrap_or_default();
        let serial = if boxed {
            serial_of(conn, &line.item_id)?
        } else {
            None
        };
        let mut received_pcs = 0;
        let mut open = Vec::new();
        for b in own {
            if b.received {
                if b.item_id == line.item_id {
                    received_pcs += b.pcs;
                }
            } else if serial.is_some() && b.item_id == line.item_id {
                open.push(b);
            } else {
                plan.remove.push(b.id);
            }
        }
        let Some(serial) = serial else { continue };
        let size = serial.size(line.per_box)?;
        let sizes = box_sizes(line.qty - received_pcs, size);
        if sizes.len() > MAX_BOXES {
            return Err(Error::Invalid(format!(
                "{} would need {} boxes; at most {MAX_BOXES} on one line",
                serial.item_code,
                sizes.len()
            )));
        }
        let mut open = open.into_iter();
        for pcs in sizes {
            match open.next() {
                Some(b) if b.pcs == pcs => {}
                Some(b) => plan.resize.push((b.id, pcs)),
                None => plan.create.push(NewBox {
                    line: i,
                    item_id: line.item_id.clone(),
                    pcs,
                    serial: serial.clone(),
                }),
            }
        }
        plan.remove.extend(open.map(|b| b.id));
    }
    // Lines that are gone, and boxes without a line.
    for b in by_line.into_values().flatten().chain(loose) {
        if !b.received {
            plan.remove.push(b.id);
        }
    }
    Ok(plan)
}

/// Carries out a plan. `line_ids` are the PO's line ids as saved, in the
/// order the plan's lines were given.
pub(crate) fn apply_po(
    tx: &mut WriteTx,
    po_id: &str,
    plan: PoPlan,
    line_ids: &[String],
    serials: &mut Serials,
) -> Result<()> {
    for id in &plan.remove {
        delete_box(tx, id)?;
    }
    for (id, pcs) in &plan.resize {
        tx.update(BOXES, id, json!({ "pcs": pcs }))?;
    }
    for b in &plan.create {
        let line = line_ids
            .get(b.line)
            .ok_or_else(|| Error::Invalid("a line of this purchase order is missing".into()))?;
        create_box(
            tx,
            serials,
            &b.serial,
            &plan.year,
            json!({
                "item_id": b.item_id,
                "po_id": po_id,
                "po_line_id": line,
                "pcs": b.pcs,
            }),
        )?;
    }
    Ok(())
}

/// Whether the PO has any live box.
pub(crate) fn po_has_boxes(conn: &Connection, po_id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM boxes WHERE po_id = ?1 AND _deleted = 0)",
        [po_id],
        |r| r.get(0),
    )?)
}

/// One box of a PO line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoBox {
    pub id: String,
    pub serial: Option<String>,
    /// What it holds when full.
    pub pcs: i64,
    /// Received on posted receiving reports (0 = not yet).
    pub received: i64,
}

/// Boxes of a PO's lines, lowest serial first, by line id.
pub(crate) fn po_boxes(conn: &Connection, po_id: &str) -> Result<HashMap<String, Vec<PoBox>>> {
    let mut stmt = conn.prepare_cached(
        "SELECT b.po_line_id, b.id, n.no, COALESCE(b.pcs, 0), COALESCE(v.qty, 0)
         FROM boxes b
         LEFT JOIN doc_numbers n ON n.ref_id = b.id AND n.ref_tbl = 'boxes' AND n._deleted = 0
         LEFT JOIN v_box_received v ON v.box_id = b.id
         WHERE b.po_id = ?1 AND b._deleted = 0 AND b.po_line_id IS NOT NULL
         ORDER BY n.period, n.seq, b.id",
    )?;
    let rows = stmt.query_map([po_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            PoBox {
                id: r.get(1)?,
                serial: r.get(2)?,
                pcs: r.get(3)?,
                received: r.get(4)?,
            },
        ))
    })?;
    let mut out: HashMap<String, Vec<PoBox>> = HashMap::new();
    for row in rows {
        let (line, b) = row?;
        out.entry(line).or_default().push(b);
    }
    Ok(out)
}

/// Boxes per PO, for lists.
pub(crate) fn box_counts(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT po_id, COUNT(*) FROM boxes
         WHERE _deleted = 0 AND po_id IS NOT NULL GROUP BY po_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// Stock receipts ---------------------------------------------------------------

/// Serials a set of new boxes needs.
pub(crate) fn needs_for<'a>(
    boxes: impl IntoIterator<Item = &'a Serial>,
    year: &str,
) -> Vec<SerialNeed> {
    needs_of(boxes.into_iter().map(|s| (s, year)))
}

/// Deletes the boxes a voided stock receipt made, once nothing else moves
/// stock in or out of them (a DR draw counts too).
pub(crate) fn drop_unused_boxes_of_doc(tx: &mut WriteTx, doc_id: &str) -> Result<()> {
    let unused: Vec<String> = {
        let mut stmt = tx.conn().prepare_cached(
            "SELECT b.id FROM boxes b
             WHERE b.doc_id = ?1 AND b._deleted = 0
               AND NOT EXISTS (SELECT 1 FROM v_stock_moves m WHERE m.box_id = b.id)
               AND NOT EXISTS (SELECT 1 FROM dr_line_boxes d
                               WHERE d.box_id = b.id AND d._deleted = 0 AND d.qty IS NOT NULL)",
        )?;
        let rows = stmt.query_map([doc_id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for id in unused {
        delete_box(tx, &id)?;
    }
    Ok(())
}

// Box stock --------------------------------------------------------------------

/// What is left in one box at one location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BoxStock {
    pub box_id: String,
    pub serial: Option<String>,
    pub item_id: String,
    pub item_code: String,
    pub unit: Option<String>,
    pub location_id: String,
    pub location_name: Option<String>,
    pub expiry: Option<String>,
    pub days_left: Option<i64>,
    /// What the box holds when full.
    pub pcs: i64,
    /// Left in it (negative if offline PCs took the same pieces).
    pub qty: i64,
    /// Started: some pieces were taken out.
    pub open: bool,
    pub po_id: Option<String>,
    pub po_no: Option<String>,
}

/// Non-empty boxes of live items, optionally of one item and location,
/// lowest serial first.
pub fn box_stock(
    conn: &Connection,
    item_id: Option<&str>,
    location_id: Option<&str>,
    today: &str,
) -> Result<Vec<BoxStock>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.box_id, n.no, s.item_id, i.code, i.unit, s.location_id, l.name, s.expiry,
                COALESCE(b.pcs, 0), s.qty, b.po_id, pn.no
         FROM v_stock s
         JOIN boxes b ON b.id = s.box_id
         JOIN items i ON i.id = s.item_id AND i._deleted = 0
         LEFT JOIN locations l ON l.id = s.location_id
         LEFT JOIN doc_numbers n ON n.ref_id = b.id AND n.ref_tbl = 'boxes' AND n._deleted = 0
         LEFT JOIN doc_numbers pn ON pn.ref_id = b.po_id AND pn.series = 'PO' AND pn._deleted = 0
         WHERE s.qty <> 0 AND s.box_id IS NOT NULL
           AND (?1 IS NULL OR s.item_id = ?1) AND (?2 IS NULL OR s.location_id = ?2)
         ORDER BY i.code COLLATE NOCASE, s.item_id, n.series, n.period, n.seq, b.id,
                  l.sort, l.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map((item_id, location_id), |r| {
        let expiry: Option<String> = r.get(7)?;
        let (pcs, qty): (i64, i64) = (r.get(8)?, r.get(9)?);
        Ok(BoxStock {
            box_id: r.get(0)?,
            serial: r.get(1)?,
            item_id: r.get(2)?,
            item_code: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            unit: r.get(4)?,
            location_id: r.get(5)?,
            location_name: r.get(6)?,
            days_left: expiry.as_deref().and_then(|e| days_between(today, e)),
            expiry,
            pcs,
            qty,
            open: qty < pcs,
            po_id: r.get(10)?,
            po_no: r.get(11)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// DR draws ---------------------------------------------------------------------

/// Pieces a DR line takes from one box.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BoxDrawInput {
    pub box_id: String,
    pub qty: i64,
}

/// A DR line's draw from one box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BoxDraw {
    pub box_id: String,
    pub serial: Option<String>,
    pub expiry: Option<String>,
    pub qty: i64,
}

/// Picks boxes for `qty` pieces of an item at a location: started boxes
/// first, then the earliest expiry, then the lowest serial. `available` is
/// what each box has for this DR (stock plus what the DR itself already
/// took, minus what earlier lines took). Returns what it could fill.
pub(crate) fn auto_draws(
    order: &[(String, Option<String>)],
    available: &HashMap<String, i64>,
    sizes: &HashMap<String, i64>,
    qty: i64,
) -> Vec<(String, Option<String>, i64)> {
    let mut boxes: Vec<(usize, &String, &Option<String>, i64)> = order
        .iter()
        .enumerate()
        .filter_map(|(i, (id, expiry))| {
            let left = available.get(id).copied().unwrap_or(0);
            (left > 0).then_some((i, id, expiry, left))
        })
        .collect();
    boxes.sort_by(|a, b| {
        let open_a = a.3 < sizes.get(a.1).copied().unwrap_or(0);
        let open_b = b.3 < sizes.get(b.1).copied().unwrap_or(0);
        open_b
            .cmp(&open_a)
            .then_with(|| match (a.2, b.2) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then(a.0.cmp(&b.0))
    });
    let mut out = Vec::new();
    let mut need = qty;
    for (_, id, expiry, left) in boxes {
        if need <= 0 {
            break;
        }
        let take = left.min(need);
        out.push((id.clone(), expiry.clone(), take));
        need -= take;
    }
    out
}

/// Draws of a DR's lines, by line id (cleared ones left out).
pub(crate) fn draws_of(
    conn: &Connection,
    dr_id: Option<&str>,
) -> Result<HashMap<String, Vec<BoxDraw>>> {
    let mut stmt = conn.prepare_cached(
        "SELECT d.dr_line_id, d.box_id, n.no, d.expiry, d.qty
         FROM dr_line_boxes d
         LEFT JOIN doc_numbers n ON n.ref_id = d.box_id AND n.ref_tbl = 'boxes' AND n._deleted = 0
         WHERE d._deleted = 0 AND d.qty IS NOT NULL AND (?1 IS NULL OR d.dr_id = ?1)
         ORDER BY d.dr_line_id, n.period, n.seq, d.box_id",
    )?;
    let rows = stmt.query_map([dr_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            BoxDraw {
                box_id: r.get(1)?,
                serial: r.get(2)?,
                expiry: r.get(3)?,
                qty: r.get(4)?,
            },
        ))
    })?;
    let mut out: HashMap<String, Vec<BoxDraw>> = HashMap::new();
    for row in rows {
        let (line, d) = row?;
        out.entry(line).or_default().push(d);
    }
    Ok(out)
}

/// Makes a DR line's draws match `draws` (box, expiry, qty): new ones are
/// added, changed ones updated, the rest cleared.
pub(crate) fn set_draws(
    tx: &mut WriteTx,
    dr_id: &str,
    line_id: &str,
    draws: &[(String, Option<String>, i64)],
) -> Result<()> {
    let old: Vec<String> = {
        let mut stmt = tx.conn().prepare_cached(
            "SELECT id FROM dr_line_boxes
             WHERE dr_line_id = ?1 AND _deleted = 0 AND qty IS NOT NULL",
        )?;
        let rows = stmt.query_map([line_id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut keep = Vec::new();
    for (box_id, expiry, qty) in draws {
        let id = crate::ids::derived_id(line_id, &format!("box/{box_id}"));
        tx.upsert(
            DR_LINE_BOXES,
            &id,
            json!({
                "dr_id": dr_id,
                "dr_line_id": line_id,
                "box_id": box_id,
                "expiry": expiry,
                "qty": qty,
            }),
        )?;
        keep.push(id);
    }
    for id in old.into_iter().filter(|id| !keep.contains(id)) {
        tx.update(DR_LINE_BOXES, &id, json!({ "qty": null }))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials_render() {
        assert_eq!(render("MT", "2026", 3, 1), "MT26-001");
        assert_eq!(render("MT", "2026", 4, 1), "MT26-0001");
        assert_eq!(render("MT", "2027", 5, 42), "MT27-00042");
        assert_eq!(render("MT", "2026", 3, 1234), "MT26-1234");
    }

    #[test]
    fn sizes_and_codes() {
        assert_eq!(
            box_sizes(97, 10),
            vec![10, 10, 10, 10, 10, 10, 10, 10, 10, 7]
        );
        assert_eq!(box_sizes(20, 10), vec![10, 10]);
        assert_eq!(box_sizes(3, 10), vec![3]);
        assert!(box_sizes(0, 10).is_empty());
        assert_eq!(check_code(" mt ").unwrap().as_deref(), Some("MT"));
        assert_eq!(check_code("").unwrap(), None);
        assert!(check_code("M2").is_err());
        assert!(check_code("ABCDEFG").is_err());
        assert_eq!(check_year("2025").unwrap().as_deref(), Some("2025"));
        assert!(check_year("26").is_err());
    }

    #[test]
    fn draws_prefer_open_then_expiry_then_serial() {
        let order: Vec<(String, Option<String>)> = vec![
            ("a".into(), Some("2027-05-01".into())),
            ("b".into(), Some("2027-01-01".into())),
            ("c".into(), Some("2027-01-01".into())),
            ("d".into(), Some("2028-01-01".into())),
        ];
        let sizes: HashMap<String, i64> = ["a", "b", "c", "d"]
            .iter()
            .map(|k| (k.to_string(), 10))
            .collect();
        let available: HashMap<String, i64> = [("a", 10), ("b", 10), ("c", 10), ("d", 4)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        let got = auto_draws(&order, &available, &sizes, 25);
        let ids: Vec<(&str, i64)> = got.iter().map(|(id, _, q)| (id.as_str(), *q)).collect();
        // d is started; then b and c (earliest expiry, lower serial first).
        assert_eq!(ids, vec![("d", 4), ("b", 10), ("c", 10), ("a", 1)]);
    }
}
