import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type Aging, type ReceivablesReport } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatPeso } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  BUCKETS,
  BUCKET_LABEL,
  MONEY_TABLES,
  PaymentFormModal,
  blankPayment,
} from "~/components/accounting/accounting";
import { RECEIPT, receiptHref } from "~/components/sales/sales";
import { Alert, Badge, Empty, NoAccess, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { btnGhost, card, hint, table, td, th } from "~/components/ui/styles";

const AGING_KEYS: (keyof Aging)[] = ["not_due_c", "d1_30_c", "d31_60_c", "d61_90_c", "over_90_c"];

const Tile = component$<{ label: string; value: number; note?: string; tone?: "danger" }>(
  ({ label, value, note, tone }) => (
    <div class={[card, "px-4 py-3"]}>
      <div class="text-xs font-semibold uppercase tracking-wide text-ink-muted">{label}</div>
      <div class={["mt-1 text-lg font-semibold tabular-nums", tone === "danger" && value > 0 && "text-danger"]}>
        {formatPeso(value)}
      </div>
      {note && <div class={hint}>{note}</div>}
    </div>
  ),
);

/** A peso amount, or a dash for zero. */
const amt = (c: number) => (c === 0 ? "—" : formatPeso(c));

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const report = useSignal<ReceivablesReport | null>(null);
  const error = useSignal("");
  const formOpen = useSignal(false);
  const form = useStore(blankPayment());

  useLiveLoad(
    MONEY_TABLES,
    $(async () => {
      try {
        report.value = unwrap(await commands.receivablesReport());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const filtered = useComputed$(() => {
    const r = report.value;
    const f = filtersOf(loc.url, ["q", "from", "to", "client", "bucket"]);
    const q = f.q.toLowerCase();
    const clients = (r?.clients ?? []).filter(
      (c) =>
        (!q || [c.client_name, c.client_code].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.client || c.client_id === f.client) &&
        (!f.bucket ||
          (f.bucket === "overdue"
            ? c.overdue_c > 0
            : f.bucket === "over_limit"
              ? c.credit_limit_c != null && c.balance_c > c.credit_limit_c
              : f.bucket === "credit"
                ? c.credit_c > 0
                : r!.drs.some((d) => d.client_id === c.client_id && d.bucket === f.bucket && d.balance_c > 0))),
    );
    const ids = new Set(clients.map((c) => c.client_id));
    const drs = (r?.drs ?? []).filter(
      (d) =>
        ids.has(d.client_id ?? "") &&
        (!q || [d.no, d.client_name, d.client_code].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || (d.delivered_date ?? "") >= f.from) &&
        (!f.to || (d.delivered_date ?? "") <= f.to) &&
        (!f.bucket ||
          ["over_limit", "credit"].includes(f.bucket) ||
          (f.bucket === "overdue" ? d.days_overdue > 0 : d.bucket === f.bucket)),
    );
    return { clients, drs };
  });
  // Three lists on one screen, so each pages on its own key.
  const clientRows = useComputed$(() => filtered.value.clients);
  const drRows = useComputed$(() => filtered.value.drs.filter((d) => d.kind === "dr"));
  const invoiceRows = useComputed$(() => filtered.value.drs.filter((d) => d.kind === "invoice"));
  const clientPg = usePage(clientRows, undefined, "clients_page");
  const drPg = usePage(drRows, undefined, "dr_page");
  const invoicePg = usePage(invoiceRows, undefined, "invoice_page");

  if (!can(session.user, "accounting")) return <NoAccess />;
  const r = report.value;
  if (!r) return error.value ? <Alert>{error.value}</Alert> : null;
  const overdue = r.totals.d1_30_c + r.totals.d31_60_c + r.totals.d61_90_c + r.totals.over_90_c;
  const canPay = can(session.user, "accounting", "create");

  return (
    <>
      <PageHeader
        title="Receivables"
        subtitle={`What clients owe on delivered DRs and invoices, as of ${formatDay(r.as_of)}. Days count from each one's due date (delivery + payment terms).`}
      />
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <Tile
          label="Outstanding"
          value={r.outstanding_c}
          note={`${r.drs.filter((d) => d.balance_c > 0 && d.kind === "dr").length} unpaid DR(s) · ${r.drs.filter((d) => d.balance_c > 0 && d.kind === "invoice").length} unpaid invoice(s)`}
        />
        <Tile label="Overdue" value={overdue} tone="danger" />
        <Tile label="Client credit" value={r.credit_c} note="Paid but not applied to a DR or invoice" />
        <Tile label="Paid by uncleared checks" value={r.uncleared_c} note="On hand or deposited" />
      </div>
      <FilterBar
        search="Search client, DR or invoice no."
        dates
        selects={[
          {
            key: "bucket",
            label: "Balances",
            allLabel: "All balances",
            options: [
              { value: "overdue", label: "Overdue" },
              ...BUCKETS.map((b) => ({ value: b.key, label: b.label })),
              { value: "over_limit", label: "Over credit limit" },
              { value: "credit", label: "With credit" },
            ],
          },
          {
            key: "client",
            label: "Clients",
            options: r.clients.map((c) => ({ value: c.client_id, label: c.client_name ?? "—" })),
          },
        ]}
      >
        <span class={hint}>Dates filter the delivery date of DRs and invoices.</span>
      </FilterBar>

      <section class={[card, "mb-4 overflow-x-auto"]}>
        <h2 class="border-b border-line px-4 py-3 font-semibold">Aging by client</h2>
        {filtered.value.clients.length === 0 ? (
          <Empty>{r.clients.length ? "No clients match." : "Nothing owed. Delivered DRs and invoices show here until they're paid."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Client</th>
                {BUCKETS.map((b) => (
                  <th key={b.key} class={[th, "text-right"]}>
                    {b.label}
                  </th>
                ))}
                <th class={[th, "text-right"]}>Credit</th>
                <th class={[th, "text-right"]}>Balance</th>
                <th class={[th, "text-right"]}>Limit</th>
                {canPay && <th class={th}></th>}
              </tr>
            </thead>
            <tbody>
              {clientPg.value.rows.map((c) => {
                const over = c.credit_limit_c != null && c.balance_c > c.credit_limit_c;
                return (
                  <tr key={c.client_id}>
                    <td class={td}>
                      <Link class="font-medium text-brand-700 hover:underline" href={`/clients/?id=${c.client_id}`}>
                        {c.client_name ?? "—"}
                      </Link>
                      <span class={[hint, "block"]}>
                        {c.terms_days != null ? `${c.terms_days} days` : "No terms"}
                        {c.oldest_overdue_days != null && ` · oldest ${c.oldest_overdue_days} days overdue`}
                      </span>
                    </td>
                    {AGING_KEYS.map((k, i) => (
                      <td key={k} class={[td, "text-right tabular-nums", i > 0 && c.aging[k] > 0 && "text-danger"]}>
                        {amt(c.aging[k])}
                      </td>
                    ))}
                    <td class={[td, "text-right tabular-nums"]}>{amt(-c.credit_c)}</td>
                    <td class={[td, "text-right font-semibold tabular-nums"]}>{formatPeso(c.balance_c)}</td>
                    <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                      {c.credit_limit_c == null ? "—" : formatPeso(c.credit_limit_c)}
                      {over && (
                        <span class="ml-1">
                          <Badge tone="danger">over</Badge>
                        </span>
                      )}
                    </td>
                    {canPay && (
                      <td class={[td, "text-right whitespace-nowrap"]}>
                        <button
                          class={btnGhost}
                          onClick$={() => {
                            Object.assign(form, blankPayment(c.client_id));
                            formOpen.value = true;
                          }}
                        >
                          Record payment
                        </button>
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
            {filtered.value.clients.length > 1 && (
              <tfoot>
                <tr class="font-semibold">
                  <td class={td}>Total</td>
                  {AGING_KEYS.map((k) => (
                    <td key={k} class={[td, "text-right tabular-nums"]}>
                      {amt(filtered.value.clients.reduce((s, c) => s + c.aging[k], 0))}
                    </td>
                  ))}
                  <td class={[td, "text-right tabular-nums"]}>
                    {amt(-filtered.value.clients.reduce((s, c) => s + c.credit_c, 0))}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {formatPeso(filtered.value.clients.reduce((s, c) => s + c.balance_c, 0))}
                  </td>
                  <td class={td}></td>
                  {canPay && <td class={td}></td>}
                </tr>
              </tfoot>
            )}
          </table>
        )}
        <Pager of={clientPg} />
      </section>

      {(
        [
          ["dr", drPg],
          ["invoice", invoicePg],
        ] as const
      ).map(([kind, pg]) => {
        const k = RECEIPT[kind];
        const rows = pg.value.rows;
        return (
          <section key={kind} class={[card, "mb-4 overflow-x-auto"]}>
            <h2 class="border-b border-line px-4 py-3 font-semibold">Unpaid {k.plural}</h2>
            {pg.value.total === 0 ? (
              <Empty>No unpaid {k.plural} match.</Empty>
            ) : (
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>{k.title} no.</th>
                    <th class={th}>Client</th>
                    <th class={th}>Delivered</th>
                    <th class={th}>Due</th>
                    <th class={th}>Age</th>
                    <th class={[th, "text-right"]}>Gross</th>
                    <th class={[th, "text-right"]}>Paid</th>
                    <th class={[th, "text-right"]}>Balance</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((d) => (
                    <tr key={d.dr_id}>
                      <td class={[td, "whitespace-nowrap"]}>
                        <Link class="font-medium text-brand-700 hover:underline" href={receiptHref(d.kind, d.dr_id)}>
                          {d.no ?? `No ${k.name} no.`}
                        </Link>
                      </td>
                      <td class={td}>{d.client_name ?? "—"}</td>
                      <td class={[td, "whitespace-nowrap"]}>{formatDay(d.delivered_date)}</td>
                      <td class={[td, "whitespace-nowrap"]}>{formatDay(d.due_date)}</td>
                      <td class={[td, "whitespace-nowrap"]}>
                        {d.balance_c < 0 ? (
                          <Badge tone="warning">overpaid</Badge>
                        ) : d.days_overdue > 0 ? (
                          <Badge tone="danger">{d.days_overdue} days overdue</Badge>
                        ) : (
                          <span class="text-ink-muted">{BUCKET_LABEL[d.bucket]}</span>
                        )}
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.gross_c)}</td>
                      <td class={[td, "text-right tabular-nums"]}>{amt(d.paid_c)}</td>
                      <td class={[td, "text-right font-semibold tabular-nums", d.balance_c < 0 && "text-amber-800"]}>
                        {formatPeso(d.balance_c)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            <Pager of={pg} />
          </section>
        );
      })}
      <PaymentFormModal
        open={formOpen}
        form={form}
        saved$={$(async (id: string) => {
          await nav(`/accounting/payments/?id=${id}`);
        })}
      />
    </>
  );
});

export const head: DocumentHead = { title: "Receivables · Sanco" };
