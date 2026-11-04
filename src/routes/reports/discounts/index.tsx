import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type Choice, type DiscountReport } from "~/lib/bindings";
import { useLiveLoadUrl } from "~/lib/events";
import { discountAs } from "~/lib/discount";
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

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const data = useSignal<DiscountReport>({ clients: [], discounts: [] });
  const clients = useSignal<Choice[]>([]);
  const error = useSignal("");

  useLiveLoadUrl(
    REPORT_TABLES,
    $(async () => {
      if (!session.user) return;
      const f = filtersOf(loc.url, ["from", "to"]);
      try {
        data.value = unwrap(
          await commands.reportDiscounts({ from: f.from || null, to: f.to || null }),
        );
        clients.value = unwrap(await commands.clientOptions());
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const client = useComputed$(() => loc.url.searchParams.get("client") ?? "");

  const byClient = useComputed$(() => {
    const q = loc.url.searchParams.get("q") ?? "";
    const rows: Cell[][] = data.value.clients
      .filter((r) => !client.value || r.client_id === client.value)
      .map((r) => [r.code, r.name, r.invoices, r.invoiced_c, r.discount_c]);
    return makeReport(
      "discounts-by-client",
      [
        { label: "Code" },
        { label: "Client" },
        { label: "Invoices", kind: "qty" },
        { label: "Invoice totals", kind: "money" },
        { label: "Discount given", kind: "money" },
      ],
      searchRows(rows, q),
    );
  });

  const invoices = useComputed$(() => {
    const q = loc.url.searchParams.get("q") ?? "";
    const names = new Map(data.value.clients.map((c) => [c.client_id, c]));
    const rows: Cell[][] = data.value.discounts
      .filter((r) => !client.value || r.client_id === client.value)
      .map((r) => {
        const c = names.get(r.client_id);
        return [
          r.date,
          r.payment_no,
          r.invoice_no,
          c?.code ?? null,
          c?.name ?? "",
          discountAs(r.discount),
          r.invoice_c,
          r.discount_c,
        ];
      });
    const rep = makeReport(
      "discounts-by-payment",
      [
        { label: "Paid on", kind: "date" },
        { label: "Payment" },
        { label: "Invoice no." },
        { label: "Code" },
        { label: "Client" },
        { label: "Discount as" },
        { label: "Invoice total", kind: "money" },
        { label: "Discount", kind: "money" },
      ],
      searchRows(rows, q),
    );
    // An invoice can be discounted on more than one payment.
    if (rep.total) rep.total[6] = null;
    return rep;
  });

  return (
    <>
      <PageHeader
        title="Discounts"
        subtitle="Discounts given on invoices when they were paid, by payment date. Void payments and bounced or replaced checks are left out."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search client or invoice"
        dates
        selects={[
          {
            key: "client",
            label: "Client",
            allLabel: "All clients",
            options: clients.value.map((c) => ({ value: c.id, label: c.label })),
          },
        ]}
      />
      <h2 class="mt-2 mb-2 font-semibold">By client</h2>
      <ReportTable report={byClient} empty="No discounts in this range." />
      <h2 class="mt-6 mb-2 font-semibold">Discounts given</h2>
      <ReportTable report={invoices} pageKey="inv_page" empty="No discounts in this range." />
    </>
  );
});

export const head: DocumentHead = { title: "Discounts report · Sanco" };
