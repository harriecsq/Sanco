import { component$, Slot, useSignal, type Signal } from "@builder.io/qwik";
import { commands } from "~/lib/bindings";
import { formatDay, formatPct, formatPeso } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Empty } from "~/components/ui/ui";
import { btnGhost, btnSecondary, card, table, td, th } from "~/components/ui/styles";

export type Kind = "text" | "money" | "qty" | "pct" | "date";
export interface Column {
  label: string;
  kind?: Kind;
}
export type Cell = string | number | null;

export interface ReportData {
  /** File name stem for CSV, e.g. "sales-by-client". */
  name: string;
  columns: Column[];
  rows: Cell[][];
  /** Footer row, or none. */
  total: Cell[] | null;
}

const numeric = (k: Kind | undefined) => k === "money" || k === "qty" || k === "pct";

/** A report with an optional "Total" row summing money and qty columns
 *  (the first column says "Total"; other columns stay blank). */
export function makeReport(
  name: string,
  columns: Column[],
  rows: Cell[][],
  totals = true,
): ReportData {
  let total: Cell[] | null = null;
  if (totals && rows.length > 1) {
    total = columns.map((c, i) =>
      i === 0
        ? "Total"
        : c.kind === "money" || c.kind === "qty"
          ? rows.reduce<number>((s, r) => s + (typeof r[i] === "number" ? (r[i] as number) : 0), 0)
          : null,
    );
  }
  return { name, columns, rows, total };
}

/** Rows with any text cell containing `q` (case-insensitive). */
export function searchRows(rows: Cell[][], q: string): Cell[][] {
  const s = q.trim().toLowerCase();
  if (!s) return rows;
  return rows.filter((r) => r.some((c) => typeof c === "string" && c.toLowerCase().includes(s)));
}

function show(c: Cell, kind: Kind | undefined): string {
  if (c == null || c === "") return "—";
  if (typeof c === "string") return kind === "date" ? formatDay(c) : c;
  switch (kind) {
    case "money":
      return formatPeso(c);
    case "pct":
      return formatPct(c);
    default:
      return c.toLocaleString("en-PH");
  }
}

function csvCell(c: Cell, kind: Kind | undefined): string {
  let s: string;
  if (c == null) s = "";
  else if (typeof c === "number") {
    s = kind === "money" || kind === "pct" ? (c / 100).toFixed(2) : String(c);
  } else s = c;
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function toCsv(r: ReportData): string {
  const head = r.columns.map((c) =>
    csvCell(c.kind === "money" ? `${c.label} (PHP)` : c.kind === "pct" ? `${c.label} (%)` : c.label, "text"),
  );
  const lines = [head.join(",")];
  for (const row of [...r.rows, ...(r.total ? [r.total] : [])]) {
    lines.push(row.map((c, i) => csvCell(c, r.columns[i]?.kind)).join(","));
  }
  return lines.join("\r\n") + "\r\n";
}

/** On-screen table with a CSV export. Pass the report as a signal (a
 *  `useComputed$`), so it updates with the filters. Extra controls can go
 *  in the default slot (left of the export button). */
export const ReportTable = component$<{ report: Signal<ReportData>; empty?: string }>(
  ({ report, empty }) => {
    const saved = useSignal("");
    const error = useSignal("");
    const r = report.value;
    return (
      <div class="space-y-2">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <p class="text-sm text-ink-muted">
            {r.rows.length} {r.rows.length === 1 ? "row" : "rows"}
          </p>
          <div class="flex items-center gap-2">
            <Slot />
            <button
              type="button"
              class={btnSecondary}
              disabled={r.rows.length === 0}
              onClick$={async () => {
                error.value = "";
                saved.value = "";
                try {
                  saved.value = unwrap(await commands.saveCsv(report.value.name, toCsv(report.value)));
                } catch (e) {
                  error.value = errorText(e);
                }
              }}
            >
              Export CSV
            </button>
          </div>
        </div>
        {error.value && <Alert>{error.value}</Alert>}
        {saved.value && (
          <Alert tone="brand">
            Saved to {saved.value}{" "}
            <button
              type="button"
              class={[btnGhost, "px-1 py-0"]}
              onClick$={() => commands.showInFolder(saved.value)}
            >
              Show in folder
            </button>
          </Alert>
        )}
        <div class={[card, "overflow-x-auto"]}>
          {r.rows.length === 0 ? (
            <Empty>{empty ?? "Nothing in this range."}</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  {r.columns.map((c) => (
                    <th key={c.label} class={[th, numeric(c.kind) && "text-right"]}>
                      {c.label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {r.rows.map((row, i) => (
                  <tr key={i} class="hover:bg-surface-muted">
                    {row.map((c, j) => (
                      <td
                        key={j}
                        class={[
                          td,
                          numeric(r.columns[j]?.kind) && "text-right tabular-nums",
                          typeof c === "number" && c < 0 && "text-danger",
                        ]}
                      >
                        {show(c, r.columns[j]?.kind)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
              {r.total && (
                <tfoot>
                  <tr class="font-semibold">
                    {r.total.map((c, j) => (
                      <td key={j} class={[td, numeric(r.columns[j]?.kind) && "text-right tabular-nums"]}>
                        {c == null ? "" : show(c, r.columns[j]?.kind)}
                      </td>
                    ))}
                  </tr>
                </tfoot>
              )}
            </table>
          )}
        </div>
      </div>
    );
  },
);

/** Tables whose changes can move a report. */
export const REPORT_TABLES = [
  "delivery_receipts",
  "dr_lines",
  "payments",
  "payment_allocations",
  "clients",
  "items",
  "item_costs",
  "item_cost_charges",
  "stock_docs",
  "stock_doc_lines",
  "bd_requests",
  "bd_lines",
  "purchase_orders",
  "po_lines",
  "po_receipts",
  "po_receipt_lines",
  "supplier_payments",
  "suppliers",
  "locations",
];
