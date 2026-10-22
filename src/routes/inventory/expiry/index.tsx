import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type LocationInfo, type StockLot } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatQty } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { ExpiryBadge, NEAR_EXPIRY_DAYS } from "~/components/inventory/expiry-badge";
import { Alert, Empty, PageHeader } from "~/components/ui/ui";
import { card, table, td, th } from "~/components/ui/styles";

const WINDOWS = [30, 60, 90, 180, 365];
/** The widest window is loaded once; narrower ones filter it. */
const MAX_WINDOW = Math.max(...WINDOWS);

export default component$(() => {
  const loc = useLocation();
  const lots = useSignal<StockLot[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    ["items", "locations", "stock_docs", "stock_doc_lines", "delivery_receipts"],
    $(async () => {
      try {
        lots.value = unwrap(await commands.expiringStock(MAX_WINDOW));
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const f = filtersOf(loc.url, ["q", "location", "within"]);
  const within = f.within === "expired" ? -1 : Number(f.within) || NEAR_EXPIRY_DAYS;

  const shown = useComputed$(() => {
    const g = filtersOf(loc.url, ["q", "location", "within"]);
    const days = g.within === "expired" ? -1 : Number(g.within) || NEAR_EXPIRY_DAYS;
    const q = g.q.toLowerCase();
    return lots.value.filter(
      (l) =>
        (l.days_left ?? 0) <= days &&
        (!g.location || l.location_id === g.location) &&
        (!q || [l.item_code, l.item_name, l.category].some((v) => v?.toLowerCase().includes(q))),
    );
  });

  return (
    <>
      <PageHeader
        title="Expiring stock"
        subtitle="Lots in stock that have expired or expire soon, soonest first."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, category"
        selects={[
          {
            key: "within",
            label: "Window",
            allLabel: `Within ${NEAR_EXPIRY_DAYS} days`,
            options: [
              { value: "expired", label: "Expired only" },
              ...WINDOWS.filter((w) => w !== NEAR_EXPIRY_DAYS).map((w) => ({
                value: String(w),
                label: `Within ${w} days`,
              })),
            ],
          },
          {
            key: "location",
            label: "Locations",
            options: locations.value.map((l) => ({ value: l.id, label: l.name })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>
            {within < 0 ? "No expired stock." : `Nothing expires within ${within} days.`}
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Expiry</th>
                <th class={th}>Item</th>
                <th class={th}>Location</th>
                <th class={[th, "text-right"]}>Qty</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((l) => (
                <tr key={`${l.item_id}/${l.location_id}/${l.expiry}`}>
                  <td class={[td, "whitespace-nowrap"]}>
                    {formatDay(l.expiry)} <ExpiryBadge days={l.days_left} within={MAX_WINDOW} />
                  </td>
                  <td class={td}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/inventory/?item=${l.item_id}`}
                    >
                      {l.item_code}
                    </Link>{" "}
                    {l.item_name}
                  </td>
                  <td class={td}>{l.location_name ?? "Deleted location"}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatQty(l.qty, l.unit)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </>
  );
});

export const head: DocumentHead = { title: "Expiring stock · Sanco" };
