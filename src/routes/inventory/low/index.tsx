import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type ItemStock, type SupplierChoice } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatQty } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { Alert, Empty, PageHeader } from "~/components/ui/ui";
import { card, table, td, th } from "~/components/ui/styles";

export default component$(() => {
  const loc = useLocation();
  const stock = useSignal<ItemStock[]>([]);
  const suppliers = useSignal<SupplierChoice[]>([]);
  const error = useSignal("");

  useLiveLoad(
    ["items", "suppliers", "stock_docs", "stock_doc_lines", "delivery_receipts"],
    $(async () => {
      try {
        stock.value = unwrap(await commands.stockOverview());
        suppliers.value = unwrap(await commands.supplierOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const low = useComputed$(() => stock.value.filter((s) => s.low));
  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "category", "supplier"]);
    const q = f.q.toLowerCase();
    return low.value.filter(
      (s) =>
        (!q || [s.code, s.name, s.category].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.category || s.category === f.category) &&
        (!f.supplier || s.supplier_id === f.supplier),
    );
  });
  const categories = [...new Set(low.value.map((s) => s.category).filter(Boolean))] as string[];
  const supplierName = (id: string | null) =>
    suppliers.value.find((s) => s.id === id)?.label ?? "—";

  return (
    <>
      <PageHeader
        title="Low stock"
        subtitle="Active items at or below their reorder level (set on each item)."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, category"
        selects={[
          {
            key: "category",
            label: "Categories",
            options: categories.sort().map((c) => ({ value: c, label: c })),
          },
          {
            key: "supplier",
            label: "Suppliers",
            options: suppliers.value.map((s) => ({ value: s.id, label: s.label })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{low.value.length ? "No items match." : "No items are low on stock."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Code</th>
                <th class={th}>Name</th>
                <th class={th}>Supplier</th>
                <th class={[th, "text-right"]}>On hand</th>
                <th class={[th, "text-right"]}>Reorder at</th>
                <th class={[th, "text-right"]}>Below by</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((s) => (
                <tr key={s.item_id}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/inventory/?item=${s.item_id}`}
                    >
                      {s.code}
                    </Link>
                  </td>
                  <td class={td}>{s.name}</td>
                  <td class={td}>{supplierName(s.supplier_id)}</td>
                  <td class={[td, "text-right tabular-nums", s.on_hand < 0 && "text-danger"]}>
                    {formatQty(s.on_hand, s.unit)}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>{formatQty(s.reorder_level ?? 0)}</td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {formatQty((s.reorder_level ?? 0) - s.on_hand)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </>
  );
});

export const head: DocumentHead = { title: "Low stock · Sanco" };
