# Sanco Desktop App — Implementation Plan

## Context
Sanco is a 5-employee Philippine wholesaler that imports medical equipment from Taiwan and China suppliers and distributes it to retailers. They need one desktop app for their whole workflow, from client inquiry to delivery and collection, and from supplier order to stock. Every PC must see the same data, synced over Tailscale. They have no always-on server PC and no paid services. The repo is empty except for a README (`C:\Users\Harrie\Sanco`).

**Decisions already made with the user**
- **Stack:** Tauri 2.12 + QwikCity 1.20.2 as a static SPA. A Rust backend owns the database, sync and business rules.
- **Platforms:** Windows desktop first. Mobile can come later, but only as a client of an online desktop PC.
- **Sync:** peer-to-peer with no host. Every PC keeps a full SQLite replica and syncs with whichever peers are online over Tailscale.
- **Logins:** one login per person, with an admin or staff role. Everything is recorded in an audit trail.
- **Checks:** only checks received from clients, including PDCs. Supplier payments are telegraphic transfers, entered as plain fields on the PO.
- **Currency:** sales in PHP. POs in USD/CNY/TWD, with an exchange rate entered on each PO.
- **Inventory:** quantity per location plus expiry date (no lots or serial numbers).
- **Printing:** none for now.
- **Item costing:** a manual cost sheet per item: supplier price × FX + freight + duties + brokerage + other charges = landed cost, then margin on top.
- **Client pricing:** both a margin tier per client and a manual price per client per item.
- **Document numbers:** shared, with no PC prefix (e.g. DR-0042). Assigned instantly. A rare clash is auto-renumbered (see Sync §Numbering).

## Modules (8)
_Business Development was removed on 2026-10-27 (user decision); see "BD removal notes" under Progress._
1. **Dashboard:** KPIs, alerts and a to-do list.
   - KPIs: open requests, bookings awaiting DR, DRs to deliver, receivables due or overdue, PDCs due to deposit this week, bounced checks, low stock, stock near expiry, POs in transit.
   - Alerts: sync conflicts and data issues.
   - To-dos: personal or shared, assignable, with due dates.
2. ~~**Business Development:**~~ (removed 2026-10-27) client requests and inquiries (client or prospect, requested items, qty, quoted rate, follow-up date). Status: open → quoted → won/lost. A won request converts to a Booking.
3. **Booking:** confirmed client orders (lines: item, qty, rate taken from the client's rate). Status: draft → confirmed → closed/cancelled. Creates DR requests.
4. **Accounting:**
   - **DR requests:** requested → approved → delivered/cancelled, with the source location per DR. "Delivered" takes stock out.
   - **Receivables:** delivered DR totals minus payments, with aging buckets.
   - **Checks received:** bank, check no., check date (PDC if in the future), amount. Status: on hand → deposited → cleared / bounced / replaced.
   - **Payments:** cash, check or bank transfer, allocated to DRs.
5. **Items:** the item catalog.
   - Catalog fields: code, name, unit, category, supplier, tracks-expiry flag, reorder level.
   - **Cost sheet:** supplier unit price, currency, FX, freight per unit, duties (% or amount), brokerage, other charges → landed cost in PHP.
   - **Pricing:** default margin % → base selling price.
   - **Per-client rates:** a manual price per client per item. Shows the margin and warns when the price is below cost.
6. **Inventory:** stock tracking only.
   - On-hand quantity by item, location and expiry date.
   - Locations: Office and Warehouse, with more addable.
   - Manual stock receipts, adjustments and transfers between locations.
   - Near-expiry and low-stock lists.
7. **Order:**
   - **Suppliers:** TW/CN/other, with a default currency.
   - **Purchase orders:** currency, FX and expected arrival. Status: draft → ordered → shipped → arrived → closed.
   - **TT payment entries:** date, amount, currency, FX, reference.
   - **Receiving:** receive into a location with expiry dates. Received-vs-ordered status is computed.
8. **Clients:** client list (code, name, contact, TIN, address, payment terms, margin tier %, credit limit). The detail view shows the client's rates, bookings, DRs, balance and checks.
9. **Reports:** shown on screen with CSV export.
   - Sales by client, item and month.
   - Receivables aging.
   - Collections and checks (PDC calendar, bounced checks).
   - Stock by location, and expiring stock.
   - PO spend by supplier (PHP equivalent).
   - ~~Business Development conversion (won/lost).~~ (removed 2026-10-27)
   - Item margin.

**Settings (admin):** users and per-user module access, devices and sync status, conflicts, backups, company info, number formats.

**Price resolution for a client and item:**
1. The manual client-item rate, if set.
2. Otherwise landed cost × the client's margin tier, if the client has one.
3. Otherwise the item's base price (landed cost × the item's default margin).

The UI shows which rule produced the price.

## Architecture

### Layout
```
Sanco/
  CLAUDE.md
  Cargo.toml                      # workspace
  crates/sanco-core/              # pure Rust, no Tauri dep → testable with cargo test
    src/db/{conn.rs, registry.rs, migrations/}
    src/sync/{hlc.rs, merge.rs, versions.rs, protocol.rs, numbering.rs, invariants.rs}
    src/domain/{auth, users, clients, suppliers, items, pricing, inventory, bd,
                bookings, delivery, payments, checks, purchasing, todos, reports, dashboard}.rs
    src/repo.rs                   # THE ONLY write path
    tests/sim.rs                  # multi-peer convergence simulation
  crates/sanco-net/              # sync transport (no Tauri dep): server, client, node loop, join codes, tests/net.rs
  src-tauri/src/{lib.rs, state.rs, commands/<module>.rs}
  src/                            # QwikCity static SPA
    routes/{login, index(dashboard), bd, bookings, accounting/{dr,receivables,checks,payments},
            items/{catalog,costing,rates}, inventory/{stock,movements}, orders/{pos,suppliers},
            clients, reports, settings/{users,devices,sync,backups,company}}
    lib/{bindings.ts (generated by tauri-specta), use-query.ts, money.ts, dates.ts}
    components/{ui, data-table, form}
```

### Rust crates and plugins
- **Database:** `rusqlite` (bundled) and `rusqlite_migration`.
- **Networking:** `axum` 0.8, `reqwest` (rustls), `tower-http`.
- **IDs and money:** `uuid` (v7, v5), `rust_decimal` for FX rates.
- **Auth:** `argon2`, `hkdf`, `sha2`, `subtle`.
- **Dates:** `jiff` or `chrono`.
- **Utilities:** `serde_json`, `thiserror`, `tracing`.
- **Tests:** `rand_chacha`, `proptest`, `tempfile`.
- **TypeScript bindings:** `tauri-specta =2.0.0-rc.25` + `specta =2.0.0-rc.25` + `specta-typescript 0.0.12`, pinned exactly, with BigInt exported as `number`.
- **Tauri plugins:** single-instance, log, autostart, window-state, dialog, notification, and the `tray-icon` feature.
- **Frontend:** Tailwind v4 via `@tailwindcss/vite`. Check in M0 that `@qwik-ui/headless` works with Qwik 1.20; otherwise use plain Tailwind components.

### Rules (these go in CLAUDE.md)
- **Writes:** all writes go through `repo.rs`. It stamps the HLC, bumps `row_versions`, writes `audit_log` and emits a `db-changed` event so the UI refetches.
- **No stored derived values.** Stock on hand, receivables, PO receiving status and landed cost are SQL views or computed values, never columns.
- **Generated child rows** (if any) use deterministic IDs: `UUIDv5(source_id, kind)`.
- **No SQLite foreign keys** on replicated tables, because rows can arrive out of order.
- **Units:**
  - Money is `i64` centavos (minor units for foreign currency).
  - FX is decimal text.
  - Quantities are integers in the base unit.
  - Timestamps are UTC milliseconds, displayed in Asia/Manila.
  - Calendar dates are `YYYY-MM-DD`.
- **Migrations are additive only.** Backfills are deterministic and must not bump the HLC or `row_versions`. Run `VACUUM INTO` before each migration.
- **Qwik static gotchas:** call `invoke` only in the browser (`useVisibleTask$`, event handlers). No `routeLoader$` or `server$`. No dynamic `[id]` routes; use `?id=` query params.
- **Database connections:** one write connection on a dedicated thread with a job channel, plus read connections. WAL with `synchronous=FULL`.

### Sync design
- **Row metadata:** every replicated row has `id (UUIDv7), _hlc, _site, _vv, _deleted, created_at, created_by, updated_by`. `_vv` is a version vector as JSON text (`{site: hlc}`), one entry per site that ever wrote the row. _(Changed in M1: replaced `_base_hlc`; see M1 notes.)_
- **Local-only tables:** `meta(site_id, epoch, fingerprint, last_hlc, ver)`, `row_versions`, `peer_watermarks`, `number_claims` (M2).
- **Merge rule:** whole-row; keep the greatest `(deleted, hlc, site)`.
  - Deletes always win.
  - If neither version's `_vv` dominates the other, two people edited the same row without seeing each other's edit. The losing version is saved to `conflicts` and shown on the Dashboard, so no edit is silently lost. Identical data (e.g. the same derived row made on two PCs) is not a conflict.
- **Delta sync:** a pull returns rows with `row_versions.ver > watermark`. Rows accepted from a peer bump the local version, so changes pass through intermediate PCs (A→B→C works even if A and C are never online together). Echoes are ignored.
- **Transport:** axum on `0.0.0.0:PORT`.
  - Only addresses in Tailscale's ranges are accepted (`100.64.0.0/10`, `fd7a:115c:a1e0::/48`).
  - Requests need a bearer token derived from the workspace sync key (HKDF).
  - After each commit, a debounced `POST /poke` goes to online peers, which then pull. Every PC also pulls every 30 s and when a peer reconnects.
- **Peers:** a replicated `devices` table (id = site id; host = Tailscale IPv4, port, app/schema version, last_seen). Each PC updates its own row. _(Changed in M3: IP instead of MagicDNS name; see M3 notes.)_
- **Joining:**
  - The first PC creates the workspace, the sync key and the admin user.
  - A new PC enters a join code: base32(sync key + the inviting PC's host:port).
  - It receives a snapshot of the database, then generates its own `site_id` and `epoch`.
- **Handshake checks:**
  - Different schema version: no sync, show "update required". Local work continues.
  - Clock more than 2 minutes ahead: reject the batch and show "PC-x clock is wrong".
  - Known site_id with a different epoch (restored or copied database): reject.
- **Invariant checks:** negative stock, over-allocated payments, duplicate codes, numbers used twice and similar. They never block anything. _(Changed in M10: computed live as Dashboard "issues" instead of written as to-dos after each sync; see M10 notes.)_
- **Staying online:**
  - Closing the window minimizes to the tray instead of quitting.
  - The app autostarts on login.
  - The NSIS install hook adds a firewall rule allowing the port from `100.64.0.0/10`, which needs a per-machine install.
  - Tailscale "Shields Up" must be off.
- **Backups:** daily `VACUUM INTO`, keeping 14 days. Every PC is also a full replica.

### Numbering (DR-0042 style, shared, no prefix)
1. Next number = the higher of this PC's max and the maxes reported by online peers, plus 1.
2. The number is claimed with every reachable peer before it is used. A peer rejects it if it already has that number, and the PC retries with +1.
3. **A clash is only possible** if two PCs can't reach each other (Tailscale down, offline laptop) and both create the same document type. After the merge, the earlier holder by `(assigned_at, id)` keeps the number _(changed in M2 from `(hlc, site)`; see M2 notes)_. Only the PC that created the later record renumbers it, so all PCs agree. Both users are notified and the audit trail records old → new.
4. If no peers are reachable, the document shows a small warning: "offline — number may change".
5. The same service gives out client and supplier codes. Item codes are typed in by staff; duplicates are flagged and an admin can merge them.
6. _(Added in M7.)_ DR numbers are typed from the BIR booklet by default (setting `number_mode.DR`): claimed exactly with peers (`reserve_exact`), never renumbered; an offline clash is flagged with a badge and to-dos instead.

## Core data model
- **System:**
  - `devices`, `users(username, pw_hash, role, modules_json, active)`, `audit_log`, `todos(title, notes, due, assignee, done, ref)`, `conflicts`, `settings`.
- **Master data:**
  - `clients(code, name, contact, tin, address, terms_days, margin_tier_pct, credit_limit_c)`
  - `suppliers(code, name, country, currency)`
  - `locations`
- **Items:**
  - `items(code, name, unit, category, supplier_id, tracks_expiry, reorder_lvl, default_margin_pct)`
  - `item_costs(item_id, supplier_price_minor, currency, fx_rate, freight_c, duty_pct, duty_c, brokerage_c)` + `item_cost_charges(label, amount_c)`
  - `client_item_rates(id=v5(client,item), price_c)`
- **Business development and booking:**
  - `bd_requests(no, client_id|prospect_name, status, follow_up)` + `bd_lines`
  - `bookings(no, client_id, bd_request_id, status)` + `booking_lines(item, qty, price_c)`
- **Delivery:**
  - `delivery_receipts(no, booking_id, from_location_id, status, requested_by, delivered_at, manual_ref)` + `dr_lines(booking_line_id, item, expiry, qty, price_c)`
- **Stock:**
  - `stock_receipts`, `stock_adjustments` and `transfers`, each with line tables.
  - `v_stock` = receipts + PO receipts − delivered DR lines ± adjustments/transfers, grouped by item, location and expiry. It only counts rows whose parent is in the right status and not deleted.
- **Collections:**
  - `checks_received(client, bank, check_no, check_date, amount_c, status)`
  - `payments(client, method, check_id, amount_c)` + `payment_allocations(payment_id, dr_id, amount_c)`
  - `v_receivables` = delivered DR totals − allocations, excluding bounced checks.
- **Purchasing:**
  - `purchase_orders(no, supplier, currency, fx_rate, expected_at, status)` + `po_lines(item, qty, unit_cost_minor)`
  - `supplier_payments(po_id, paid_at, currency, amount_minor, fx_rate, tt_ref)`
  - `po_receipts(no, po_id, location_id)` + `po_receipt_lines(po_line_id, item, expiry, qty)`

## Progress
_Update this section when each milestone is finished: set its status, the date, and any notes that affect later milestones._

| Milestone | Status | Date | Notes |
|---|---|---|---|
| M0 Setup | ✅ Done | 2026-10-21 | See M0 notes below. Commit `c426393` on main (not pushed). |
| M1 Sync core | ✅ Done | 2026-10-21 | See M1 notes below. Commit `1791128` on main (not pushed). |
| M2 Numbering | ✅ Done | 2026-10-21 | See M2 notes below. Commit `309c39c` on main (not pushed). |
| M3 Networking | ✅ Done | 2026-10-21 | See M3 notes below. Commit `8f4cf85` on main (not pushed). |
| M4 App shell + pilot | ✅ Done | 2026-10-22 | See M4 notes below. Pilot on 2 real PCs over Tailscale passed (user report, 2026-10-22). Commit `c539139` on main (not pushed). |
| M5 Data entry | ✅ Done | 2026-10-22 | See M5 notes below. Commit `30ab5ae` on main (not pushed). |
| M6 Inventory | ✅ Done | 2026-10-22 | See M6 notes below. Commit `6ffde3a` on main (not pushed). Schema v5: every PC must update together. |
| M7 Sales flow | ✅ Done | 2026-10-22 | See M7 notes below. Commit `c69a44b` on main (not pushed). Schema v6: every PC must update together. |
| M8 Accounting | ✅ Done | 2026-10-22 | See M8 notes below. Commit `196c4db` on main (not pushed). Schema v7: every PC must update together. |
| M9 Orders | ✅ Done | 2026-10-23 | See M9 notes below. Commit `16239a2` on main (not pushed). Schema v8: every PC must update together. |
| M10 Finish + rollout | 🟡 Code done, rollout pending | 2026-10-23 | See M10 notes below. Dashboard, reports, to-dos, update flow built; version 1.0.0. Commit `0a1ad3c` on main (not pushed). **Rollout to the 5 PCs needs the user** (checklist in the notes). No schema change (still v8). |
| M11 Boxes + serials | ✅ Done | 2026-10-27 | See M11 notes below. Commit `f6400c2` on main (not pushed); version not bumped yet (still 1.1.0). Plan was `m11-boxes-serials.md`. Schema **v11** (0010 shipment charges committed first as `e2e29bb`): every PC must update together. |
| M12 DR or Invoice | 🟡 Code done, UI click-through pending | 2026-10-27 | See M12 notes below. Plan was `m12-dr-invoice.md`. Commit `d305311` on main (not pushed; `7956423` before it is cargo fmt only); version not bumped (still 1.1.0). Schema **v12**: every PC must update together. |
| BD removal | ✅ Code done | 2026-10-27 | See BD removal notes below. Commit `aea9cdf` on main. No schema change (still v12). |
| M13 macOS support | 🟡 **Next.** Code on branch, Mac build + test pending | 2026-10-31 | See M13 notes below. Branch `macos-support` (pushed; `f52a909`, on top of `8334eb6` version 1.2.0 and `c5b90d9` cargo fmt). **Not merged to main.** Never compiled on a Mac yet. No schema change (still v12). |

**M0 notes**
- **Toolchain:** VS Build Tools 2022 (C++) and rustc 1.99.1 (MSVC) installed via winget. Node 24, Tailscale 1.102 were already present.
- **specta-typescript 0.0.12 API changed:** the plan's `BigIntExportBehavior::Number` doesn't exist anymore. Use `Builder::dangerously_cast_bigints_to_number()` instead.
- **Tauri test binaries crash on Windows** (`STATUS_ENTRYPOINT_NOT_FOUND`, missing comctl32 manifest).
  - Don't write `#[test]` in `src-tauri`; tests go in `sanco-core`.
  - Bindings are regenerated by `cargo run -p sanco -- --export-bindings` (`npm run bindings`), and on every debug start.
- `@qwik-ui/headless` 0.7.7 builds and typechecks with Qwik 1.20. It must be in devDependencies. Its `Select` `bind:value` needs `Signal<string | string[]>`. Runtime behavior is still unchecked; confirm on the first real screens.
- **Built so far:**
  - Cargo workspace (`crates/sanco-core`, `src-tauri` package `sanco` / lib `sanco_lib`). Identifier `ph.sanco.desktop`, NSIS target only.
  - Theme tokens in `src/global.css`, app icon from `assets/app-icon.svg`.
  - Test command `greet` and the placeholder page `src/routes/index.tsx`; replace both in M4.
- **Not done yet:** Tauri plugins (single-instance, autostart, tray, …) are deferred to M4. Python is not installed, so don't use it in scripts.

**M1 notes**
- **Decision change, `_base_hlc` → `_vv` version vector.** Delta pulls send only a row's latest state, so a peer that skips intermediate versions (A→B edits twice→C) would see a `_base_hlc` mismatch and report a false conflict. Version vectors detect real concurrency exactly. CLAUDE.md and the Sync design section are updated.
- **HLC encoding** is `millis * 1000 + counter` (not a 48/16-bit split) so values stay below 2^53 and are safe as JS numbers. State lives on the writer thread; `meta.last_hlc` is saved on every commit.
- **Conflicts replicate.** A conflict row's id is `derived_id("conflict/{tbl}/{row_id}", "{hlc}/{site}")` of the lost version, and its `_hlc`/`_site` are the lost version's. So every PC that detects it writes a byte-identical row, and a resolution (a normal update) always beats a late re-detection. Conflicts on the `conflicts` table itself are not recorded.
- **Identity:** `meta.fingerprint` = `machine_id|canonical db path`. Opening on another machine or path takes a new site id + epoch automatically. In-place restores aren't detectable locally, so the restore flow (M4) must pass `OpenOptions.new_identity = true`. As a safety net, `apply_batch` rejects a known site whose `head` is below our watermark (`PeerRewound`) or whose epoch changed.
- **Dependency deviations:** no `rusqlite_migration` (a 20-line runner in `db/migrations/mod.rs` using `user_version`), no `rand`/`rand_chacha`/`proptest` (own SplitMix64 `rng::Rng64`, seedable for the sim). rusqlite is 0.40.2, uuid 1.28.
- **Built (sanco-core):**
  - `db/conn.rs` `Db`: writer thread + job channel, reader pool (`query_only`), `write`, `read`, `changes_since(since, limit)`, `apply_batch(batch)`, `watermark(site)`, `backup_to` (VACUUM INTO), `dump`. `OpenOptions { path, machine_id, clock, seed, durable, new_identity, on_change }`.
  - `repo.rs` `WriteTx`: `insert` (optional `id` for derived ids), `update` (returns false and writes nothing on a no-op), `upsert`, `delete`, `get`, `new_id`, `conn()`. Rejects unknown/meta columns and wrong types; `audit_log` is never writable, `conflicts` only updatable. `pw_hash` is redacted in audit.
  - `sync/`: `hlc`, `merge` (pure `decide`), `versions`, `protocol` (`ChangeBatch { site, epoch, schema, head, upto, more, rows }`, `ApplyReport`).
  - Migration 0001: `meta`, `row_versions`, `peer_watermarks`, and replicated `devices, users, roles, user_roles, audit_log, todos, conflicts, settings`.
- **Tests:** 12 unit + 15 in `tests/repo.rs` + `tests/sim.rs` (4 peers, 1500 steps, seeds 1–6 without and 101–103 with restores; ~9 s). The sim checks convergence, quiescence and "no edit vanishes without a conflict". A mutation run (conflict recording disabled) makes it fail, so the check works. `SANCO_SIM_SEED=n` reruns one seed.
- **For M2:** the numbering service needs `number_claims` (local-only) plus a migration 0002; add numbering ops to the sim. Claims must go through the writer thread (add a `Db` method that calls a `Writer` fn, like `apply_batch`).
- **For M3 (from M1):** the network layer just moves `ChangeBatch` JSON: `to.apply_batch(from.changes_since(to.watermark(from.site), limit))` in a loop while `more`. Poke peers on `ChangeEvent { origin: Local }`. The skew/schema/epoch/rewind checks already exist in `apply_batch`; the handshake should surface the same errors before pulling.
- **Not done:** invariant checks (`sync/invariants.rs`) wait for domain tables (M6+). The Tauri shell doesn't open the database yet (M4).

**M2 notes**
- **Built:** `sync/numbering.rs`, `time.rs` (Manila dates, no chrono/jiff yet), migration 0002 (schema v2).
  - Replicated `doc_numbers(series, period, seq, no, ref_tbl, ref_id, site, assigned_at, prev_no, confirmed)`, one row per document and series, id `number_row_id(ref_id, series)` = `derived_id(ref_id, "number/{series}")`. **Documents don't get a `no` column**: they join `doc_numbers` on `ref_id` (or call `numbering::number_of`), so the number lives in one place and renumbering touches one row.
  - Local-only `number_claims(series, period, seq, ref_id, site_id, expires_at)` (TTL 10 min) and `previous_sites(site_id, retired_at)`.
  - `period` = Manila year when the series resets yearly, else `""`. Formats: setting `number_format.{series}` (`format_key`), JSON `{prefix, year, reset_yearly, pad}`, missing fields default. Defaults: documents `DR-2026-0001`; `CL`/`SUP` codes `CL-0001` (no year, no reset). Series constants in `numbering::series` (BD, BK, DR, PAY, PO, RR, CL, SUP). The rendered `no` is fixed at assignment, so a format change only affects new numbers.
  - Settings rows use id `derived_id("setting", key)` (same as the sim). Write them with `upsert`.
- **API:**
  - `trait NumberPeer { fn claim(&NumberClaim) -> Result<ClaimReply> }`; `Db` implements it in-process (`Db::claim_number`, runs on the writer thread). `Err` = unreachable.
  - `numbering::reserve(db, peers, series, ref_id) -> Reservation`: local max + 1, claimed locally then on every peer; any refusal or higher peer max → retry at max + 1 (20 rounds max, then `NumberUnavailable`). `Reservation.confirmed()` is false if any peer was unreachable → UI shows "offline — number may change" (stored as `doc_numbers.confirmed`).
  - `numbering::assign(tx, &res, ref_tbl)` inside the document's write. `Db::new_id()` gives the document id before the write.
  - `numbering::resolve_clashes(db, peers) -> Vec<Renumbered>`: run after each applied batch. Returns what changed so the shell can show a notification.
- **Decision change, clash winner:** the earliest `(assigned_at, id)` keeps the number instead of `(hlc, site)`, because `_hlc`/`_site` change on every edit while `assigned_at` is set once per assignment (and again on renumber). Only the site in `doc_numbers.site` (or a PC that has it in `previous_sites`) renumbers a loser. It writes `prev_no`, a new `assigned_at`, and one to-do per involved user (`created_by` of both holders, derived ids). The audit log records old → new via the normal update diff.
- **Restore successor:** `Db::open` with `new_identity` records the old site in `previous_sites`, so the restored PC renumbers clashes the old site caused. A plain copy (fingerprint mismatch without `new_identity`) clears `previous_sites`. **M3: the join snapshot must open without `new_identity`**, or the new PC would take over the sender's numbers. Known gap: restoring the same backup twice orphans the middle site's numbers (no PC owns them); the sim tolerates only that case. Add an admin manual renumber later if it ever happens.
- **Tests:** 6 in `tests/numbering.rs`. The sim now creates numbered docs (DR + CL) through partition-aware `Link` peers, resolves clashes mid-run and at settle, and checks one holder per number, unique formatted numbers, and no conflicts on `doc_numbers`. 5–17 renumbers per seed; ~16 s total. A mutation (ignore `previous_sites`) makes the restore sim fail.
- **For M3:** expose `POST /numbers/claim` (body `NumberClaim`, reply `ClaimReply`) and implement `NumberPeer` over HTTP with a short timeout (~2 s), peers taken from `devices`. Call `resolve_clashes` after a pull that accepted `doc_numbers` rows.
- **For M7 (open question):** "DR number = BIR booklet number" may mean staff type the booklet number instead of auto-numbering. If so, add an exact-number claim (`reserve_exact`) and **don't auto-renumber** manual numbers; flag the clash as a to-do instead. Ask the user before building DRs.

**M3 notes**
- **Deviation, new crate `crates/sanco-net`** instead of `src-tauri/src/net/`, so the network layer is testable (Tauri test binaries crash). Deps: axum 0.8.9, reqwest 0.13.5 (`default-features = false`, `json` only: plain HTTP, no TLS crates), tokio 1.54, hkdf 0.13 + sha2 0.11, subtle. reqwest 0.13 makes `query` optional; URLs are built by hand.
- **Core additions (`sync/peer.rs`, migration 0003, schema v3):** local-only `workspace(workspace_id, name, sync_key hex, created_at)`; `Db::{workspace, create_workspace, head, hello, check_peer, snapshot, finish_join, devices, register_device}`. `Hello { site, epoch, schema, app_version, name, now_ms, head }`. `check_peer` = `apply_batch`'s checks plus clocks **behind** by > 2 min (`ClockSkew.ahead_ms` is negative then). `Db` now implements `Debug`; `Db::call` is `pub(crate)`.
- **Endpoints** (`server.rs`): `GET /hello`, `GET /changes?since&limit` (≤ 2000), `POST /poke {port}` (peer address = connection IP + that port), `POST /numbers/claim`, `GET /snapshot` (VACUUM INTO a temp file on the writer thread, info in the `x-sanco-snapshot` header). Guard: Tailscale ranges **or loopback** (needed for several instances on one PC), then constant-time bearer check. Token = hex(HKDF-SHA256(key, salt `sanco-sync`, info `bearer-token/v1`)).
- **Node** (`node.rs`): `Node::start(Arc<Db>, LocalChanges, NodeConfig, EventFn)`. Pull loop every 30 s (first tick at once), poke loop (300 ms debounce) woken by local commits **and by pulls that accepted rows** (so relays don't wait for the timer; terminates because echoes accept 0 rows). One pull at a time per peer. `PeerState` Unknown/Online/Offline/Blocked (blocked = wrong key, schema, clock, epoch, rewound). `resolve_clashes` runs after a pull that accepted `doc_numbers` rows; `NodeEvent::Renumbered` reaches the shell. `Node::reserve_number` claims over HTTP (2 s timeout); peers known offline/blocked fail at once so an offline laptop doesn't add 2 s per round.
- **Peers** = `devices` rows (id = site id, host = Tailscale IPv4 from `tailscale ip -4`, falling back to `C:\Program Files\Tailscale\tailscale.exe`) + addresses that poked us. A poked address is dropped once its device row arrives or a pull from it fails. Device row is rewritten only when a field changes or hourly (`DEVICE_REFRESH_MS`), checked every 10 min; rows of `previous_sites` are deleted on register. **Deviation:** IP instead of MagicDNS name (no DNS dependency).
- **Join:** `JoinCode` = Crockford base32 (dash groups, forgiving of case/O/I/L) of `[v1][32-byte key][host:port][4-byte SHA-256 checksum]`. `sanco_net::join(code, opts, timeout)`: hello → schema + clock check → snapshot to `sanco.joining` → rename → `Db::open` (a copy: new site, `previous_sites` cleared) → `finish_join` (watermark for the sender = snapshot head, drop copied `number_claims`).
- **Shell:** flags `--data-dir --port --host --name` (`state.rs::Args`); default data dir = Tauri app data dir, port 7470, name = `COMPUTERNAME`, machine id = registry `MachineGuid`. Startup resumes sync if `sanco.db` has a workspace. Commands: `workspace_status, create_workspace, join_workspace, invite_code, sync_status, sync_now` and **temporary** `dev_notes, dev_add_note` (to-dos, `Actor::system()`, no auth). Events: `db-changed` (tables), `sync-status`, `renumbered`. `tracing` has its `log` feature on, so core/net logs reach tauri-plugin-log, now enabled in release too (Info). Join moves a workspace-less `sanco.db` aside (`sanco.unused-<ts>.db`), never deletes it.
- **M3 test page** `src/routes/index.tsx` (create/join, join code, peers table, notes). Replace in M4. Built and type-checked, **not clicked through**: the backend was verified by running the real app on `.dev/a` and joining a headless node through it (`cargo run -p sanco-net --example node`): snapshot served, B's rows pulled.
- **Tests:** 5 in `crates/sanco-core/tests/peer.rs`; 5 unit + 5 in `crates/sanco-net/tests/net.rs` (3 PCs on loopback: join + poke sync, wrong key/typo code refused, offline edits → one conflict on both, skewed clock blocked then recovered, number claims confirmed/unconfirmed and offline peer not waited for). ~5 s; stable over 8 runs. Windows takes ~2 s to refuse a closed loopback port, which shows up as "timed out".
- **For M4:**
  - Pilot over Tailscale needs the firewall rule (NSIS hook) and Shields Up off. Windows will prompt on the first bind of `0.0.0.0` in dev builds.
  - Replace `dev_*` commands and the test page; build Settings → Devices/Sync from `sync_status` + `devices`. Show `addr == None` ("Tailscale not running") and blocked peers' errors.
  - The join code is the whole workspace secret (it can download everything, password hashes included): show it only to admins.
  - Removing a device = deleting its `devices` row; that PC's `register_device` then fails with `Deleted` and it should stop syncing (not done yet). Until removed, an offline device makes every number "unconfirmed".
  - If the node fails to start (port in use), the app shows the error but has no retry; add one.
  - Known risk: `tailscale serve/funnel` proxying the port would arrive from loopback and skip the IP check (the token still applies). Don't proxy the sync port.
  - Not done: key rotation, rate limiting, peer-level `last_seen` beyond the hourly device refresh.

**M4 notes**
- **Core (`sanco-core`):** `domain/{auth, users, roles, conflicts, audit}.rs`, `backup.rs`, optional feature `specta` (derives `specta::Type` on DTOs, enabled by `src-tauri`, so types aren't duplicated). No migration: `users`, `roles`, `user_roles` already existed (schema stays v3).
  - **RBAC:** `roles.perms_json` = `Perms { admin, modules: {module: [actions]}, fields: [costs|margins] }`. Modules: dashboard, bd, bookings, accounting, items, inventory, orders, clients, reports, settings. A user's perms = union of their roles. `admin` = everything incl. future modules, and is required for users, roles, join code, device removal and restore. Settings view/edit covers devices, conflicts, backups, audit log, autostart. Seed roles Admin/Staff use derived ids `derived_id("role", "admin"|"staff")`, seeded with the first admin. Staff = all actions on all modules except Settings, no sensitive fields.
  - `users::ensure_admin_remains` refuses any change leaving no active user with an admin role (checked inside the write; the failed tx rolls back). Users can't deactivate themselves. Users are never deleted. `user_roles` rows use v7 ids and are deleted on removal (duplicates from offline edits are cleaned on the next save).
  - **Passwords:** argon2 0.6 (Argon2id defaults), min 6 chars, hashed *before* `Db::write` (not on the writer thread). `[profile.dev.package.argon2/blake2] opt-level = 3`, otherwise sign-in takes seconds in debug. Usernames lowercase, `[a-z0-9._-]{2,32}`. Login picks the matching hash among same-name rows (duplicates possible after offline merges).
  - `WriteTx::log_event(action, tbl, row_id, details)` writes an audit row without touching a data row (sign-in/out), so it can never conflict.
  - **Conflicts:** `conflicts::list` shows a per-column diff (lost vs current; `pw_hash` hidden); `resolve(restore)` writes the lost values back as a normal edit, then sets `resolved/resolved_by`. Restoring onto a deleted row is refused.
  - **Backups:** `<data dir>/backups/`: `sanco-daily-YYYY-MM-DD.db` (Manila day, newest 14 kept), `sanco-manual-…`, `sanco-before-restore-…`, plus the migration runner's `pre-migration-…`. Written to `.partial` then renamed. Restore = `check_restorable` (same `workspace_id`, schema ≤ ours) → `VACUUM INTO` a before-restore copy → stop node, wait for the last `Arc<Db>` → `replace_db` → `Db::open` with `new_identity` → start node. Tested in `tests/backup.rs` (new site, peers accept it, synced rows come back).
  - `Db::{is_removed, remove_device}`; `time::{manila_day_start_ms, manila_stamp}`; error variants `NotSignedIn, BadLogin, Forbidden, Invalid` (Invalid = user-facing message).
- **Net:** `NodeEvent::Removed`, emitted when the device refresh hits `Deleted` or a pull accepts rows and our `devices` row is deleted. The shell stops the node, `workspace_status.removed` shows a screen, and `leave_workspace` sets the DB aside (`sanco.removed-<ts>.db`) so the PC can join again.
- **Shell:** session = signed-in user id in `AppState` (one window per instance; sign in again after each app start). Every command calls `commands::{signed_in, require(module, action), require_admin}`, which re-reads the user and perms from the DB, so deactivation/role edits on another PC apply at once. `session-changed` event on sign-in/out. Commands: auth (`session_info, login, logout, create_first_admin, change_password, perm_catalog`), users/roles, settings (`list_devices, remove_device, list_conflicts, resolve_conflict, list_backups, backup_now, restore_backup, audit_log, audit_tables, pc_settings, set_autostart`), sync (+ `retry_sync, leave_workspace`; `create_workspace(name, admin)` creates the first admin and signs in; `invite_code` admin-only). `dev_*` commands and the M3 test page are gone.
  - `workspace_status.starting` stays true until the startup `resume` finished, so the UI shows a splash instead of flashing the setup screen. `needs_admin` covers a workspace created without users (fallback screen).
  - **Plugins:** single-instance (skipped with `--data-dir`, so test instances still work), autostart (`--minimized`; enabled automatically on create/join in release builds only, toggle in Settings → This PC), window-state (all flags except VISIBLE), notification (renumbered documents), tray (`tray.rs`: left click opens, menu Open/Quit). The main window starts hidden (`visible: false`) and `setup` shows it unless `--minimized`. Closing hides to the tray. Daily backup checked every 10 min.
  - **Installer:** `bundle.windows.nsis.installMode = perMachine`, hooks in `src-tauri/windows/hooks.nsh`: POSTINSTALL deletes then adds firewall rule "Sanco sync" (inbound TCP 7470, remote `100.64.0.0/10,fd7a:115c:a1e0::/48`, program = `$INSTDIR\${MAINBINARYNAME}.exe`); PREUNINSTALL deletes it. A rule only covers port 7470: a PC started with another `--port` needs its own rule.
- **Frontend:** `routes/layout.tsx` is the gate (splash → removed → setup → first admin → sign-in → shell). `SessionCtx` (`lib/session.ts`) holds status, user and `syncTick`. Sidebar filters modules by `view` permission and shows a sync summary. Settings tabs: This PC, Devices & sync, Users*, Roles*, Conflicts, Backups, Audit log (*admin). Placeholder pages for the 8 business modules (`components/shell/placeholder.tsx`). Shared: `components/ui/{styles.ts, ui.tsx}` (class sets, `PageHeader, Badge, Alert, Empty, Modal, NoAccess`), `components/filter-bar` (`FilterBar` + `filtersOf(url, keys)`, `allLabel` per select), `lib/{events (useLiveLoad), format, perms, result (unwrap/errorText), nav}`.
  - **Qwik gotchas found:** (1) The SSG pass renders route content in a hidden `q:template` *before anyone is signed in*; loaders must read perms from the session store when they run, never capture them at render. (2) Visible tasks with the default `intersection-observer` strategy don't fire for content projected later or inside a closed `<dialog>`: data loaders and `Modal` use `{ strategy: "document-ready" }`. `useLiveLoad(tables, fn)` handles both (tracks `session.user?.id`). (3) A command parameter named `new` breaks the generated JS; avoid reserved words. (4) `input` class has `w-full`; use `inputBase` when setting a width.
- **Verified:** core tests (7 in `tests/auth.rs`, 3 in `tests/backup.rs`, unit tests for perms/passwords/dates); clippy clean; `npm run build` clean. Clicked through on two local instances driven over CDP (WebView2 `--remote-debugging-port`, separate `WEBVIEW2_USER_DATA_FOLDER` each): create workspace + admin, staff user, join from B, staff sees no Settings and Rust refuses admin/settings commands, a real conflict from simultaneous edits recorded identically on both PCs and restored from the UI, audit log, roles matrix, backup + restore on B (new site id, resynced), remove B from A → B shows "removed" → leave → rejoin as a new device, password change. `npx tauri build` produces `target/release/bundle/nsis/Sanco_0.1.0_x64-setup.exe` (4.5 MB); the generated `installer.nsi` includes the hooks with `MAINBINARYNAME = sanco`. Tray, close-to-tray, autostart and the firewall rule were **not** exercised (they need the installed build).
- **Pilot (2026-10-22):** the user installed the NSIS build on two PCs and reported that everything works. Gotcha seen: PC 1 was first signed in to the wrong tailnet; fix = switch tailnet, quit Sanco from the tray and restart (the IP is re-read at start and every 10 min), and make a new join code (it embeds the old IP). PC 1's Sanco data was wiped and set up again.
- **Questions for M6/M9 (asked 2026-10-22).** Answers (2026-10-22): (1) **Store and Office are the same location** — keep the seeded Office + Warehouse; "Warehouse → Store" means Warehouse → Office. (2) **Transfers are instant**: one transfer document moves stock out of one location and into another at the same moment (no in-transit state). (3) **No location on the PO**: the location is chosen only when goods are received (each receipt picks its own; partial receipts can go to different locations). Original questions: (1) Is "Store" a separate location from "Office", or should the seeded locations be Warehouse + Store? (2) Transfers: instant, or sent → in transit → received? (3) Should a PO carry an intended receiving location that pre-fills receiving (recommended), or is the location chosen only when receiving? Confirmed: stock can move Warehouse → Store via transfers (M6), and one PO can be received in parts into different locations.
- **Disk:** C: was full during M4 (0 bytes free; `target/` was 15 GB). Deleted `target/debug/incremental` (4 GB); use `CARGO_INCREMENTAL=0` or `cargo clean` if it fills again.
- **Pilot checklist (2 real PCs over Tailscale, needs the user):** install the NSIS build on both (per-machine, admin prompt); check `netsh advfirewall firewall show rule name="Sanco sync"`; Tailscale Shields Up off; PC 1 creates the workspace, PC 2 joins with the code; edit on one, see it on the other within seconds; restart Tailscale on one and confirm sync resumes; close the window (tray keeps syncing); reboot and confirm autostart brings it back in the tray; quit from the tray on one PC and confirm the other shows it offline.
- **For M5+:** every new command starts with `require(&state, Module::X, Action::Y)`; hide cost/margin values unless `user.perms.sees(Field::Costs|Margins)` (check in Rust: strip the values from the DTO, don't just hide in the UI). List screens use `FilterBar` + `filtersOf` and `useLiveLoad([tables], load)`. Not done: idle auto-lock, login rate limiting beyond a 500 ms delay, audit retention (audit rows grow forever; ~1 row per change), sync-key rotation after removing a lost PC.

**M5 notes**
- **Migration 0004 (schema v4):** replicated `clients, suppliers, locations, charge_types, items, item_costs, item_cost_charges, client_item_rates`. Dependency: `rust_decimal 1.43` (`default-features = false, features = ["std"]`) for FX.
- **Units decision:** percentages are integer **basis points** in `*_bp` columns (20% = 2000): `clients.margin_tier_bp`, `items.default_margin_bp`, pct charge values, setting `vat_rate_bp` (default 1200). Rounding is half away from zero everywhere (`pricing::div_round`). Added to CLAUDE.md.
- **Data-model deviations from the plan's sketch:**
  - Client and supplier **codes are not columns**: they're `doc_numbers` rows (series `CL`/`SUP`), joined on `ref_id`, reserved via `Node::reserve_number` (shell helper `commands::sync::reserve_number`) before the write. Item codes are typed (`items.code`), refused if a live item has the same code (NOCASE); offline duplicates show `duplicate_code`. Clients/suppliers show `duplicate_name`.
  - Cost sheet: `item_costs` id `derived_id(item, "cost")` (currency, `supplier_price_minor`, `fx_rate` text, notes) + `item_cost_charges` id `derived_id(item, "charge/{type}")` (`mode` per_unit|pct, `value` = centavos or bp of the **PHP base cost** = supplier price × FX). Cleared line = value NULL. PHP sheets ignore FX. A sheet shows every active charge type plus inactive ones that have a value.
  - `client_item_rates` id `derived_id(client, item)`: `price_c` is **net** (before VAT) + `vat` flag; cleared = `price_c` NULL. Gross is computed at the current VAT rate when read.
  - Suppliers: `country` TW|CN|OTHER, `currency` PHP|USD|CNY|TWD (`pricing::CURRENCIES`).
- **Seeds:** Office/Warehouse (`derived_id("location", key)`) and the 5 charge types (`derived_id("charge_type", key)`) via `domain::seed_defaults`, run in `create_first_admin` **and on every node start** (for pre-M5 workspaces). A seed row that exists, even deleted, is skipped; identical seeds from two PCs merge without a conflict (tested).
- **Price resolution** `pricing::quote(conn, client, item)` → `PriceQuote { rule, net_c, vat (rate's flag or None), vat_amounts, cost_c, margin_bp, below_cost }`. `below_cost` is shown to everyone (only a yes/no); `cost_c`/`margin_bp` are stripped per field. M7 bookings should call `quote` and store the resolved price + rule on the line.
- **Permissions:** DTOs have `strip(&perms)`; inputs carry margins but `save` ignores them when the user can't see margins (`margins: bool` param), so a staff edit never wipes a tier/default margin (verified in UI). Cost sheets need `Field::Costs` to open or save (`get_cost_sheet` refuses). Shell helpers `require_save(module, is_new)` (create vs edit) and `require_view_any(modules)`. Pickers (`client_options`, `item_options`, `supplier_options`, `list_locations`, `list_charge_types`, `pricing_settings`, `company_info`) only need sign-in. Locations live under Inventory perms, suppliers under Orders, rates under Items (rates list also readable with Clients view).
- **Deletes:** client → also its rates; item → its cost sheet, charge lines and rates; supplier refused while items use it; charge type refused while a sheet uses it (deactivate instead); location delete unrestricted until M6 adds stock (**M6: block deleting a location with stock or movements**). Codes of deleted rows are never reused.
- **Number formats** (`settings::set_number_format`): prefix letters/digits ≤ 12 (uppercased), pad 1–8, yearly reset requires the year shown, and the yearly reset **can't change once a series has numbers** (it would restart at 1 and could repeat).
- **Frontend:** Clients (list + `?id=` detail with rates), Items (`/items/` catalog + `?id=` detail with cost sheet and rates, `/items/rates/`), Order → Suppliers (`/orders/suppliers/`), Inventory → Locations (`/inventory/locations/`), Settings → Company, Pricing (VAT + charge types), Number formats. Shared: `components/ui/tabs.tsx` (`TabNav`, settings layout uses it), `Field`, `InfoList`, `Confirm` in `ui.tsx`, `components/rates/rates.tsx` (`RatesTable`, `RateModal`, `QuoteSummary`), `components/items/cost-sheet.tsx` (live preview, "changed elsewhere" warning while editing). `lib/format.ts`: `formatPct, formatMinor, formatPesoOr, parseMoney, parsePct, parseCount, moneyText, pctText`. Detail views remount per id with `key={id}`. Qwik gotcha: input typings lack `list` (datalist): spread `{...datalist(id)}`. Grids with tables need `minmax(0,1fr)` columns or they overflow.
- **Verified:** 7 tests in `tests/master_data.rs` + 5 unit tests in `pricing.rs` (rounding, margin on price, VAT, FX parsing, landed cost); full core/net suite incl. sim passes; clippy clean; `npm run build` clean. Clicked through on 2 local instances over CDP: workspace → seeds shown, supplier SUP-0001, item + cost sheet (US$10 × 56.50 + ₱20 + 10% = ₱641.50, base ₱801.88 at 20%), client CL-0001 at 30% tier (quote ₱916.43), below-cost rate flagged, join from B, staff on B sees no costs/margins (and Rust refuses `get_cost_sheet`), staff edit synced to A with the margin kept, client created on B got CL-0002.
- Not done: CSV import (deferred per user answer 5), admin merge of duplicate items (flag only), price history.

**M6 notes**
- **Migration 0005 (schema v5):** replicated `stock_docs(kind receipt|adjustment|transfer, date, location_id, to_location_id, reason, notes, status posted|void, voided_at, voided_by, void_reason)` + `stock_doc_lines(doc_id, line_no, item_id, expiry, qty)`; views `v_stock_moves` (one row per line and location; a transfer gives −qty at `location_id` and +qty at `to_location_id`) and `v_stock` (SUM by item, location, expiry; can be zero/negative).
- **Deviation from the data-model sketch:** one header/line table pair with a `kind` column instead of separate `stock_receipts`/`stock_adjustments`/`transfers` tables.
- **Documents are immutable:** posted on save, never edited. A mistake is **voided** (status → void, needs a reason; Inventory *delete* permission) and entered again. So two PCs never edit the same lines concurrently. Voided docs stay listed.
- **Numbers:** new series `SR` (stock receipt), `ADJ`, `TR` in `numbering::series` and Settings → Number formats (`settings::SERIES` now 11).
- **Lots** = (item, location, expiry). Incoming lines (receipts, positive adjustments) need an expiry exactly when the item tracks expiry (dropped otherwise); outgoing lines name the lot as received. Adjustment qty is signed; others positive. Date ≤ today (Manila).
- **No negative stock locally:** `inventory::validate` / `create` / `void` refuse anything taking a lot below zero (message names item, lot, location, on hand, needed); voiding a receipt whose stock moved on is refused. Offline PCs can still both take the same stock → negative lot after sync, shown as a "negative" badge + warning on Stock; further reductions are refused until an adjustment fixes it. Invariant to-dos for this wait for the Dashboard (M10).
- **Gotcha — validate before reserving a number.** A failed write after `reserve_number` leaves the claim held (10-min TTL, also on peers), so the next document skips a number. `post_stock_doc` runs `inventory::validate` (read-only) first. **M7/M8/M9 must do the same** (clients/suppliers still reserve first; minor).
- Location delete refused if any stock doc (even void) uses it; item delete refused if any stock line uses it (mark inactive instead).
- Views aren't replicated, so **later migrations may DROP and recreate `v_stock_moves`/`v_stock`** (M7: delivered DR lines out; M9: PO receipt lines in). Keep the column list `(doc_id, kind, date, line_id, item_id, location_id, expiry, qty)`; `StockKind` must then grow (`item_moves` skips unknown kinds).
- `time::{parse_date, add_days, days_between}`; `inventory::NEAR_EXPIRY_DAYS = 90` (frontend `NEAR_EXPIRY_DAYS` in `components/inventory/expiry-badge.tsx`); M10 Dashboard can use `inventory::{low_stock, expiring}`.
- **Commands:** `stock_overview, stock_lots, expiring_stock, item_stock_moves, list_stock_docs, get_stock_doc, post_stock_doc, void_stock_doc` (all Inventory perms).
- **Frontend:** Inventory tabs Stock (`/inventory/`, per-item × location table, `?item=` detail with lots + movement history), Movements (`/inventory/movements/`, `?id=` detail + void), Expiring, Low stock, Locations. Shared `components/inventory/stock-doc-form.tsx` (`StockDocModal`, `blankDoc`, lot picker with on-hand → after preview). `format.ts`: `formatDay, todayManila, formatQty, parseSigned`.
- **Verified:** 6 tests in `tests/inventory.rs` (moves, void, negative refusal + validate, validation, delete blocks, low/expiry lists, two offline PCs oversell → same negative lot on both, no conflicts) + `time` unit test; full core/net suite incl. sim; clippy clean; `npm run build` clean. Clicked through on one instance over CDP: multi-line receipt into Warehouse, refused then valid transfer (no number gap after the fix), adjustment with an existing lot and a new-expiry lot, stock/item/expiry/low/movements pages, void refused (stock moved on) and void of a transfer.
- `cargo fmt` was run on sanco-core during M6 and reformatted some untouched files (formatting only).
- Not done: stock valuation (needs Costs perm; maybe Reports), stock-count sheet UI, configurable near-expiry window.

**M7 notes**
- **Migration 0006 (schema v6):** replicated `bd_requests` + `bd_lines`, `bookings` + `booking_lines`, `delivery_receipts` + `dr_lines`; `ALTER TABLE doc_numbers ADD COLUMN manual` (registry col appended last); `v_stock_moves`/`v_stock` dropped and recreated with delivered DR lines (kind `delivery`, `doc_id` = DR id, date = `delivered_date`, −qty at `from_location_id`). `StockKind::Delivery` added; `post_stock_doc` refuses it; `item_moves` LEFT JOINs `stock_docs`/`delivery_receipts`.
- **Domain** (`domain/{bd, bookings, delivery}.rs`, helpers in `domain/mod.rs`: `MAX_LINES`, `require_date`, `date_opt`, `replace_lines` (diff lines by id: update/insert/delete), `line_ids`, `line_summary`, `line_amount`):
  - **BD:** client *or* prospect name; lines = catalog item or free-text description, qty, optional quoted net price; status open|quoted|lost via edit, **won only by `convert`** (needs a client; free-text lines are left out; quoted prices carry over with rule `quoted`; sets the request's client). Won requests can't be edited/deleted. Link to the booking = `bookings.bd_request_id` (not stored on the request).
  - **Booking:** draft → confirmed (Bookings *approve*) → closed, or cancelled; back_to_draft only without DRs (requested/approved/delivered); close only without open DRs; cancel only without DRs; delete only drafts. Lines editable only in draft. Line price: blank → `pricing::quote`; typed equal to the quote → quote's rule; unchanged from saved → keeps its rule; else `custom` (`PriceSource`: manual | client_tier | item_default | quoted | custom). `vat_bp` snapshot stored on save. Progress (pending/delivered/remaining per line, none/partial/full) computed from DRs.
  - **DR:** requested (from a confirmed booking, one location, lot per line, Bookings *or* Accounting *create*) → approved (Accounting *approve*) → delivered (Accounting *edit*; date ≤ today, received_by) ; cancel open ones (Accounting *edit*); **void** delivered ones = status cancelled with `delivered_at` kept (`DrInfo.voided`, Accounting *delete*, reason required) → stock comes back. DR lines are **immutable** (cancel + re-request). Request refuses qty above what's left on the booking line and lots without stock; delivery re-checks stock (`inventory::check_not_negative`, now `pub(crate)`). Price copied from the booking line; VAT flag from the booking; `vat_bp` set at request and refreshed at delivery (receivables in M8 should use DR totals as stored).
- **DR numbers (user answer 10 implemented):** setting `number_mode.DR` = manual (default) | auto (`numbering::{is_manual, mode_key, MANUAL_SERIES}`, `settings::set_number_mode`, Settings → Number formats has the switch; other series refuse manual). Manual: `numbering::reserve_exact` (digits only ≤ 12, leading zeros kept in `no`, `seq` = value, period `""`) claims with peers like `reserve`, refused if any PC has it; `Reservation.manual` (serde default false) → `doc_numbers.manual = 1`. Typed any time until delivery, **required to deliver**; `set_dr_number` corrects it (`numbering::assign_or_change`, old text in `prev_no`). Auto: number given at approval (`reserve`) if none. **`resolve_clashes` never renumbers manual rows**; the PC owning the later one writes to-dos ("Number X is used twice", derived ids, shared `notify_both` helper with the renumber to-dos — ids unchanged) and `DrInfo.no_clash` shows a badge until someone corrects one.
- Shell: `commands/{bd, bookings, delivery}.rs`, `Node::reserve_exact` + `commands::sync::reserve_exact_number`, `config::set_number_mode`. `dr_lots(booking_id)` gives lots of a booking's items without Inventory perms. Booking detail strips line `cost_c`/`margin_bp` per field (verified as staff).
- Delete guards: clients with BD/bookings/DRs, items on BD/booking/DR lines, locations used by DRs are refused (mark inactive).
- **Frontend:** `/bd/` (list + `?id=` detail, form with item-or-free-text lines and price prefill from the client's quote, convert modal), `/bookings/` (list with "still to deliver" filter, detail with progress + DRs, form with live quotes/rule labels/below-cost and totals, status actions), `/accounting/` (layout with a "Delivery receipts" tab; open to Accounting **or** Bookings viewers; DR list with status/no-number/clash filters, detail with approve/deliver/number/cancel/void modals), `components/sales/{sales.tsx (status maps, `DrNo`, `Totals`, badges), dr-request.tsx (FEFO lot default, "+ lot" split, shortage hint)}`. Client detail lists its bookings and DRs. Inventory item history links deliveries to the DR. Stock screens reload on `delivery_receipts` changes.
- **Qwik gotchas found:** (1) a module-level `let` incremented inside a `$()`/task segment fails the build ("Illegal reassignment of import"): increment it inside a module-level function (`newKey()`). (2) Local functions can't be captured by `$()`: make them module-level helpers. (3) `q:slot` on children of a `<>` fragment isn't projected: put `q:slot` on each element.
- **Verified:** 4 tests in `tests/sales.rs` (BD → booking → DR flow with lots, numbers, stock, void, guards; price rules kept on re-save; auto DR numbers; two offline PCs typing the same DR number → no renumber, both flagged, to-dos for both users, fixed by correcting one); full core/net suite incl. sim; clippy (`--all-targets`) clean; `npm run build` clean. Clicked through on one instance over CDP: BD with prospect + free-text line → convert (picks client, warns about skipped line) → edit draft (blank price → ₱125 item default) → confirm → DR request split over two lots → approve without number → deliver refused without number → delivered with typed number → correct number (bad text refused) → void (stock back) → Settings DR mode switch; staff sees no costs/margins on bookings.
- **For M8:** receivables = delivered DRs (`status = 'delivered'`) totals as stored (`vat_bp` on the DR) minus allocations; **block `delivery::void` when payments are allocated to the DR**; add Accounting tabs to `routes/accounting/layout.tsx`. Client detail has a placeholder line for balance/payments/checks. Credit-limit warnings on bookings aren't done (needs receivables). Dashboard KPIs (M10) can use `bookings::list` (confirmed, progress ≠ full), `delivery::list` (open DRs, no number, clash), BD follow-ups due.
- Not done: printing (none planned), booking line notes in the form (kept on save, not editable), auto-close of fully delivered bookings (shown as "Delivered"; user closes).
- **Decision change (user, 2026-10-22): no edit locks.** Supersedes the "lines editable only in draft", "won requests can't be edited" and "DR lines immutable" points above. Now: BD requests editable in any status (a won one may stay won; delete of a won one is still refused); bookings editable in any status but cancelled (removing a line that is on a non-cancelled DR is refused, "back to draft" now only un-confirms); DRs editable until cancelled via `delivery::update` / `update_dr` (Accounting *or* Booking *edit*): date, location, address, notes, lines (add/remove/qty/lot, `DrLineInput.id` keeps a line), and for delivered DRs the delivery date and receiver. Edit checks: qty ≤ booking line left + the DR's own qty; stock checked on the net difference (a delivered DR's old lines are added back first). Kept DR lines keep their price; new lines take the booking line's. `get_booking` is readable with Accounting view too (the DR edit modal loads it). The `DrRequestModal` (`components/sales/dr-request.tsx`) does both request and edit (`bookingId`, optional `dr`). Test `dr_lines_stay_editable_even_after_delivery`; clicked through editing a delivered DR (stock moved by the difference).

**M8 notes**
- **User answers (2026-10-22):** (1) payments get **automatic** `PAY-2026-0001` numbers plus an optional free-text `or_no` (OR/CR booklet number); (2) aging counts days past the **due date** = delivery date + the client's *current* `terms_days` (buckets Not due, 1–30, 31–60, 61–90, Over 90); (3) a check (PDC too) **counts as paid from receipt** until it bounces or is replaced.
- **Migration 0007 (schema v7):** replicated `payments` + `payment_allocations`; views `v_dr_totals` (net/VAT/gross per live DR from its lines, VAT rounded half away from zero in SQL, equal to `pricing::with_vat`; tested), `v_payment_counts`, `v_allocations` (counting payment × delivered DR × non-null amount), `v_receivables` (delivered DRs: gross, paid, balance).
- **Deviation from the data-model sketch:** no separate `checks_received` table. A check is a payment with `method = 'check'` and `check_bank/check_no/check_date/check_status (on_hand|deposited|cleared|bounced)`, `deposited_date, deposit_account, cleared_date, bounced_date, bounce_reason`. **"Replaced" is not stored**: a check is replaced while a posted payment names it in `replaces_id` (so voiding the replacement brings the old check back). A payment counts unless void, or a bounced/replaced check.
- **Allocations** id `derived_id(payment, "allocation/{dr}")`, cleared with `amount_c = NULL` (never deleted). Only delivered DRs of the same client; total ≤ payment amount; per DR ≤ gross − other counting payments (this payment and the check it replaces excluded). What a counting payment doesn't apply (or a DR overpaid) is the client's **credit**; balance = outstanding − credit.
- **Domain:** `domain/payments.rs` (`validate/create/update/void`, `check_step` Deposit|Clear|Bounce|Undo, `list/get/open_drs/for_dr`), `domain/receivables.rs` (`report`, `client_balance`, `client_drs`, `dr_paid`, `paid_by_dr`, `due_date`, `AgingBucket`). Payments stay editable (amount, details, allocations; not method or client; replaced check fixed at creation); a wrong one is voided (Accounting *delete*, reason). Check rules: deposit only from on hand/bounced (redeposit) and not before the check date; clear from deposited; bounce from deposited/cleared with a reason; undo one step; nothing on a replaced or void check. Dates ≤ today.
- **DR integration:** `DrInfo.{paid_c, balance_c, due_date}`, `DrDetail.payments`. **`delivery::void` is refused while counting payments are applied**, and editing a delivered DR can't take its gross below what's paid. Client delete refused when it has payments. `domain::peso()` formats ₱ for messages.
- **Offline:** two PCs paying the same DR offline → DR balance negative ("overpaid" badge, counts as credit), no conflict (tested). An allocation to a DR voided on another PC drops out of `applied` and returns as credit.
- **Commands** (`commands/accounting.rs`): `list_payments, get_payment, payment_open_drs, save_payment` (validate → reserve PAY → create; edit needs Accounting *edit*), `void_payment, check_step, receivables_report, client_balance` (Accounting/Bookings/Clients view), `client_receivables`.
- **Frontend:** Accounting tabs (Receivables/Payments/Checks only with Accounting view): `/accounting/receivables/` (tiles, aging by client with limit/over badge, unpaid DR list, filters overdue/bucket/over limit/credit), `/accounting/payments/` (list + `?id=` detail with void, check steps, "Record replacement"), `/accounting/checks/` (tiles: to deposit within 7 days, PDCs, deposited, bounced; inline step buttons). Shared `components/accounting/accounting.tsx`: `PaymentFormModal` (allocation table with "Fill oldest first", replaced-check picker that pre-fills amount + allocations), `CheckStepModal`, `CreditWarning` (booking form + draft/confirmed booking detail: balance + this booking vs credit limit; warning only), badges, `MONEY_TABLES`. DR list has a Balance column, DR detail a Payments section, client detail a Balance card + payments table. `format.ts`: `addDays`.
- **Gotchas:** (1) **A running Vite dev server doesn't learn about new route folders**: client-side nav into them silently does nothing (full reloads work). Restart Vite after adding routes. (2) A child component's bare `{count}` number prop didn't update on re-render (Checks tiles); passing a preformatted string prop works. (3) Modal `saved$` that navigates: reset `busy` before awaiting it, or a hung nav leaves the submit button disabled. (4) Disk filled again during M8 (`target/debug/incremental` deleted, ~2.5 GB free); build with `CARGO_INCREMENTAL=0`. (5) In Git Bash `cargo` isn't on PATH: `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Verified:** 3 tests in `tests/accounting.rs` (full flow: PDC, guards, deposit-before-date, bounce/undo, replacement and its void, credit, edit/clear allocations, DR void after unallocating; VAT rounding view = Rust; offline double payment → overpaid, no conflicts) + aging unit test; full core/net suite incl. sim; clippy clean; `npm run build` clean. Clicked through on one instance over CDP: receivables aging (52 days overdue in 31–60), PDC via form with "Fill oldest first", check recorded → deposit → bounce (DR owes again) → replacement pre-filled → paid, checks tiles, DR payments section, client balance card, booking credit-limit warning.
- **For M9/M10:** Dashboard KPIs can use `receivables::report` (overdue, clients over limit), `payments::list` (PDCs due to deposit this week = on hand with check date ≤ today+7, bounced not replaced). Reports (M10): collections, PDC calendar, aging CSV from the same functions. Not done: bounced-check to-dos/notifications, aging "as of" a past date, deposit batches/bank reconciliation, payment receipts printing.

**M9 notes**
- **User answers (2026-10-23):** (1) PO unit prices, totals, FX and TT payments are **cost data** (`Field::Costs`): stripped in Rust for users without it; their PO saves keep the stored prices/currency/FX (new POs get the supplier's currency, no prices); TT commands and the TT payments tab need costs. (2) Receiving **doesn't** touch cost sheets; a PO-page button "Update cost sheets from this PO" (`po_to_cost_sheets`, needs costs + Items *edit*) copies each item's first priced line: supplier price, PO currency and FX (charge lines kept).
- **Migration 0008 (schema v8):** replicated `purchase_orders(supplier_id, date, currency, fx_rate, expected_date, status, supplier_ref, notes, ordered_at/by, shipped_date, arrived_date, closed_at/by, cancel_reason)`, `po_lines(po_id, line_no, item_id, qty, price_minor, notes)`, `supplier_payments(po_id, date, currency, amount_minor, fx_rate, reference, notes)`, `po_receipts(po_id, date, location_id, notes, status posted|void, voided_*)`, `po_receipt_lines(receipt_id, line_no, po_line_id, item_id, expiry, qty)`. `v_stock_moves`/`v_stock` recreated with kind `po_receipt` (+qty at the receipt's location, on its date); new view `v_po_received(po_line_id, qty)` from posted reports. Column list of `v_stock_moves` unchanged.
- **Deviation from the data-model sketch:** `po_receipts` has no `no` column (numbers in `doc_numbers`, series `RR`; PO series `PO`, both automatic). TT table is `supplier_payments` as sketched; TT entries are plain editable/deletable rows (v7 ids), any currency with their own FX (required unless PHP), date ≤ today.
- **Domain** (`domain/purchasing.rs`): `PoStatus` draft → ordered (Orders *approve*) → shipped → arrived → closed, or cancelled. Ship/arrive take a date (blank = today, not future). Back to draft / cancel only without posted receipts; close from ordered/shipped/arrived; reopen returns to arrived/shipped/ordered by the dates set. Delete only drafts without TT payments. **No edit locks** (like bookings): editable in any status but cancelled; a received line can't be removed, change item, or go below what was received. Receiving (`validate_receipt` → reserve RR → `receive`, Orders *create*): PO must be ordered/shipped/arrived; one location; date ≤ today; expiry required exactly for items that track it; qty per PO line ≤ ordered − received ("raise the PO quantity first"). A receipt moves an ordered/shipped PO to **arrived** (arrived_date = receipt date if unset). Receipts are posted documents: **void and re-enter** (Orders *delete*, reason; refused if the stock was used — `inventory::check_not_negative` on the undo). Progress none/partial/full and per-line received/remaining are computed. `PoInfo` has total (PO currency), total in PHP at the PO FX, paid in PO currency (TTs in other currencies only count in `paid_php_c`), unpriced line count.
- **Inventory integration:** `StockKind::PoReceipt` (`post_stock_doc` refuses it), `StockMove.po_id`, `item_moves` joins `po_receipts`; `inventory::{live_location, doc_effect}` now `pub(crate)`. Location delete blocked by receipts, item delete by PO lines, supplier delete by POs.
- **Offline:** two PCs receiving the same PO line offline → both count (line remaining negative, shown red), no conflict (tested).
- **Commands** (`commands/orders.rs`): `list_pos, get_po, save_po, set_po_status, delete_po, po_to_cost_sheets, list_po_payments, save_po_payment, delete_po_payment, list_po_receipts, get_po_receipt (Orders or Inventory view), receive_po, void_po_receipt`. `commands::items::require_costs` is now `pub(crate)`.
- **Frontend:** Order tabs Purchase orders (`/orders/`: list with status/"still to receive"/supplier filters and a "late" badge past the expected date; `?id=` detail with status actions, lines with received/left, totals + PHP + paid + balance, TT section, receiving reports, cost-sheet button), Receiving (`/orders/receiving/` list + `?id=` detail with void), TT payments (`/orders/payments/`, costs only, PHP total of shown rows), Suppliers. Shared `components/orders/orders.tsx`: `PoFormModal` (currency follows the supplier on new POs, live total + PHP), `TtModal`, `ReceiveModal` (one row per open line prefilled with what's left, "+ lot" splits by expiry, parent-owned form via `receiveForm(detail)`), `PO_STATUS`, `ORDER_TABLES`. Inventory item history links PO receipts to `/orders/receiving/?id=`; stock screens reload on `po_receipts`.
- **Qwik gotcha found (also fixed in Bookings):** a child prop computed inline from a signal (`title={ACTION_TEXT[action.value].title}`) is treated as constant, so the `Modal` title went stale (body text in the parent updated fine). Pass a `useComputed$` signal's `.value` instead. Probably the same cause as M8 gotcha (2).
- **Verified:** 2 tests in `tests/orders.rs` (full flow: validation, numbers, statuses, TT in USD and PHP, receiving with expiry rules and over-receipt refusal, edit guards, no-costs save keeps prices, second report, void refused when stock used / allowed otherwise, re-entry, close/reopen, cost sheets, delete guards; offline double receipt) ; full core/net suite incl. sim; clippy (`--all-targets`) clean; `npm run build` clean. Clicked through on one instance over CDP: new PO via form (US$1,370 = ₱77,405 preview), ordered → shipped (date modal) → TT US$500 at 56 → receive with a split GLV lot + TAPE into Warehouse (RR-2026-0001, PO arrived, partly received) → cost sheets updated → over-receipt refused → receipt into Office → void → stock page shows the RR moves; staff user sees no prices/FX/TT, `list_po_payments` refused, staff edit kept prices/currency/FX.
- **Disk:** C: filled again (11 GB in `target/debug/deps`, mostly stale test binaries). Deleted `target/debug/incremental` and older duplicate `*.exe/*.pdb` per test name (~2.4 GB). ~2 GB free after the build.
- **For M10:** Dashboard KPI "POs in transit" = `purchasing::list` with status ordered/shipped (or open with progress ≠ full); late = expected date past. Report "PO spend by supplier (PHP)" can sum `TtInfo.php_c` (paid) or `PoInfo.total_php_c` (ordered). Near-expiry alert after receiving works through `inventory::expiring`. Not done: prefill PO prices from cost sheets, PO printing, partial cancellation of a line (lower the qty instead), landed-cost per PO (actual freight/duty invoices).

**M10 notes**
- **No migration** (schema stays v8). App version bumped to **1.0.0** (`Cargo.toml` workspace + `tauri.conf.json`) for the rollout build.
- **Dashboard** (`domain/dashboard.rs`, command `dashboard`, Dashboard view): KPIs per module, each only for users who can view that module — open BD requests (follow-ups due), bookings to deliver, DRs to deliver (requested/approved), receivables + overdue (clients over limit), checks to deposit within 7 days, bounced checks, low stock, near expiry (expired lots), POs in transit (late = expected date past). Each KPI links to the filtered list.
- **Deviation — data issues are computed live, not written as to-dos.** The plan said invariant findings become Dashboard to-dos; instead `dashboard::summary` returns `issues` (negative stock, DR number used twice, DRs overpaid, payments applied above their amount, PO lines received above ordered, duplicate item codes / client names, unresolved conflicts) that disappear once fixed. No stored derived values, no to-do cleanup. Blocked sync peers show from `sync_status`. App-written to-dos (renumbering, typed-twice numbers) still exist.
- New list filters for the issue links: Orders `?status=late|over` (`PoInfo.over_received`), Clients `?status=duplicate`.
- **To-dos** (`domain/todos.rs`, commands `list_todos, save_todo, set_todo_done, delete_todo`, any signed-in user): shared (no assignee) or for one active user. Everyone sees shared ones, their own and those they assigned; anyone who sees one may edit/tick it; delete = creator, assignee or admin. Ordered by due date, undated last. A to-do with `ref_tbl/ref_id` links to its document (number from `doc_numbers`).
- **Reports** (`domain/reports.rs` + `commands/reports.rs`, Reports view; `/reports/` tabs): Sales by client/item/month (delivered DRs by delivery date; item VAT per line), Receivables aging (by client or DR, from `receivables::report`), Collections & checks (payments by date with totals per method, PDC calendar, bounced checks — from `payments::list`), Stock (by location, by lot, expiring), PO spend by supplier (costs; ordered at PO FX + TT paid at own FX), BD conversion by month (win rate = won ÷ (won + lost)), Item margin (costs + margins; **today's landed cost**, costs aren't kept per delivery). Shared `components/reports/report.tsx` (`makeReport`, `searchRows`, `ReportTable` with Export CSV, `toCsv`); `lib/events.ts` `useLiveLoadUrl` reloads on query-string changes. **CSV** = `save_csv` writes `Downloads/sanco-<name>-<stamp>.csv` (UTF-8 BOM, CRLF, money/percent as plain decimals) and `show_in_folder` opens Explorer. No dialog plugin added.
- **Update flow (no server):** `sanco-net/src/update.rs`. The NSIS hook copies the installer itself to `$INSTDIR\sanco-setup.exe` (deleted on uninstall); every installed PC serves it at `GET /update` (info: version, size, SHA-256) and `GET /update/installer` (bearer token as usual). `pull_from` now records a peer's hello (name, `app_version`) **before** the handshake checks, so a peer refused for another schema still shows its version. `Node::update_offer` = newest peer version above ours; `Node::download_update` checks it is newer, size + SHA-256 + `MZ`, saves `<data>/updates/Sanco_<v>_x64-setup.exe`. Shell: `update_status` (offer, older peers, `can_share`), `install_update(addr)` downloads, starts `cmd /C start "" <exe> /P /R` (ShellExecute → UAC prompt; passive install; NSIS `/R` restarts Sanco) and quits. Banner `components/shell/update-banner.tsx` on top of every screen: "Sanco X is available on PC-Y · Update now", or older PCs that can't sync (blocked). Testing flags: `--installer PATH`, `--app-version V`.
- **Gotcha:** an updated PC can only hand on versions installed with the setup program (`sanco-setup.exe` must exist next to `sanco.exe`). The pilot PCs run 0.1.0, which has no update feature: **install 1.0.0 by hand on every PC once**; later versions spread with "Update now".
- **Verified:** 3 tests in `tests/finish.rs` (KPIs/issues and per-module visibility, sales/margin/BD reports, to-do visibility and delete rights), 2 unit + 1 net test for updates (offer, download, refusal of non-newer, missing installer); full core/net suite incl. sim; clippy clean; `npm run build` clean. Clicked through on two local instances over CDP: Dashboard figures (₱560 DR − ₱200 PDC = ₱360 overdue, follow-up due, late PO), to-do add/tick/show done/edit/delete, every report incl. filter change and CSV export, update banner from a `--app-version 9.9.0` peer and "Update now" (downloaded, checksum ok, installer started, app quit).
- **Rollout checklist (needs the user, 5 PCs):** (1) Back up first: Settings → Backups → "Back up now" on the PC with the most data. (2) **All PCs must update together** (schema v8 since M9; a PC on an older schema stops syncing until updated). (3) Run `target/release/bundle/nsis/Sanco_1.0.0_x64-setup.exe` on each PC (admin prompt; per-machine). (4) Check `netsh advfirewall firewall show rule name="Sanco sync"` and Tailscale Shields Up off. (5) New PCs: join with a code from Settings → Devices (admin). (6) Check Settings → Devices shows every PC online, then create users/roles for the 5 staff. (7) Later updates: install the new version on one PC; the others get the banner.
- Not done: printing (none planned), report "as of" past dates, stock valuation report, scheduled CSV exports, signing the installer (Windows SmartScreen may warn on first run).

**Post-M10 additions (2026-10-23, not committed yet)**
- **Searchable dropdowns** (`src/lib/select-search.ts`, installed in `routes/layout.tsx` like the date picker): any single `<select>` with ≥ 8 options opens a themed popover with a search box instead of the native list; a pick sets the value and fires `input` + `change`. `data-search="off"|"on"` overrides. No screen changes needed.
- **Bank accounts (migration 0009, schema v9; every PC must update together).** User answers: link check deposits, bank transfers received and TT payments; the check's *drawee* bank stays free text; totals per account, **no running balance** (withdrawals and bank charges aren't recorded).
  - Replicated `bank_accounts` (name, bank, account_no, currency, notes, active). New columns `payments.bank_account_id` (checks: deposited to, set by the deposit step and cleared by undo; bank transfers: received into, saved with the payment) and `supplier_payments.bank_account_id` (paid from). The old free-text `payments.deposit_account` stays for earlier deposits; `CheckInfo.deposit_account` shows the account name, else that text. `CheckStepInput.account` became `bank_account_id`.
  - `domain/bank_accounts.rs`: `list` (cleared / pending / bounced checks, transfers in, TT out in PHP), `moves` (activity, TT only with costs), `usable` (exists and active, or the one already stored), `save` (unique name), `delete` (refused once any payment or TT names it → mark inactive). Commands in `commands/bank_accounts.rs`: list = Accounting **or** Orders view (TT totals stripped without costs); moves = Accounting view; save/delete = Accounting create/edit/delete.
  - UI: Accounting → **Bank accounts** tab (`routes/accounting/bank-accounts/`): account cards, add/edit/delete modal, activity table with FilterBar (`?account=&kind=&status=`). Shared `BankAccountSelect` (`components/accounting/bank-account-select.tsx`) in the payment form (bank transfer), the check deposit step and the TT form.
  - Verified: `tests/bank_accounts.rs` (2 tests), full core/net suite, clippy, `npm run build`; clicked through over CDP (add accounts, bank transfer into BDO, check deposited to BDO, the tab's totals and list).
- **M12 DR or Invoice** (requested 2026-10-27): plan in `docs/plans/m12-dr-invoice.md`; notes below the M11 notes.
- **Rollout impact:** the 1.0.0 installer built in M10 is schema v8. Done: version bumped to **1.1.0** (commit `f651ea9`, pushed) and `target/release/bundle/nsis/Sanco_1.1.0_x64-setup.exe` built. Roll out 1.1.0 (not 1.0.0) to all PCs together; the M10 checklist otherwise applies.

**M11 notes (boxes and box serials, 2026-10-27)**
- **Migration 0011 (schema v11):** `items.serial_code/serial_digits/pcs_per_box`, `purchase_orders.serial_year`, `po_lines.pcs_per_box`, `stock_doc_lines.box_id`, `po_receipt_lines.box_id`; replicated `boxes(item_id, po_id, po_line_id, doc_id, pcs)` and `dr_line_boxes(dr_id, dr_line_id, box_id, expiry, qty)`; `v_stock_moves`/`v_stock` recreated with `box_id` (delivery rows come from `dr_line_boxes` when a line has draws); new view `v_box_received`.
- **Decisions made while building (not in the agreed plan):**
  - Box rows use fresh ids (not derived), so a removed box never blocks a later one. Removing a box also deletes its `doc_numbers` row; `max_seq` still counts deleted rows, so serials are never reused.
  - Loose pieces on a PO line make **one more (partial) box** with its own serial (`pcs` = the rest). `boxes.pcs` = size when full; what is left is `v_stock`.
  - **Back to draft and cancel remove unreceived boxes** (drafts strictly use no numbers); re-ordering gives new serials. The serial year can only change while the PO has no boxes.
  - A received box keeps its `pcs` even if fewer arrived; a short box shows as "open". Lowering qty keeps all received boxes and plans the rest from `qty − Σ received box pcs`.
  - Stock receipts (SR) without a box make boxes (`boxes.doc_id`, serial year chosen on the form, default this year); voiding the SR deletes its boxes once nothing else moved them. Adjustments/transfers name the box (expiry taken from the box's first incoming lot). A positive adjustment of a boxed item needs a box; an outgoing line without a box only works if the item has loose (unboxed) stock there.
  - DR draws: empty = auto at save (`boxes::auto_draws`: started boxes, earliest expiry, lowest serial), with a delivered DR's own draws counted back when editing. Draws are **not** re-picked at delivery: if a box was emptied meanwhile, delivery is refused and the DR is edited.
  - Numbering: `NumberClaim.count` (serde default 1) claims a run in one round per peer (`numbering::reserve_batch` → `Batch`, `assign_seq`). The claim's `ref_id` is the PO/SR id; each box's `doc_numbers` row has the box id. `resolve_clashes` skips `BOX:` series (`keeps_clashes`).
  - `item_options` now returns `ItemChoice` (+ `unit`, `serial_code`, `pcs_per_box`) so PO/booking forms know box sizes without Items view.
  - `ItemStock.negative` also flags a box below zero hidden in a non-negative lot.
- **Command flow:** `save_po` (edit) → `purchasing::serials_for_update` → `reserve_serials` → `update(…, &mut Serials)`; `set_po_status` → `serials_for_status`; `post_stock_doc` → `inventory::serials_for_doc`. New commands `box_stock(item_id)` (Inventory view) and `dr_boxes(booking_id)` (same viewers as `dr_lots`). `Node::reserve_serials`, `commands::sync::reserve_serials`.
- **UI:** item form (serial code / digits / pcs per box); PO form (serial year, boxes + loose + pcs/box per boxed line); PO detail (serial year, box list per line); receive modal (one row per unreceived box, tick/untick, expiry for all ticked); receiving detail (Box column); stock doc form (SR boxes + loose + year; adjust/transfer box picker, "whole box"); inventory list/item (box summary "8 full + 1 open (7)", Boxes card, Box column in moves); DR modal (auto preview, "Choose boxes" per-box qty, saved draws kept in edit); DR detail (draws per line); bookings (box equivalent).
- **Verified:** `tests/boxes.rs` (3 tests: serials/padding/year/shared counter, order/raise/lower/back-to-draft/cancel, receiving per box and its refusals, `v_stock`/lots per box, SR boxes + void, adjust/transfer per box, DR auto/typed draws, edit of a delivered DR, void returns pieces, batch claims across peers, offline clash not renumbered) + 3 unit tests in `domain/boxes.rs`; full core/net suite incl. sim; clippy clean; `npm run build` clean. Clicked through on one instance over CDP: PO with GLV (MT, 10/box) 97 pc + MSK 12 pc at 4/box ordered → MT26-0001…0013, edit form shows 9 boxes + 7 loose, receiving 12 boxes with "expiry for all" and one box skipped, inventory box summary/list, adjustment box picker, DR auto preview = saved draws, edit of the delivered DR with "Choose boxes".
- **Not done:** putting pieces back into an empty box from the UI (backend supports SR with `box_id`), box labels/printing, re-picking draws automatically at delivery.

**M12 notes (DR or Invoice, 2026-10-27)**
- **Model:** no new column. `delivery_receipts.vat` *is* the type (`delivery::ReceiptKind`, serde `"dr"`/`"invoice"`). `DrRequestInput.kind: Option<ReceiptKind>` (None = the booking's VAT on request, unchanged on edit). Switching is refused once delivered or cancelled, and it resets `vat_bp` to the current rate.
- **Numbers:**
  - New series `SI` (`numbering::series::INVOICE`), in `MANUAL_SERIES` and in Settings → Number formats ("Sales invoices").
  - Migration **0012** only adds view `v_receipt_no`: the receipt's live DR/SI row, preferring its type's series. Every DR-number join now uses it (delivery, receivables, payments, inventory `item_moves`).
  - `delivery::number_series` = the series of the row it holds, else the type's. So pre-M12 VAT DRs became invoices with DR-series numbers, and corrections to them stay in the DR series.
  - `numbering::release` frees a number: it sets `seq`/`no` NULL and keeps the text in `prev_no`. It drops local claims; peers' claims expire after 10 min. NULL rows fall out of the clash, max and claim queries on their own. `assign_or_change` re-numbers a freed row.
- **DTO changes:**
  - New fields: `DrInfo.kind`/`no_series`, `ReceivableDr.kind`, `AllocationInfo.kind`, `OpenDr.kind`, `ClientBalance.open_invoices`, `SalesRow.invoices`, `StockMove.invoice`.
  - `ClientBalance.open_drs` and `SalesRow.drs` now count DRs only.
  - `PaymentInfo.dr_nos` → `receipts`, with labels like "DR 123" / "Invoice 456".
  - Dashboard: KPIs `drs_open` + `invoices_open`, issues `dr_clash` + `invoice_clash`.
  - Error messages say DR or invoice by type.
- **UI:**
  - Accounting tabs **Delivery receipts** + **Invoices** (`routes/accounting/invoices/`). Both render `components/sales/receipts.tsx` (`ReceiptsPage kind`), and a receipt opened under the other route redirects. `sales.tsx` has `RECEIPT`, `receiptHref`, `receiptName`.
  - Request/edit modal: a "The client gets" DR / Invoice picker (default from the booking, locked once delivered) plus live net/VAT/gross.
  - Lists split by type: booking detail; client page (tabs Delivery receipts / Invoices; balance reads "Owed on N DRs and M invoices"); receivables (Unpaid DRs / Unpaid invoices).
  - The payment picker stays one list with a "DR / invoice" column, because one payment can cover both.
  - Sales report: separate DRs and Invoices columns (no type filter added). Receivables report by receipt: a Type column.
- **Verified:**
  - 2 new tests in `tests/sales.rs`: DR vs invoice totals, the same digits in SI and DR don't clash, a switch frees the number for reuse, switching back, refusal after delivery, receivables and stock moves per type, and a pre-M12 invoice keeps its DR number.
  - Existing tests updated: their VAT bookings now make invoices.
  - The sim has `SI` and a release op (`release_number`); doc_numbers conflicts are allowed only on released rows.
  - Full core/net suite, clippy and `npm run build` are clean.
- **Not done:**
  - **UI click-through:** the disk filled up (≈3 GB free) while building a second target dir, which was then removed. Run `npm run app` (restart Vite first: there is a new route folder) and check the screens above. A copy of the M11 dev data is at `.dev/m12`.
  - `cargo fmt` also reformatted some untouched files (`examples/import_master.rs`, `domain/items.rs`, `domain/mod.rs`, `domain/purchasing.rs`, `tests/bank_accounts.rs`, `tests/inventory.rs`, `tests/orders.rs`). The changes are formatting only.

**BD removal notes (2026-10-27)**
- **User decision:** the Business Development module is no longer needed. Removed: `domain/bd.rs`, `commands/bd.rs` (5 commands), `report_bd` + `reports::bd_conversion`/`BdRow`/`win_rate`, routes `/bd/` and `/reports/bd/`, nav entry, Dashboard "Open requests" KPI, `Module::Bd`, series `BD` (const and Settings → Number formats row), `BookingInfo.bd_request_id`/`bd_no` and the "From request" link, `bookings::insert` (and the `quoted` flags in `prepare`).
- **Kept on purpose:** tables `bd_requests`, `bd_lines` and column `bookings.bd_request_id` stay in migrations and `registry.rs` (additive only; a PC still on 1.1.0 syncs them). No migration, no schema bump: old and new versions still handshake. `PriceSource::Quoted` stays so old booking lines keep their rule (label "Quoted in BD"). Existing `doc_numbers` rows with series `BD` stay; nothing claims new ones.
- **Gotcha fixed:** `Perms` used to fail to parse a role whose `perms_json` named an unknown module, and a failed parse grants nothing. `Perms.modules` now deserializes through `known_modules`, which skips unknown keys (test `perms_skip_unknown_modules`). Saving a role drops the stale `"bd"` grant.
- Client and item delete guards no longer look at BD requests/lines.
- **Tests:** `tests/sales.rs` `bd_request_to_delivered_dr` → `booking_to_delivered_dr` (booking typed directly; gloves line now `Custom`); BD part of the reports test in `tests/finish.rs` dropped.
- **Verified:** full core/net suite, clippy clean, `npm run build` clean. `src/lib/bindings.ts` was **not** regenerated (the dev app held `target/debug/sanco.exe`; disk also near full, `target/debug/incremental` was cleared): the next debug start or `npm run bindings` regenerates it, then commit it.

**M13 notes (macOS support, started 2026-10-28)**
- **Plan location:** moved into the repo as `docs/PLAN.md` on 2026-10-31, so it follows the code to other PCs. The copy in `~/.claude/plans` is out of date.
- **Goal:** Sanco on Mac laptops, syncing with the Windows PCs. The sync protocol is OS-independent, so a mixed fleet is fine.
- **Done on `macos-support`:**
  - `sanco_core::machine` (`machine.rs`): `id()` = `MachineGuid` on Windows (unchanged, so existing PCs keep their site ids), `IOPlatformUUID` on macOS; `name()` adds `scutil --get ComputerName`. Used by `state.rs` and `examples/import_master.rs`. Without it every Mac had id `"PC"`, so a database copied between Macs at the same path could keep its site id.
  - `Hello.os` (`std::env::consts::OS`, serde default `"windows"` for older peers; older peers ignore the field). `PeerStatus.os`, `NodeConfig.os` (tests fake it). `update_offer` only offers peers on the same OS.
  - `update.rs`: `INSTALLER_NAME` per OS (`sanco-setup.dmg` on macOS), `download_name()`, `is_installer()` (`MZ`, or the dmg's `koly` trailer 512 bytes from the end). `install_update` → `start_installer()`: on macOS `open` the dmg, the user drags Sanco to Applications and restarts it.
  - `show_in_folder`: `open -R` on macOS. `tailscale::detect_ip`: also the app bundle and Homebrew paths (apps started from Finder don't see the shell's PATH).
  - `crates/sanco-net/tests/net.rs` covers a newer peer on another OS (no offer). Its `MZ` fake installer is Windows-only: the test needs a dmg-shaped file before it can pass on a Mac.
- **Verified on Windows only:** clippy, full core/net suite incl. the sim. `#[cfg(target_os = "macos")]` code has never been compiled.
- **Windows PCs need no update for the Mac test:** installed 1.2.0 has the same schema (v12).
- **Not done:**
  1. **Mac build.** A friend's Mac is compiling it (`npm run app`, then `npx tauri build --bundles app,dmg`; `tauri.conf.json` targets only `nsis`, so the flag is needed). Can't cross-compile from Windows. Fix any Mac errors on the branch.
  2. **Mac test:** Tailscale, join with a code, a booking both ways, tray, autostart, "Show in folder", CSS/date formatting in WebKit. If it used the real workspace: remove the Mac from Settings → Devices, delete Sanco and `~/Library/Application Support/ph.sanco.desktop`.
  3. **Bundle config:** add a `tauri.macos.conf.json` with targets `app`, `dmg`; decide on Intel + Apple Silicon (`--target universal-apple-darwin`).
  4. **Distribution:** decide GitHub Actions (`tauri-apps/tauri-action`) vs building on a Mac. Unsigned is the current assumption: each user clicks System Settings → Privacy & Security → "Open Anyway" once. Signing + notarization needs the $99/year Apple account.
  5. **Mac-to-Mac updates:** nothing puts a dmg next to the app, so a Mac only shares one with `--installer`. Decide how the dmg gets there. Until then Macs update by hand.
  6. **Finish:** merge to main; change "Windows only for now" in `CLAUDE.md`, and the `Commands`/`Sync` update notes there.
- **Gotchas:**
  - A Windows PC older than this branch doesn't filter by OS: with a newer Mac it may show a banner, and "Update now" fails safely with "not a Windows program".
  - `cargo fmt --all` reformats untouched files; `c5b90d9` caught them up. Reverting them with `git checkout` was blocked as destructive.
  - Sanco on this PC is installed in the repo folder (`C:\Users\Harrie\Sanco`, hence the untracked `sanco.exe`, `sanco-setup.exe`, `uninstall.exe`). Don't `git clean`. Better reinstalled to the default location.

## Milestones
- **M0 — Setup.**
  - Install rustup (MSVC) and VS Build Tools (C++ workload).
  - Scaffold the Tauri 2 + QwikCity static adapter + Tailwind v4.
  - Get one tauri-specta command working end to end.
  - **Write `CLAUDE.md`:** overview, stack and pinned versions, commands, architecture rules (above), domain glossary (BD, booking, DR, PDC, TT, landed cost, margin tier), module map, testing rules.
- **M1 — Sync core** in `sanco-core`: HLC, registry, repo, merge, row_versions, conflicts, audit, plus the `tests/sim.rs` simulation.
- **M2 — Numbering service,** added to the simulation.
- **M3 — Networking:** server, token, poke/pull, devices, snapshot join, handshake checks. Test with 3 local instances (`--data-dir`, `--port`).
- **M4 — App shell:**
  - Login, roles and per-user module access, audit.
  - App shell and sidebar, plus settings (sync status, conflicts, backups).
  - Tray and autostart, the NSIS installer with the firewall hook.
  - **Pilot on 2 real PCs over Tailscale.**
- **M5 — Data entry:** Clients, Suppliers, Items (catalog, cost sheet, margins, client rates, price resolution), Locations.
- **M6 — Inventory:** stock view, receipts, adjustments, transfers, expiry and low-stock lists.
- **M7 — Sales flow:** Business Development → Booking → DR request → delivered.
- **M8 — Accounting:** checks, payments, allocations, receivables and aging.
- **M9 — Orders:** POs, TT entries, receiving into stock.
- **M10 — Finish:** Dashboard (KPIs, alerts, to-dos), Reports with CSV export, an update flow (version-mismatch banner and installer handoff), then rollout to all 5 PCs.
- **M13 — macOS support:** Mac build and test against the Windows PCs, bundle config, distribution, Mac-to-Mac updates, merge `macos-support`. Details in the M13 notes under Progress.

## Verification
- **Simulation (`cargo test -p sanco-core`):**
  - N peers with a seeded random generator and a skewed clock per peer.
  - Random operations, network partitions, restarts and restores.
  - Finish with full-mesh sync and check that table hashes match on every peer, no new versions appear after things settle, and no duplicate document numbers survive.
- **Unit tests:** price resolution, landed cost, `v_stock`, `v_receivables`, aging.
- **Three local instances:** create on A and see it on B and C within seconds. Partition A, edit on both sides, reconnect, and check the conflict shows on the Dashboard. Skewed clock gets rejected. A restored database gets a new epoch.
- **Two real PCs over Tailscale (M4 pilot):** firewall rule works, sync survives a Tailscale restart, and tray/autostart keeps the app online.
- **End-to-end business flows:**
  - BD request → booking → DR delivered: stock drops and a receivable appears. A PDC check is received and allocated: balance is 0. Bounce it: the balance reappears.
  - PO → TT entry → receive into Warehouse with expiry: stock rises, and the near-expiry alert shows on the Dashboard.
- **Typechecks:** `npm run build` (Qwik typecheck) and `cargo clippy` stay clean.

## User answers (2026-10-21)
1. **Margin = margin on price:** price = cost ÷ (1 − margin%). ₱100 at 20% → ₱125.
2. **Number format includes the year:** default `DR-2026-0001`. The format stays editable per document type in Settings (prefix, year on/off, yearly reset, padding).
3. **Full RBAC in Settings:** admin-editable roles. Each role grants permissions per module and action (view/create/edit/delete/approve), plus sensitive fields (item costs, margins). Users get one or more roles. Seed roles: Admin (all) and Staff (no Settings, no cost/margin view). Replaces the fixed admin/staff + modules_json model: tables `roles(name, perms_json)` and `user_roles(user_id, role_id)`.
4. **VAT:** a "VAT (12%)" checkbox on bookings, DRs and per-client rates; the rate is editable in Settings. Selling price from cost = cost ÷ (1 − margin) × (1 + VAT if checked). The UI shows net, VAT and gross.
5. No Excel import for now. Build CSV import later if needed.
6. Locations: Office and Warehouse only (still addable).
7. All Windows PCs + 1 laptop, all with Tailscale. **No user/device caps:** any number of users and PCs.
8. **Cost-sheet charge types are an editable list** (Settings → Charge types), seeded with Freight, Customs duty, Brokerage, Bank fees, Import VAT. The cost sheet becomes generic lines `item_cost_charges(charge_type_id, mode: per_unit|pct, value)`.
9. DR number = the booklet number. No separate manual_ref field.
10. **DR numbers (answered 2026-10-22; built in M7, see M7 notes):** staff **type** the DR number from the BIR booklet for now; later it should become **automatic**. Build M7 so the switch is a setting, not a rewrite:
    - Keep the DR number in `doc_numbers` (series `DR`) like every other number, with a flag for manual entry, so documents still join on `ref_id`.
    - Manual mode: add `numbering::reserve_exact` (claim the typed number with reachable peers, refuse if taken), validate before reserving. Never auto-renumber a manual number: on an offline clash, flag it (badge + to-do) for staff to fix.
    - Automatic mode later: a per-series setting (e.g. `number_mode.DR = manual|auto`) switches the DR form to `numbering::reserve`; existing manual numbers keep their text.

**Additional requirements**
- **Filters on every module list screen:** client, date range, status, location/supplier where relevant, plus text search. Use one shared `FilterBar` component; filters sync to the `?query` string.
- **Theme (from the Sanco Medical Supply logo):** white background as the primary surface. Teal-green `#13876F` for primary elements (buttons, active nav, links, focus rings), with tints for hover/selected states and a light mint `#E8F5F1` for subtle fills. Dark text `#1F2937`. Logo: white "S" square on a green rounded pill.
