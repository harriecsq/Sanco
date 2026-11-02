# M14: Lots replace boxes

Decided with the user on 2026-11-02. Boxes and box serials (M11) are removed completely. Stock is tracked by lot.

## Decisions
- **A lot is one item plus its arrival month**, e.g. Aneroid Jul 2026. Shipments of one item in the same month share a lot.
- **Stock is per item, lot and location.** Every receipt, transfer, adjustment and DR line names a lot.
- **Lots are picked by hand** on DRs, transfers and adjustments. Dropdowns list the oldest lot first and show qty on hand and the serial range.
- **One expiry per lot**, for items that track expiry.
- **Serial range is two free-text fields on the lot:** serial from and serial to, shown as `ANE 1 – ANE 30`. Both are optional and typed by hand. No parsing, no counting against the qty, no continuing from the last range, no overlap check. It is a visual reference for customer complaints.
- **Individual pieces are not tracked.** DRs record the lot only.
- **Bookings stay in pieces with no lot.** The lot is chosen on the DR, where stock moves.

## Schema (migration 0014, schema v14)
**One-time exception to "migrations are additive only", approved by the user.** It is safe because peers on another schema version are refused (`repo.rs`, handshake), so no older PC can send box rows, and the real database has no boxes, items or stock (checked 2026-11-02).

Removed:
- Tables `boxes`, `dr_line_boxes`, with their registry entries.
- `doc_numbers` rows of series `BOX:*`, deleted the same way on every PC.
- Columns `items.serial_code`, `items.serial_digits`, `items.pcs_per_box`, `po_lines.pcs_per_box`, `purchase_orders.serial_year`, `box_id` on `stock_doc_lines` and `po_receipt_lines`.
- `expiry` on stock, receipt and DR lines (it moves to the lot).
- View `v_box_received`.

Added:
```sql
-- id = derived_id(item_id, "lot/YYYY-MM"), so PCs receiving offline get the same row.
-- Never deleted (deletes win the merge): a lot with no stock simply stays.
CREATE TABLE lots (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    item_id     TEXT,
    month       TEXT,  -- YYYY-MM, the arrival month
    expiry      TEXT,  -- YYYY-MM-DD or NULL
    serial_from TEXT,  -- free text, e.g. "ANE 1"
    serial_to   TEXT   -- free text, e.g. "ANE 30"
);
```
- `lot_id` on `stock_doc_lines`, `po_receipt_lines` and `dr_lines`.
- `v_stock_moves` and `v_stock` group by item, location and `lot_id`. Expiry comes from joining `lots`.

## Behavior
- **Receiving (RR and stock receipt):** the document date gives the month, which gives the lot. Each line has optional serial from, serial to and expiry inputs. Blank keeps what the lot has.
- **Lot edits:** serial range and expiry stay editable on the lot at any time. They are reference data, not posted stock.
- **Same month, second shipment:** same lot, so the from and to fields are edited to cover both.
- **Multiple lots on one DR:** one DR line per lot. Several DR lines already may point at one booking line.
- **Integrity:** stock can't go below zero per lot and location. Void-and-re-enter for stock documents is unchanged.

## Where lots and ranges show
- Lot dropdowns on DRs, transfers and adjustments.
- Stock receipt and RR forms and details.
- DR and invoice lines in Accounting.
- Inventory item detail, movements, and expiring stock on the Dashboard.
- New **Inventory → Lots** list with `FilterBar`: item, month, expiry, serial range, qty per location. Text search matches the typed text only, so "ANE 17" won't find `ANE 1 – ANE 30`; search "ANE" and read the ranges.

## Work order
1. Migration 0014, registry, `domain/lots.rs`; update `inventory.rs`, `delivery.rs`, `purchasing.rs`, `items.rs`, `bookings.rs`, `dashboard.rs`, `reports.rs`. Delete `domain/boxes.rs`.
2. Commands and bindings.
3. UI: stock doc form, DR request, orders and receiving, inventory, items, bookings, `format.ts`, new Lots page.
4. Tests: replace `tests/boxes.rs` with `tests/lots.rs`, fix the others, run `sim.rs`.
5. Update `CLAUDE.md` (boxes rules, glossary, the additive-only note) and `docs/PLAN.md`. Bump the version.
6. Inventory import: items, then one stock receipt per date and location, with lots from the cleaned file. Dry run on a `.dev` database first.

## Rollout
Schema v14: HARRIE-PC and the MacBook must update together; they won't sync until both have.
