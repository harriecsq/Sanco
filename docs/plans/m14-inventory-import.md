# M14: inventory import

Seeds the real stock into Sanco from the cleaned Excel file. Decided with the user on 2026-11-02. Read `docs/plans/m14-lots.md` and the lots rules in `CLAUDE.md` first.

## Start here
- Lots are built, committed as `73c726c` and pushed on branch `lots-replace-boxes`. They aren't merged to `main` and aren't released.
- **Never run `npm run app` or a plain `sanco.exe` on this PC for testing.** That opens the real database and migrates it to v14. Use `--data-dir .dev/x`.
- The real database is at `%APPDATA%\ph.sanco.desktop\sanco.db`: schema v13, 0 items, 0 stock. Its locations are Office, Warehouse and KB. Its devices are HARRIE-PC and Wynona's MacBook Air.

## Source file
`C:\Users\Harrie\Downloads\Sanco inventory - cleaned.xlsx`, sheet `Inventory`. It's built from `Sanco inventory.xlsx` in the same folder; don't change either original.

| Column | Use |
|---|---|
| Key | Item code |
| Desc | Item name |
| Shipment date | Lot month (blank = undated) |
| KB | Qty at location KB |
| SM STORE | Qty at location Office |
| SM WAREHOUSE | Qty at location Warehouse |
| Source row, Original key, Notes | Trace only; not imported |

- 266 rows, 251 unique keys, one row per item and shipment. 66 rows have a date, 200 are blank.
- Expected totals before the PIB-100 fix below: KB 2,000, SM STORE 6,595, SM WAREHOUSE 107,582.
- Read `.xlsx` with `exceljs`. It isn't in the repo; Python isn't installed. Install it outside the project (scratchpad), since it's only needed for conversion.

## Rules (user decisions)
- **Items:** one per unique Key: code and name from the file, unit, category and supplier blank, `tracks_expiry` off, active on.
- **Same Key on several rows:** the same item, with one lot per shipment date. The cleaned file already gives each row of a Key the same name.
- **Blank Shipment date:** use **Oct 31, 2026**, which gives lot Oct 2026.
- **Receipts:** normal stock receipts with `SR` numbers, one document per date and location, with the note `Opening balance`. There's no special opening-balance type. A document holds at most 500 lines.
- **Zero or blank quantities:** skip that line. Receipts refuse qty 0.
- **PIB-100 at SM STORE (-2):** treat as 0, so skip it. Its warehouse qty of 28 stays. Expected SM STORE total after the fix is 6,597, since the file's total includes the -2.
- **Serial ranges and expiry:** none. Leave them blank. The owner adds them later under Inventory → Lots.

## Open decision: how to run it
Numbers must be claimed through the sync node, and writes must go through `db.write`, so raw SQL is out. Recommended:
1. A Node script (scratchpad) turns the cleaned `.xlsx` into JSON, applying the rules above.
2. `sanco-core/src/domain/import.rs` validates that JSON and creates items and receipts through `items::save` and `inventory::create`.
3. An admin-only Tauri command, started from Settings, reserves `SR` numbers first and refuses if any item already exists, so it can't run twice.

Confirm this with the user before building. If it's a one-time tool, ask whether to remove the command afterwards.

## Work order
1. Confirm the approach above.
2. Build the conversion and the import, with tests in `crates/sanco-core/tests/` (see `tests/lots.rs`).
3. Dry run on `.dev/import` with locations Office, Warehouse and KB. Check item count (251), totals per location, and that items with several shipments show several lots:
   DBC-6, DBC-9, B-043-608-BK, CK-605P H-01, H-05, H-06, H-08, H-09, H-16, TH01C, MS-116, MS-200M (3 lots), CK-605-01A, EN1-EB1.
4. Show the user the result before touching real data.
5. The real import needs a release first: merge to `main`, bump the version in `Cargo.toml` **and** `tauri.conf.json`, tag `v*`, publish the draft, and update the PC that runs the import. The other PC can update later: it can't sync with v14 until it does (schema v14), then it pulls everything. "Release X.Y.Z" from the user means the full tag, build, publish and install flow.
6. Back up, then run the import on one PC only. The other gets it by sync. Confirm the totals again.

## Done means
- All rows are in stock with totals matching the file, except the skipped -2.
- `cargo test -p sanco-core -p sanco-net`, `cargo clippy --workspace` and `npm run build` pass.
- `docs/PLAN.md` Progress and M14 notes are updated, then run `graphify update .`.
