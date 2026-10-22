import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type BdRow } from "~/lib/bindings";
import { useLiveLoadUrl } from "~/lib/events";
import { formatPct } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { makeReport, REPORT_TABLES, ReportTable, type Cell } from "~/components/reports/report";
import { Alert, PageHeader } from "~/components/ui/ui";
import { card } from "~/components/ui/styles";

/** Won ÷ (won + lost) in basis points, rounded like the backend. */
const winRate = (won: number, lost: number) =>
  won + lost > 0 ? Math.floor((won * 10000 + Math.floor((won + lost) / 2)) / (won + lost)) : null;

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const rows = useSignal<BdRow[]>([]);
  const error = useSignal("");

  useLiveLoadUrl(
    REPORT_TABLES,
    $(async () => {
      if (!session.user) return;
      const f = filtersOf(loc.url, ["from", "to"]);
      try {
        rows.value = unwrap(await commands.reportBd({ from: f.from || null, to: f.to || null }));
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const data: Cell[][] = rows.value.map((r) => [
      r.month,
      r.total,
      r.open,
      r.quoted,
      r.won,
      r.lost,
      r.win_rate_bp,
      r.won_net_c,
      r.lost_net_c,
    ]);
    const rep = makeReport(
      "bd-conversion",
      [
        { label: "Month" },
        { label: "Requests", kind: "qty" },
        { label: "Open", kind: "qty" },
        { label: "Quoted", kind: "qty" },
        { label: "Won", kind: "qty" },
        { label: "Lost", kind: "qty" },
        { label: "Win rate", kind: "pct" },
        { label: "Won (quoted)", kind: "money" },
        { label: "Lost (quoted)", kind: "money" },
      ],
      data,
    );
    if (rep.total) rep.total[6] = winRate(rep.total[4] as number, rep.total[5] as number);
    return rep;
  });

  const totals = useComputed$(() => {
    const won = rows.value.reduce((s, r) => s + r.won, 0);
    const lost = rows.value.reduce((s, r) => s + r.lost, 0);
    const rate = winRate(won, lost);
    return { rate: rate == null ? "—" : formatPct(rate), counts: `${won} · ${lost}` };
  });

  return (
    <>
      <PageHeader
        title="BD conversion"
        subtitle="Requests by month of their date. Win rate = won ÷ (won + lost). Values are quoted prices before VAT."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar dates />
      <div class="mb-3 flex flex-wrap gap-3">
        <div class={[card, "px-4 py-2"]}>
          <p class="text-xs font-semibold uppercase tracking-wide text-ink-muted">Win rate</p>
          <p class="font-semibold tabular-nums">{totals.value.rate}</p>
        </div>
        <div class={[card, "px-4 py-2"]}>
          <p class="text-xs font-semibold uppercase tracking-wide text-ink-muted">Won · lost</p>
          <p class="font-semibold tabular-nums">{totals.value.counts}</p>
        </div>
      </div>
      <ReportTable report={report} />
    </>
  );
});

export const head: DocumentHead = { title: "BD conversion · Sanco" };
