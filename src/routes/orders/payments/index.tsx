import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type SupplierChoice, type TtInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatMinor, formatPeso } from "~/lib/format";
import { sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { CURRENCIES, ORDER_TABLES } from "~/components/orders/orders";
import { Alert, Empty, NoAccess, PageHeader } from "~/components/ui/ui";
import { card, hint, table, td, th } from "~/components/ui/styles";

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const payments = useSignal<TtInfo[]>([]);
  const suppliers = useSignal<SupplierChoice[]>([]);
  const error = useSignal("");

  useLiveLoad(
    ORDER_TABLES,
    $(async () => {
      if (!sees(session.user, "costs")) return;
      try {
        payments.value = unwrap(await commands.listPoPayments());
        suppliers.value = unwrap(await commands.supplierOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "supplier", "currency"]);
    const q = f.q.toLowerCase();
    return payments.value.filter(
      (t) =>
        (!q || [t.po_no, t.supplier_name, t.reference, t.notes].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || t.date >= f.from) &&
        (!f.to || t.date <= f.to) &&
        (!f.supplier || t.supplier_id === f.supplier) &&
        (!f.currency || t.currency === f.currency),
    );
  });

  if (!sees(session.user, "costs")) return <NoAccess />;
  const totalPhp = shown.value.reduce((s, t) => s + (t.php_c ?? 0), 0);
  const missingFx = shown.value.some((t) => t.php_c == null);

  return (
    <>
      <PageHeader title="TT payments" subtitle="Payments to suppliers. Add them from the PO's page." />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search PO, supplier, reference"
        dates
        selects={[
          {
            key: "supplier",
            label: "Suppliers",
            options: suppliers.value.map((s) => ({ value: s.id, label: s.label })),
          },
          {
            key: "currency",
            label: "Currencies",
            options: CURRENCIES.map((c) => ({ value: c, label: c })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{payments.value.length ? "No payments match." : "No TT payments yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Date</th>
                <th class={th}>PO</th>
                <th class={th}>Supplier</th>
                <th class={[th, "text-right"]}>Amount</th>
                <th class={[th, "text-right"]}>Rate</th>
                <th class={[th, "text-right"]}>In pesos</th>
                <th class={th}>Reference</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((t) => (
                <tr key={t.id}>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(t.date)}</td>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="text-brand-700 hover:underline" href={`/orders/?id=${t.po_id}`}>
                      {t.po_no ?? "—"}
                    </Link>
                  </td>
                  <td class={td}>{t.supplier_name ?? "—"}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatMinor(t.amount_minor, t.currency)}</td>
                  <td class={[td, "text-right tabular-nums"]}>{t.fx_rate ?? "—"}</td>
                  <td class={[td, "text-right tabular-nums"]}>{t.php_c == null ? "—" : formatPeso(t.php_c)}</td>
                  <td class={td}>
                    {t.reference ?? "—"}
                    {t.notes && <span class={[hint, "block"]}>{t.notes}</span>}
                  </td>
                </tr>
              ))}
            </tbody>
            <tfoot>
              <tr>
                <td class={[td, "font-semibold"]} colSpan={5}>
                  Total shown{missingFx && <span class={[hint, "ml-2 font-normal"]}>(some without a rate)</span>}
                </td>
                <td class={[td, "text-right font-semibold tabular-nums"]}>{formatPeso(totalPhp)}</td>
                <td class={td} />
              </tr>
            </tfoot>
          </table>
        )}
      </section>
    </>
  );
});

export const head: DocumentHead = { title: "TT payments · Sanco" };
