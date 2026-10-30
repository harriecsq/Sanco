# M12 — DR or Invoice

**Status:** 🟡 Code done 2026-10-27, UI click-through pending. What was built and the deviations are in the M12 notes of the main plan.
**Requested:** 2026-10-27.

## Request (user's words)
> When giving receipt to the client, it allows you to select either DR (no VAT) or Invoice (has VAT). It's that simple. Everything else stays exactly the same. Ensure all screens that have DR also have this Invoice aspect as well. Segregate them as separate lists as well, for tables like the ones in each client's page.

## User answers (2026-10-27)
1. **Separate number series:** Invoices have their own BIR booklet and number series, with their own format and typed/auto mode in Settings.
2. **Existing DRs with VAT on become Invoices.** They keep the numbers they already have.
3. **The type can change while the receipt is not yet delivered.** After delivery, void and enter it again.
4. **Switching type frees the old number** instead of keeping it as spoiled (confirmed after an explanation). The audit log keeps the history.

## Design

### Type = the existing `vat` flag
- A receipt is still one row in `delivery_receipts`. **`vat = 0` means a DR and `vat = 1` means an Invoice.** No new column.
  - `v_dr_totals`, `v_receivables` and `v_allocations` already compute net, VAT and gross from `vat`/`vat_bp`, so the money logic doesn't change.
  - Existing VAT DRs become Invoices with no backfill, which matches answer 2.
  - Rust DTOs expose `kind: ReceiptKind { Dr, Invoice }` (serde `"dr"`/`"invoice"`), mapped from `vat`. Inputs take `kind`, not `vat`.
  - Rejected: a separate `kind` column. It would duplicate `vat`, and keeping the two in step would be a stored derived value.
- **Default when requesting:** Invoice if the booking has VAT, DR otherwise. Staff pick in the request modal. The booking's VAT checkbox stays as it is, as a quote setting and the default.
- **Switching type** (`update_dr`): allowed only while the status is requested or approved. Refuse once delivered, cancelled or void: "Delivered receipts can't change type. Void it and enter it again." Switching changes the gross, so the existing paid-vs-gross check in `check_update` applies. Payments only go to delivered receipts, so this is normally moot.

### Numbers
- New series **`SI`** (sales invoice) in `sync/numbering.rs` `series`. Add it to `MANUAL_SERIES` (typed from the booklet by default; `number_mode.SI = auto` switches it). In `settings.rs` `SERIES`, add `(series::SI, "Sales invoices")`. The default auto format is `SI-2026-0001`.
- **Which series a receipt's number lives in:** the series of its live `doc_numbers` row if it has one, otherwise the series that matches its type (DR→`DR`, Invoice→`SI`).
  - Legacy Invoices (VAT DRs from before M12) therefore keep their number in the `DR` series. That is physically true, because they were written in the DR booklet, so they can never clash with new invoice-booklet numbers.
  - Add a helper `delivery::number_series(tx, dr_id, kind)`. Use it in `number_of`, `check_reservation` (accept `DR` or `SI` to match), the `dr_number` command helper, `set_number` and `deliver` ("enter the invoice number first" for Invoices).
- **Type switch on a numbered receipt:** release the old number by *clearing* its `doc_numbers` row, not deleting it.
  - Deletes win the merge and the row id is `derived_id(ref_id, "number/{series}")`, so a delete would block switching back. Cleared means the number fields are emptied so the number is free again.
  - `clash_losers`, `max_seq`, `number_of`, `resolve_clashes` and the reservation checks must skip cleared rows.
  - Switching back reuses the same row through `assign_or_change`.
  - **This touches numbering, so `tests/sim.rs` must cover it:** add `SI` to `SERIES` and random type switches.
- **New migration (next free number after M11), schema bump:** create view `v_receipt_no` (`ref_id → series, no, manual, confirmed, clash`) that picks the one live number row of a receipt from `DR` or `SI`.
  - Every query that now joins `n.series='DR'` uses this view instead: `delivery.rs` SELECT and clash EXISTS (:787-806), `receivables.rs` :208, `payments.rs` :465-485, `reports.rs` :129, `inventory.rs` `item_moves` :801-806.
  - The last two currently don't filter by series at all, so a receipt with two number rows would appear twice. The view fixes that.
  - The schema bump also forces every PC to update together. An old build would show Invoices without numbers and couldn't deliver them.

### Screens (every DR screen gets the Invoice side; lists split by type)
| Where | Change |
|---|---|
| Sidebar / `accounting/layout.tsx` tabs | Two tabs: **Delivery receipts** and **Invoices**. New route folder `routes/accounting/invoices/`. Both render one shared `ReceiptsPage kind=…` component, moved out of `accounting/index.tsx`. Detail stays `?id=` on either route; a receipt opened on the wrong route redirects to its kind. |
| Receipt list | Filtered to its kind. Same FilterBar and statuses. The "No DR no. yet" and "Number used twice" texts and the search placeholder follow the kind ("No invoice no. yet", …). Column "DR no." / "Invoice no.". |
| Receipt detail | Title `DR 123` / `Invoice 456`. Steps "Approve invoice", "Invoice number (BIR booklet)", "Cancel invoice", "Void delivered invoice", and the number hints. A "Type" line showing DR (no VAT) or Invoice (VAT 12%). Totals as today. |
| `components/sales/dr-request.tsx` | **Type picker** at the top: DR — no VAT / Invoice — VAT {rate}%, defaulting from the booking. Live net/VAT/gross. Disabled with a hint when delivered. Titles "Request a DR/invoice for …", button "Request DR" / "Request invoice". |
| `components/sales/sales.tsx` | `DrNo` and the badges are kind-aware. Add `receiptLabel(kind)` / `receiptName(r)` ("DR 123", "Invoice 456", "Invoice (no number)") and use them everywhere `DR ${n}` is built by hand. |
| Booking detail (`routes/bookings/index.tsx`) | The "Delivery receipts" table becomes two tables, **Delivery receipts** and **Invoices**, each hidden when empty. One "Request DR / invoice" button (type chosen in the modal). "On DRs" becomes "On receipts". "Open DR(s)" counts both. |
| Client page (`routes/clients/index.tsx`) | `DrsTab` becomes two tabs, **Delivery receipts** and **Invoices** (same component, `kind` prop). Payments tab labels use `receiptName`. Balance summary "DR(s) · unpaid" is split per kind. |
| Receivables (`accounting/receivables`) | The per-client "unpaid DR(s)" count is split into DRs and invoices. The "Unpaid DRs" table becomes two tables, **Unpaid DRs** and **Unpaid invoices**. |
| Payments + allocation picker (`components/accounting/accounting.tsx`) | One picker, "Apply to DRs and invoices", with a **Type** column, because one payment can cover both and `fillOldest` runs across both. The allocations table in payment detail gets a Type column, and labels use `receiptName`. |
| Dashboard (Rust `dashboard.rs`) | KPI split into "DRs to deliver" and "Invoices to deliver". Issue "DR numbers used twice" is split per series (`?status=clash` on the matching route). "Paid more than their total" covers both. Booking KPI becomes "without a DR or invoice yet". To-do `REF_HREF` routes by kind. |
| Reports | Sales: "DRs" column split into "DRs" and "Invoices", plus a Type filter. Receivables report "By DR" becomes "By receipt" with a Type column (CSV too). |
| Inventory moves | "Delivered DR" or "Delivered invoice" by kind. The link goes to the right route. |
| Settings → Number formats | New "Sales invoices" row with the same typed/auto radio and format editor. Prefix hint "e.g. SI". |

Permissions: unchanged. Invoices use the same commands and the same `Accounting`/`Bookings` checks as DRs.

## Tests
- `tests/sales.rs`:
  - request as DR (gross = net) and as Invoice (gross = net × 1.12)
  - the type defaults from the booking
  - switch type before delivery, with and without a number: the old number is freed and can be typed on another receipt
  - switch refused after delivery
  - Invoice needs an `SI` number to deliver
  - typed SI numbers clash offline → flagged, not renumbered
  - DR and SI with the same digits don't clash
  - a legacy VAT DR lists as an Invoice and keeps its DR-series number
- `tests/accounting.rs`: a payment allocated across one DR and one Invoice; receivables and aging per kind.
- `tests/finish.rs`: split KPIs and issues; the sales report counts per kind.
- `tests/sim.rs`: `SI` series plus type switches, and invariants still hold (no live duplicate numbers, no cleared row counted).
- `registry_matches_schema`, clippy, `npm run build`. Click through over CDP on two instances: request an invoice, type its number, deliver, see it on the client's Invoices tab and receivables, pay a DR and an invoice with one payment.

## Docs to update when done
- CLAUDE.md:
  - VAT rule: "DR = no VAT, Invoice = VAT; the type is the receipt's `vat` flag"
  - business rule "DR number = BIR booklet number": add "invoice number = invoice booklet (`SI`)"
  - glossary: add Invoice
  - sales flow: "DR or Invoice"
  - typed-numbers paragraph: add `SI`
- Main plan Progress table: M12 row and notes.
- Bump the version in `Cargo.toml` (workspace) and `tauri.conf.json`. Schema bump: every PC must update together.
