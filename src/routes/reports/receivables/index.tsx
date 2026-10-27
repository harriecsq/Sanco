import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type ReceivablesReport } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar } from "~/components/filter-bar/filter-bar";
import { makeReport, REPORT_TABLES, ReportTable, searchRows, type Cell } from "~/components/reports/report";
import { Alert, PageHeader } from "~/components/ui/ui";

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const data = useSignal<ReceivablesReport | null>(null);
  const error = useSignal("");

  useLiveLoad(
    REPORT_TABLES,
    $(async () => {
      if (!session.user) return;
      try {
        data.value = unwrap(await commands.reportReceivables());
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const r = data.value;
    const q = loc.url.searchParams.get("q") ?? "";
    const view = loc.url.searchParams.get("view");
    if (view === "drs") {
      const rows: Cell[][] = (r?.drs ?? [])
        .filter((d) => d.balance_c !== 0)
        .map((d) => [d.kind === "invoice" ? "Invoice" : "DR", d.no, d.client_code, d.client_name, d.delivered_date, d.due_date, Math.max(d.days_overdue, 0), d.gross_c, d.paid_c, d.balance_c]);
      return makeReport(
        "receivables-by-dr",
        [
          { label: "Type" },
          { label: "No." },
          { label: "Code" },
          { label: "Client" },
          { label: "Delivered", kind: "date" },
          { label: "Due", kind: "date" },
          { label: "Days overdue", kind: "qty" },
          { label: "Gross", kind: "money" },
          { label: "Paid", kind: "money" },
          { label: "Balance", kind: "money" },
        ],
        searchRows(rows, q),
      );
    }
    const rows: Cell[][] = (r?.clients ?? []).map((c) => [
      c.client_code,
      c.client_name,
      c.terms_days,
      c.credit_limit_c,
      c.aging.not_due_c,
      c.aging.d1_30_c,
      c.aging.d31_60_c,
      c.aging.d61_90_c,
      c.aging.over_90_c,
      c.outstanding_c,
      c.credit_c,
      c.balance_c,
    ]);
    const rep = makeReport(
      "receivables-aging",
      [
        { label: "Code" },
        { label: "Client" },
        { label: "Terms (days)", kind: "qty" },
        { label: "Credit limit", kind: "money" },
        { label: "Not due", kind: "money" },
        { label: "1–30", kind: "money" },
        { label: "31–60", kind: "money" },
        { label: "61–90", kind: "money" },
        { label: "Over 90", kind: "money" },
        { label: "Outstanding", kind: "money" },
        { label: "Credit", kind: "money" },
        { label: "Balance", kind: "money" },
      ],
      searchRows(rows, q),
    );
    // Terms and limits don't add up.
    if (rep.total) rep.total[2] = rep.total[3] = null;
    return rep;
  });

  return (
    <>
      <PageHeader
        title="Receivables aging"
        subtitle={`As of ${formatDay(data.value?.as_of ?? null)}. Days past due = since delivery + the client's terms.`}
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search client, DR or invoice no."
        selects={[{ key: "view", label: "View", allLabel: "By client", options: [{ value: "drs", label: "By DR / invoice" }] }]}
      />
      <ReportTable report={report} empty="No receivables." />
    </>
  );
});

export const head: DocumentHead = { title: "Receivables aging · Sanco" };
