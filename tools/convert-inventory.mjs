import ExcelJS from "exceljs";
import fs from "node:fs";

const [src, out] = process.argv.slice(2);
const wb = new ExcelJS.Workbook();
await wb.xlsx.readFile(src);
const ws = wb.getWorksheet("Inventory");
const head = {};
ws.getRow(1).eachCell((c, i) => (head[String(c.value).trim()] = i));
for (const h of ["Key", "Desc", "Shipment date", "KB", "SM STORE", "SM WAREHOUSE"])
  if (!head[h]) throw new Error(`missing column ${h}`);

const DEFAULT_DATE = "2026-10-31";
const LOCS = [["KB", "KB"], ["SM STORE", "Office"], ["SM WAREHOUSE", "Warehouse"]];
const val = (row, h) => {
  const v = row.getCell(head[h]).value;
  return v && typeof v === "object" && "result" in v ? v.result : v;
};
const iso = (v) => {
  if (v == null || v === "") return DEFAULT_DATE;
  if (v instanceof Date) return v.toISOString().slice(0, 10);
  const s = String(v).trim();
  if (/^\d{4}-\d{2}-\d{2}/.test(s)) return s.slice(0, 10);
  const d = new Date(s);
  if (isNaN(d)) throw new Error(`bad date ${s}`);
  return d.toISOString().slice(0, 10);
};

const items = new Map();
const lines = [];
const skipped = [];
const rows = { total: 0, dated: 0 };
ws.eachRow((row, n) => {
  if (n === 1) return;
  const code = String(val(row, "Key") ?? "").trim();
  if (!code) return;
  rows.total++;
  const name = String(val(row, "Desc") ?? "").trim();
  if (items.has(code) && items.get(code) !== name) throw new Error(`${code}: two names`);
  items.set(code, name);
  const raw = val(row, "Shipment date");
  if (raw != null && raw !== "") rows.dated++;
  const date = iso(raw);
  for (const [col, location] of LOCS) {
    const q = Number(val(row, col) ?? 0);
    if (!q) continue;
    if (q < 0 || !Number.isInteger(q)) { skipped.push({ row: n, code, col, q }); continue; }
    lines.push({ code, date, location, qty: q });
  }
});
const totals = {};
for (const l of lines) totals[l.location] = (totals[l.location] ?? 0) + l.qty;
fs.writeFileSync(out, JSON.stringify({
  items: [...items].map(([code, name]) => ({ code, name })),
  lines,
}));
console.log(JSON.stringify({ rows, items: items.size, lines: lines.length, totals, skipped }, null, 1));
