# M11 — Boxes and box serials (agreed 2026-10-27, done 2026-10-27 — see M11 notes in the main plan)

Merge into `docs/PLAN.md` (Progress table + M11 notes) when done.
Start only after the shipment-charges / clients WIP (migration 0010) is committed. Use migration **0011**, schema v11 (every PC updates together).

## User decisions (2026-10-27)
- **One serial per box** (not per shipment). DRs record **qty drawn from each box**, not only which box.
- **Pcs per box:** item default, override per PO line; the box remembers its size.
- **Boxes + loose pcs entry on the stock side only:** PO, receiving, stock docs, stock screens. Bookings/DRs stay in pcs (show box equivalent), but DR lines carry per-box draws.
- **Serial format per item:** series code (e.g. `MT`) + 2-digit year + `-` + seq padded to 3, 4 or 5 digits (`MT26-001`, `MT26-0001`, `MT26-00001`). Restarts yearly. Items sharing a code share a counter. Items without a code: no boxes, behave as today.
- **Year = PO's serial year,** chosen on the PO page (selector, defaults to current Manila year, **any year** allowed). Goods delivered next year keep the PO's year.
- **Serials assigned when the PO is marked ordered** (drafts use no numbers). Raising qty later adds boxes; lowering removes trailing unreceived boxes.
- **Offline clashes: ignore** (user: won't happen). Plain shared numbering is fine; no special "used twice" handling needed.
- **Existing stock:** no conversion/"box up" needed; test DB will be reset.

## Design sketch
- `items`: `serial_code TEXT`, `serial_digits INTEGER` (3/4/5), `pcs_per_box INTEGER`.
- `purchase_orders.serial_year TEXT`; `po_lines.pcs_per_box INTEGER` (+ qty entered as boxes + loose pcs, stored as total pcs).
- Replicated `boxes(item_id, po_line_id, pcs_per_box, notes)`; id = `derived_id(po_line_id, "box:<n>")`? (check: deletes win → removed box can't come back; use fresh ids or clear instead). Serial in `doc_numbers` (series `BOX:<code>`, period = year, pad = digits), ref_id = box id. Needs a batch `reserve` (N numbers in one claim round per peer).
- Box id on move lines: `po_receipt_lines.box_id` (one row per box received, expiry per box), `stock_doc_lines.box_id`, new `dr_line_boxes(dr_line_id, box_id, qty)` (derived ids, cleared not deleted). `v_stock_moves`/`v_stock` recreated with `box_id`.
- DR draws auto-filled: open boxes first, then earliest expiry, then lowest serial; editable. Void returns pcs to the same boxes.
- Display: "9 full + 1 open (7 pcs)"; per-item box list (serial, location, pcs left, expiry, PO).
- Tests in `sanco-core`: serial format/padding/year reset, assignment on ordered, qty change adds/removes boxes, receiving per box, DR draws + void, `v_stock` per box; sim still passes.
