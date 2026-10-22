import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type SpendRow } from "~/lib/bindings";
import { useLiveLoadUrl } from "~/lib/events";
import { sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { makeReport, REPORT_TABLES, ReportTable, searchRows, type Cell } from "~/components/reports/report";
import { Alert, NoAccess, PageHeader } from "~/components/ui/ui";

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const rows = useSignal<SpendRow[]>([]);
  const error = useSignal("");

  useLiveLoadUrl(
    REPORT_TABLES,
    $(async () => {
      if (!sees(session.user, "costs")) return;
      const f = filtersOf(loc.url, ["from", "to"]);
      try {
        rows.value = unwrap(await commands.reportPoSpend({ from: f.from || null, to: f.to || null }));
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const q = loc.url.searchParams.get("q") ?? "";
    const data: Cell[][] = rows.value.map((r) => [
      r.supplier_code,
      r.supplier_name,
      r.po_count,
      r.ordered_php_c,
      r.incomplete_pos,
      r.tt_count,
      r.paid_php_c,
      r.tt_without_fx,
    ]);
    return makeReport(
      "po-spend-by-supplier",
      [
        { label: "Code" },
        { label: "Supplier" },
        { label: "POs", kind: "qty" },
        { label: "Ordered", kind: "money" },
        { label: "POs missing a price or FX", kind: "qty" },
        { label: "TT payments", kind: "qty" },
        { label: "Paid", kind: "money" },
        { label: "TTs without FX", kind: "qty" },
      ],
      searchRows(data, q),
    );
  });

  if (!sees(session.user, "costs")) return <NoAccess />;
  return (
    <>
      <PageHeader
        title="PO spend by supplier"
        subtitle="In PHP. Ordered: POs dated in the range (not drafts or cancelled) at each PO's FX. Paid: TT payments dated in the range at their own FX."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar search="Search supplier" dates />
      <ReportTable report={report} />
    </>
  );
});

export const head: DocumentHead = { title: "PO spend · Sanco" };
