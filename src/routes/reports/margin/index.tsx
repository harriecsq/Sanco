import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type MarginRow } from "~/lib/bindings";
import { useLiveLoadUrl } from "~/lib/events";
import { sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { makeReport, REPORT_TABLES, ReportTable, searchRows, type Cell } from "~/components/reports/report";
import { Alert, NoAccess, PageHeader } from "~/components/ui/ui";

/** Margin on price in basis points, rounded half away from zero. */
function marginBp(net: number, cost: number): number | null {
  if (net === 0) return null;
  const v = ((net - cost) * 10000) / net;
  return Math.sign(v) * Math.round(Math.abs(v));
}

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const rows = useSignal<MarginRow[]>([]);
  const error = useSignal("");

  useLiveLoadUrl(
    REPORT_TABLES,
    $(async () => {
      if (!sees(session.user, "costs") || !sees(session.user, "margins")) return;
      const f = filtersOf(loc.url, ["from", "to"]);
      try {
        rows.value = unwrap(await commands.reportMargin({ from: f.from || null, to: f.to || null }));
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const q = loc.url.searchParams.get("q") ?? "";
    const data: Cell[][] = rows.value.map((r) => [
      r.code,
      r.name,
      r.qty,
      r.unit,
      r.net_c,
      r.unit_cost_c,
      r.cost_c,
      r.margin_c,
      r.margin_bp,
    ]);
    const rep = makeReport(
      "item-margin",
      [
        { label: "Code" },
        { label: "Item" },
        { label: "Qty", kind: "qty" },
        { label: "Unit" },
        { label: "Sales (net)", kind: "money" },
        { label: "Unit cost now", kind: "money" },
        { label: "Cost", kind: "money" },
        { label: "Margin", kind: "money" },
        { label: "Margin %", kind: "pct" },
      ],
      searchRows(data, q),
    );
    if (rep.total) {
      rep.total[2] = rep.total[5] = null;
      // Margin % over the items with a known cost.
      const known = rows.value.filter((r) => r.cost_c != null);
      const net = known.reduce((s, r) => s + r.net_c, 0);
      const cost = known.reduce((s, r) => s + (r.cost_c ?? 0), 0);
      rep.total[8] = marginBp(net, cost);
    }
    return rep;
  });

  if (!sees(session.user, "costs") || !sees(session.user, "margins")) return <NoAccess />;
  return (
    <>
      <PageHeader
        title="Item margin"
        subtitle="Delivered sales (before VAT) against today's landed cost from the cost sheet: costs aren't kept per delivery. Items without a complete cost sheet show no margin."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar search="Search item" dates />
      <ReportTable report={report} />
    </>
  );
});

export const head: DocumentHead = { title: "Item margin · Sanco" };
