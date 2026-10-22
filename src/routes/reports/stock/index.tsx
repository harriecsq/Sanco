import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type LocationInfo, type StockLot } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { NEAR_EXPIRY_DAYS } from "~/components/inventory/expiry-badge";
import { makeReport, REPORT_TABLES, ReportTable, searchRows, type Cell } from "~/components/reports/report";
import { Alert, PageHeader } from "~/components/ui/ui";

const VIEWS = [
  { value: "lots", label: "By lot (expiry)" },
  { value: "expiring", label: `Expiring within ${NEAR_EXPIRY_DAYS} days` },
];

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const lots = useSignal<StockLot[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    REPORT_TABLES,
    $(async () => {
      if (!session.user) return;
      try {
        lots.value = unwrap(await commands.reportStock());
        locations.value = unwrap(await commands.listLocations());
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const report = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "location", "view"]);
    const shown = lots.value.filter((l) => !f.location || l.location_id === f.location);
    if (f.view === "lots" || f.view === "expiring") {
      const rows: Cell[][] = shown
        .filter((l) => f.view === "lots" || (l.days_left != null && l.days_left <= NEAR_EXPIRY_DAYS))
        .sort(
          (a, b) =>
            (a.expiry ?? "9999").localeCompare(b.expiry ?? "9999") || a.item_code.localeCompare(b.item_code),
        )
        .map((l) => [l.item_code, l.item_name, l.location_name, l.expiry, l.days_left, l.qty, l.unit]);
      const rep = makeReport(
        f.view === "lots" ? "stock-by-lot" : "expiring-stock",
        [
          { label: "Code" },
          { label: "Item" },
          { label: "Location" },
          { label: "Expiry", kind: "date" },
          { label: "Days left", kind: "qty" },
          { label: "Qty", kind: "qty" },
          { label: "Unit" },
        ],
        searchRows(rows, f.q),
        false,
      );
      return rep;
    }
    // Item × location totals.
    const sums = new Map<string, { l: StockLot; qty: number }>();
    for (const l of shown) {
      const k = `${l.item_id}|${l.location_id}`;
      const s = sums.get(k);
      if (s) s.qty += l.qty;
      else sums.set(k, { l, qty: l.qty });
    }
    const rows: Cell[][] = [...sums.values()]
      .sort(
        (a, b) =>
          a.l.item_code.localeCompare(b.l.item_code) ||
          (a.l.location_name ?? "").localeCompare(b.l.location_name ?? ""),
      )
      .map(({ l, qty }) => [l.item_code, l.item_name, l.category, l.location_name, qty, l.unit]);
    return makeReport(
      "stock-by-location",
      [
        { label: "Code" },
        { label: "Item" },
        { label: "Category" },
        { label: "Location" },
        { label: "Qty", kind: "qty" },
        { label: "Unit" },
      ],
      searchRows(rows, f.q),
      // Units of different items don't add up.
      false,
    );
  });

  return (
    <>
      <PageHeader title="Stock" subtitle="On hand now, by item and location, or per lot with expiry." />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search item, category"
        selects={[
          { key: "view", label: "View", allLabel: "By location", options: VIEWS },
          {
            key: "location",
            label: "Locations",
            options: locations.value.map((l) => ({ value: l.id, label: l.name })),
          },
        ]}
      />
      <ReportTable report={report} empty="No stock." />
    </>
  );
});

export const head: DocumentHead = { title: "Stock report · Sanco" };
