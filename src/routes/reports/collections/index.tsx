import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type PaymentInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatPeso } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { CHECK_STATUS, METHOD_LABEL } from "~/components/accounting/accounting";
import { makeReport, REPORT_TABLES, ReportTable, searchRows, type Cell } from "~/components/reports/report";
import { Alert, PageHeader } from "~/components/ui/ui";
import { card } from "~/components/ui/styles";

const VIEWS = [
  { value: "pdc", label: "PDC calendar (checks on hand)" },
  { value: "bounced", label: "Bounced checks" },
];

const inRange = (d: string | null | undefined, from: string, to: string) =>
  !!d && (!from || d >= from) && (!to || d <= to);

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const pays = useSignal<PaymentInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    REPORT_TABLES,
    $(async () => {
      if (!session.user) return;
      try {
        pays.value = unwrap(await commands.reportPayments());
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "view"]);
    const posted = pays.value.filter((p) => p.status === "posted");
    if (f.view === "pdc" || f.view === "bounced") {
      const want = f.view === "pdc" ? "on_hand" : "bounced";
      const checks = posted
        .filter((p) => p.check?.status === want && inRange(p.check.check_date, f.from, f.to))
        .sort((a, b) => (a.check!.check_date ?? "").localeCompare(b.check!.check_date ?? ""));
      const rows: Cell[][] = checks.map((p) => [
        p.check!.check_date,
        p.no,
        p.client_name,
        p.check!.bank,
        p.check!.check_no,
        f.view === "pdc" ? (p.check!.pdc ? "PDC" : "Due") : (p.check!.bounce_reason ?? ""),
        p.amount_c,
      ]);
      return makeReport(
        f.view === "pdc" ? "pdc-calendar" : "bounced-checks",
        [
          { label: "Check date", kind: "date" },
          { label: "Payment" },
          { label: "Client" },
          { label: "Bank" },
          { label: "Check no." },
          { label: f.view === "pdc" ? "Kind" : "Reason" },
          { label: "Amount", kind: "money" },
        ],
        searchRows(rows, f.q),
      );
    }
    const shown = posted.filter((p) => inRange(p.date, f.from, f.to));
    const rows: Cell[][] = shown.map((p) => [
      p.date,
      p.no,
      p.or_no,
      p.client_name,
      METHOD_LABEL[p.method],
      p.check ? CHECK_STATUS[p.check.status].label : p.counts ? "Received" : "—",
      p.amount_c,
      p.applied_c,
      p.unapplied_c,
    ]);
    return makeReport(
      "collections",
      [
        { label: "Date", kind: "date" },
        { label: "Payment" },
        { label: "OR/CR no." },
        { label: "Client" },
        { label: "Method" },
        { label: "Status" },
        { label: "Amount", kind: "money" },
        { label: "Applied", kind: "money" },
        { label: "Unapplied", kind: "money" },
      ],
      searchRows(rows, f.q),
    );
  });

  // Totals by method for the payments view.
  const byMethod = useComputed$(() => {
    const f = filtersOf(loc.url, ["from", "to", "view"]);
    if (f.view) return [];
    const sums = new Map<string, number>();
    for (const p of pays.value) {
      if (p.status !== "posted" || !p.counts || !inRange(p.date, f.from, f.to)) continue;
      const k = METHOD_LABEL[p.method];
      sums.set(k, (sums.get(k) ?? 0) + p.amount_c);
    }
    return [...sums.entries()];
  });

  return (
    <>
      <PageHeader
        title="Collections & checks"
        subtitle="Posted payments by date; the PDC calendar and bounced checks go by check date."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search payment, client, bank, check no."
        dates
        selects={[{ key: "view", label: "View", allLabel: "Payments received", options: VIEWS }]}
      />
      {byMethod.value.length > 0 && (
        <div class="mb-3 flex flex-wrap gap-3">
          {byMethod.value.map(([m, c]) => (
            <div key={m} class={[card, "px-4 py-2"]}>
              <p class="text-xs font-semibold uppercase tracking-wide text-ink-muted">{m}</p>
              <p class="font-semibold tabular-nums">{formatPeso(c)}</p>
            </div>
          ))}
        </div>
      )}
      <ReportTable report={report} />
    </>
  );
});

export const head: DocumentHead = { title: "Collections & checks · Sanco" };
