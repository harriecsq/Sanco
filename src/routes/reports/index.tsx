import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type SalesGroup, type SalesRow } from "~/lib/bindings";
import { useLiveLoadUrl } from "~/lib/events";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  makeReport,
  REPORT_TABLES,
  ReportTable,
  searchRows,
  type Cell,
} from "~/components/reports/report";
import { Alert, PageHeader } from "~/components/ui/ui";

const GROUPS: { value: SalesGroup; label: string }[] = [
  { value: "client", label: "By client" },
  { value: "item", label: "By item" },
  { value: "month", label: "By month" },
];

const groupOf = (url: URL): SalesGroup => {
  const g = url.searchParams.get("by");
  return g === "item" || g === "month" ? g : "client";
};

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const rows = useSignal<SalesRow[]>([]);
  const error = useSignal("");

  useLiveLoadUrl(
    REPORT_TABLES,
    $(async () => {
      if (!session.user) return;
      const f = filtersOf(loc.url, ["from", "to"]);
      try {
        rows.value = unwrap(await commands.reportSales({ from: f.from || null, to: f.to || null }, groupOf(loc.url)));
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const group = groupOf(loc.url);
    const q = loc.url.searchParams.get("q") ?? "";
    if (group === "month") {
      const data: Cell[][] = rows.value.map((r) => [r.key, r.drs, r.net_c, r.vat_c, r.gross_c]);
      return makeReport(
        "sales-by-month",
        [{ label: "Month" }, { label: "DRs", kind: "qty" }, { label: "Net", kind: "money" }, { label: "VAT", kind: "money" }, { label: "Gross", kind: "money" }],
        searchRows(data, q),
      );
    }
    if (group === "item") {
      const data: Cell[][] = rows.value.map((r) => [r.code, r.label, r.qty, r.drs, r.net_c, r.vat_c, r.gross_c]);
      return makeReport(
        "sales-by-item",
        [{ label: "Code" }, { label: "Item" }, { label: "Qty", kind: "qty" }, { label: "DRs", kind: "qty" }, { label: "Net", kind: "money" }, { label: "VAT", kind: "money" }, { label: "Gross", kind: "money" }],
        searchRows(data, q),
      );
    }
    const data: Cell[][] = rows.value.map((r) => [r.code, r.label, r.drs, r.net_c, r.vat_c, r.gross_c]);
    return makeReport(
      "sales-by-client",
      [{ label: "Code" }, { label: "Client" }, { label: "DRs", kind: "qty" }, { label: "Net", kind: "money" }, { label: "VAT", kind: "money" }, { label: "Gross", kind: "money" }],
      searchRows(data, q),
    );
  });

  return (
    <>
      <PageHeader title="Sales" subtitle="Delivered DRs by delivery date (voided ones left out). Item VAT is worked out per line." />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search"
        dates
        selects={[{ key: "by", label: "Grouping", allLabel: "By client", options: GROUPS.slice(1) }]}
      />
      <ReportTable report={report} />
    </>
  );
});

export const head: DocumentHead = { title: "Sales report · Sanco" };
