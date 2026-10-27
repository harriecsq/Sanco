import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type LocationInfo,
  type ReceiptDetail,
  type ReceiptInfo,
  type SupplierChoice,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDateTime, formatDay, formatQty } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { OfflineBadge } from "~/components/sales/sales";
import { ORDER_TABLES } from "~/components/orders/orders";
import { Alert, Badge, Empty, Field, InfoList, Modal, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { btnDanger, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

const ReceiptList = component$(() => {
  const loc = useLocation();
  const receipts = useSignal<ReceiptInfo[]>([]);
  const suppliers = useSignal<SupplierChoice[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    ORDER_TABLES,
    $(async () => {
      try {
        receipts.value = unwrap(await commands.listPoReceipts());
        suppliers.value = unwrap(await commands.supplierOptions());
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "supplier", "location", "status"]);
    const q = f.q.toLowerCase();
    return receipts.value.filter(
      (r) =>
        (!q ||
          [r.no, r.po_no, r.supplier_name, r.summary, r.notes].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || r.date >= f.from) &&
        (!f.to || r.date <= f.to) &&
        (!f.supplier || r.supplier_id === f.supplier) &&
        (!f.location || r.location_id === f.location) &&
        (!f.status || (f.status === "void") === r.voided),
    );
  });
  const pg = usePage(shown);

  return (
    <>
      <PageHeader
        title="Receiving"
        subtitle="Goods received on purchase orders. Receive goods from the PO's page."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search number, PO, supplier, items"
        dates
        selects={[
          {
            key: "supplier",
            label: "Suppliers",
            options: suppliers.value.map((s) => ({ value: s.id, label: s.label })),
          },
          {
            key: "location",
            label: "Locations",
            options: locations.value.map((l) => ({ value: l.id, label: l.name })),
          },
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "posted", label: "Posted" },
              { value: "void", label: "Void" },
            ],
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{receipts.value.length ? "No receiving reports match." : "Nothing received yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Number</th>
                <th class={th}>Date</th>
                <th class={th}>PO</th>
                <th class={th}>Supplier</th>
                <th class={th}>Into</th>
                <th class={th}>Items</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((r) => (
                <tr key={r.id} class={r.voided ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/orders/receiving/?id=${r.id}`}>
                      {r.no ?? "—"}
                    </Link>
                    <OfflineBadge show={!r.no_confirmed} />
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(r.date)}</td>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="text-brand-700 hover:underline" href={`/orders/?id=${r.po_id}`}>
                      {r.po_no ?? "—"}
                    </Link>
                  </td>
                  <td class={td}>{r.supplier_name ?? "—"}</td>
                  <td class={td}>{r.location_name ?? "—"}</td>
                  <td class={td}>{r.summary}</td>
                  <td class={td}>{r.voided ? <Badge tone="danger">Void</Badge> : <Badge tone="brand">Posted</Badge>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
    </>
  );
});

const ReceiptView = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const detail = useSignal<ReceiptDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const voidOpen = useSignal(false);
  const reason = useSignal("");
  const voidError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ORDER_TABLES,
    $(async () => {
      try {
        const d = unwrap(await commands.getPoReceipt(id));
        detail.value = d;
        missing.value = !d;
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  if (missing.value) {
    return (
      <>
        <PageHeader title="Receiving report not found" />
        <Link class="text-sm text-brand-700 hover:underline" href="/orders/receiving/">
          Back to receiving
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const r = det.receipt;
  const rows: [string, string][] = [
    ["Supplier", r.supplier_name ?? ""],
    ["Date received", formatDay(r.date)],
    ["Into", r.location_name ?? "Deleted location"],
    ["Notes", r.notes ?? ""],
    ["Posted", `${formatDateTime(r.created_at)} by ${r.created_by_name ?? "—"}`],
  ];
  if (r.voided) {
    rows.push(["Voided", `${formatDateTime(r.voided_at)} by ${r.voided_by_name ?? "—"}`]);
    rows.push(["Reason", r.void_reason ?? ""]);
  }
  const boxed = det.lines.some((l) => l.box_id);

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/orders/receiving/">
          ← Receiving
        </Link>
      </p>
      <PageHeader title={`Receiving report ${r.no ?? ""}`} subtitle={r.supplier_name ?? undefined}>
        {!r.voided && can(session.user, "orders", "delete") && (
          <button
            q:slot="actions"
            class={btnSecondary}
            onClick$={() => {
              reason.value = "";
              voidError.value = "";
              voidOpen.value = true;
            }}
          >
            Void
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 flex flex-wrap items-center gap-3">
        {r.voided ? <Badge tone="danger">Void</Badge> : <Badge tone="brand">Posted</Badge>}
        <OfflineBadge show={!r.no_confirmed} />
        <span class="text-sm">
          On PO{" "}
          <Link class="text-brand-700 hover:underline" href={`/orders/?id=${r.po_id}`}>
            {r.po_no ?? "—"}
          </Link>
        </span>
      </div>
      <div class="grid gap-4 xl:grid-cols-[22rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <InfoList rows={rows} />
        </section>
        <section class={[card, "min-w-0 self-start overflow-x-auto"]}>
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Item</th>
                {boxed && <th class={th}>Box</th>}
                <th class={th}>Expiry</th>
                <th class={[th, "text-right"]}>Qty</th>
              </tr>
            </thead>
            <tbody>
              {det.lines.map((l) => (
                <tr key={l.id}>
                  <td class={td}>
                    <Link class="text-brand-700 hover:underline" href={`/inventory/?item=${l.item_id}`}>
                      {l.item_code ?? "Deleted item"}
                    </Link>{" "}
                    {l.item_name}
                  </td>
                  {boxed && (
                    <td class={[td, "whitespace-nowrap font-mono"]}>{l.box_id ? (l.serial ?? "no serial") : "—"}</td>
                  )}
                  <td class={[td, "whitespace-nowrap"]}>{l.expiry ? formatDay(l.expiry) : "—"}</td>
                  <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>{formatQty(l.qty, l.unit)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {r.voided && <p class={[hint, "px-3 py-2"]}>Void: these goods don't count as received or in stock.</p>}
        </section>
      </div>
      <Modal open={voidOpen} title={`Void ${r.no ?? "this report"}?`}>
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            voidError.value = "";
            try {
              unwrap(await commands.voidPoReceipt(id, reason.value));
              voidOpen.value = false;
            } catch (e) {
              voidError.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          {voidError.value && <Alert>{voidError.value}</Alert>}
          <p class="text-sm">
            The goods come out of stock again and count as not received on the PO. Refused if the stock was
            already used. Enter the receipt again if it was wrong.
          </p>
          <Field label="Reason">
            <input class={input} required value={reason.value} onInput$={(_, el) => (reason.value = el.value)} />
          </Field>
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (voidOpen.value = false)}>
              Back
            </button>
            <button class={btnDanger} disabled={busy.value}>
              Void
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

export default component$(() => {
  const loc = useLocation();
  const id = loc.url.searchParams.get("id");
  return id ? <ReceiptView key={id} id={id} /> : <ReceiptList />;
});

export const head: DocumentHead = { title: "Receiving · Sanco" };
