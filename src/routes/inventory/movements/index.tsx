import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type LocationInfo,
  type StockDocDetail,
  type StockDocInfo,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDateTime, formatDay, formatQty } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { KIND_LABEL } from "~/components/inventory/stock-doc-form";
import { Alert, Badge, Empty, Field, InfoList, Modal, PageHeader } from "~/components/ui/ui";
import { btnDanger, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

/** "Warehouse → Office" for transfers, else the location. */
const where = (d: StockDocInfo) =>
  d.kind === "transfer"
    ? `${d.location_name ?? "?"} → ${d.to_location_name ?? "?"}`
    : (d.location_name ?? "Deleted location");

const DocNo = component$<{ d: StockDocInfo }>(({ d }) => (
  <>
    {d.no ?? "—"}
    {!d.no_confirmed && (
      <span class="ml-2" title="Given while offline from other PCs; it may still change.">
        <Badge tone="warning">offline no.</Badge>
      </span>
    )}
  </>
));

const DocList = component$(() => {
  const loc = useLocation();
  const docs = useSignal<StockDocInfo[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    ["stock_docs", "stock_doc_lines", "items", "locations", "doc_numbers", "users"],
    $(async () => {
      try {
        docs.value = unwrap(await commands.listStockDocs());
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "kind", "location", "status"]);
    const q = f.q.toLowerCase();
    return docs.value.filter(
      (d) =>
        (!q ||
          [d.no, d.summary, d.reason, d.notes, d.created_by_name].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.from || d.date >= f.from) &&
        (!f.to || d.date <= f.to) &&
        (!f.kind || d.kind === f.kind) &&
        (!f.location || d.location_id === f.location || d.to_location_id === f.location) &&
        (!f.status || (f.status === "void") === d.voided),
    );
  });

  return (
    <>
      <PageHeader
        title="Stock movements"
        subtitle="Receipts, adjustments and transfers. New ones start from the Stock tab."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search number, items, reason"
        dates
        selects={[
          {
            key: "kind",
            label: "Types",
            options: (["receipt", "adjustment", "transfer"] as const).map((k) => ({
              value: k,
              label: KIND_LABEL[k],
            })),
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
          <Empty>{docs.value.length ? "No documents match." : "No stock movements yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Number</th>
                <th class={th}>Date</th>
                <th class={th}>Type</th>
                <th class={th}>Location</th>
                <th class={th}>Items</th>
                <th class={th}>By</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((d) => (
                <tr key={d.id} class={d.voided ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/inventory/movements/?id=${d.id}`}
                    >
                      <DocNo d={d} />
                    </Link>
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(d.date)}</td>
                  <td class={td}>
                    {KIND_LABEL[d.kind]}
                    {d.reason && <span class={[hint, "block"]}>{d.reason}</span>}
                  </td>
                  <td class={td}>{where(d)}</td>
                  <td class={td}>{d.summary}</td>
                  <td class={td}>{d.created_by_name ?? "—"}</td>
                  <td class={td}>
                    <Badge tone={d.voided ? "muted" : "brand"}>{d.voided ? "Void" : "Posted"}</Badge>
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

const DocDetail = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const detail = useSignal<StockDocDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const voidOpen = useSignal(false);
  const reason = useSignal("");
  const voidError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["stock_docs", "stock_doc_lines", "items", "locations", "doc_numbers", "users"],
    $(async () => {
      try {
        const d = unwrap(await commands.getStockDoc(id));
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
        <PageHeader title="Document not found" />
        <Link class="text-sm text-brand-700 hover:underline" href="/inventory/movements/">
          Back to movements
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const d = det.doc;

  const rows: [string, string][] = [
    ["Type", KIND_LABEL[d.kind]],
    ["Date", formatDay(d.date)],
    [d.kind === "transfer" ? "Moved" : d.kind === "receipt" ? "Received at" : "Location", where(d)],
  ];
  if (d.kind === "adjustment") rows.push(["Reason", d.reason ?? ""]);
  rows.push(["Notes", d.notes ?? ""]);
  rows.push(["Posted", `${formatDateTime(d.created_at)} by ${d.created_by_name ?? "—"}`]);
  if (d.voided) {
    rows.push(["Voided", `${formatDateTime(d.voided_at)} by ${d.voided_by_name ?? "—"}`]);
    rows.push(["Void reason", d.void_reason ?? ""]);
  }

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/inventory/movements/">
          ← Movements
        </Link>
      </p>
      <PageHeader title={`${KIND_LABEL[d.kind]} ${d.no ?? ""}`}>
        {!d.voided && can(session.user, "inventory", "delete") && (
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
      <div class="mb-4 space-y-2">
        {d.voided && <Alert tone="warning">This document is void: its stock no longer counts.</Alert>}
        {!d.no_confirmed && (
          <Alert tone="warning">
            This number was given while other PCs were offline, so it may still change.
          </Alert>
        )}
      </div>
      <div class="grid gap-4 xl:grid-cols-[24rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <InfoList rows={rows} />
        </section>
        <section class={card}>
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Item</th>
                <th class={th}>Expiry</th>
                <th class={[th, "text-right"]}>{d.kind === "adjustment" ? "Change" : "Qty"}</th>
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
                  <td class={[td, "whitespace-nowrap"]}>{l.expiry ? formatDay(l.expiry) : "—"}</td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {d.kind === "adjustment" && l.qty > 0 ? "+" : ""}
                    {formatQty(l.qty, l.unit)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>

      <Modal open={voidOpen} title={`Void ${d.no ?? "document"}?`}>
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            voidError.value = "";
            try {
              unwrap(await commands.voidStockDoc(id, reason.value));
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
            Its stock movements stop counting on every PC. The document stays in the list, marked
            void. This can't be undone; enter it again if needed.
          </p>
          <Field label="Reason">
            <input
              class={input}
              required
              value={reason.value}
              onInput$={(_, el) => (reason.value = el.value)}
            />
          </Field>
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (voidOpen.value = false)}>
              Cancel
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
  return id ? <DocDetail key={id} id={id} /> : <DocList />;
});

export const head: DocumentHead = { title: "Stock movements · Sanco" };
