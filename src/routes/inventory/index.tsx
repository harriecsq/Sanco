import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type BoxStock,
  type ItemStock,
  type LocationInfo,
  type StockKind,
  type StockLot,
  type StockMove,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { boxSummary, formatDay, formatQty } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  blankDoc,
  KIND_LABEL,
  StockDocModal,
  type DocForm,
} from "~/components/inventory/stock-doc-form";
import { ExpiryBadge } from "~/components/inventory/expiry-badge";
import { Alert, Badge, Empty, PageHeader } from "~/components/ui/ui";
import { btnPrimary, btnSecondary, card, hint, table, td, th } from "~/components/ui/styles";

/** "Receive", "Adjust" and "Transfer" buttons that open the document form. */
const NewDocButtons = component$<{ item_id?: string }>(({ item_id }) => {
  const session = useSession();
  const nav = useNavigate();
  const form = useStore<DocForm>(blankDoc("receipt"));
  const open = useSignal(false);
  if (!can(session.user, "inventory", "create")) return null;
  const start = $((kind: StockKind) => {
    Object.assign(form, blankDoc(kind, { item_id }));
    open.value = true;
  });
  return (
    <>
      <button class={btnPrimary} onClick$={() => start("receipt")}>
        Receive stock
      </button>
      <button class={btnSecondary} onClick$={() => start("adjustment")}>
        Adjust
      </button>
      <button class={btnSecondary} onClick$={() => start("transfer")}>
        Transfer
      </button>
      <StockDocModal
        open={open}
        form={form}
        saved$={$((id: string) => nav(`/inventory/movements/?id=${id}`))}
      />
    </>
  );
});

const StockList = component$(() => {
  const loc = useLocation();
  const stock = useSignal<ItemStock[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    [
      "items",
      "locations",
      "stock_docs",
      "stock_doc_lines",
      "po_receipts",
      "delivery_receipts",
      "boxes",
      "dr_line_boxes",
    ],
    $(async () => {
      try {
        stock.value = unwrap(await commands.stockOverview());
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "location", "category", "status"]);
    const q = f.q.toLowerCase();
    return stock.value.filter(
      (s) =>
        (!q || [s.code, s.name, s.category].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.category || s.category === f.category) &&
        (!f.location || s.by_location.some((l) => l.location_id === f.location)) &&
        (f.status === "all"
          ? true
          : f.status === "low"
            ? s.low
            : f.status === "negative"
              ? s.negative
              : f.status === "none"
                ? s.active && s.on_hand === 0
                : s.on_hand !== 0 || s.negative),
    );
  });

  // Active locations, plus inactive ones still holding stock.
  const cols = locations.value.filter(
    (l) => l.active || stock.value.some((s) => s.by_location.some((b) => b.location_id === l.id)),
  );
  const categories = [...new Set(stock.value.map((s) => s.category).filter(Boolean))] as string[];
  const negatives = stock.value.filter((s) => s.negative).length;

  return (
    <>
      <PageHeader title="Stock on hand" subtitle="By item and location, from posted documents.">
        <div q:slot="actions" class="flex flex-wrap gap-2">
          <NewDocButtons />
        </div>
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      {negatives > 0 && (
        <div class="mb-4">
          <Alert tone="warning">
            {negatives} item(s) show negative stock, usually because two PCs took the same stock
            while offline. Count them and post an adjustment.
          </Alert>
        </div>
      )}
      <FilterBar
        search="Search code, name, category"
        selects={[
          {
            key: "location",
            label: "Locations",
            options: cols.map((l) => ({ value: l.id, label: l.name })),
          },
          {
            key: "category",
            label: "Categories",
            options: categories.sort().map((c) => ({ value: c, label: c })),
          },
          {
            key: "status",
            label: "Status",
            allLabel: "In stock",
            options: [
              { value: "all", label: "All items" },
              { value: "low", label: "Low stock" },
              { value: "none", label: "Out of stock" },
              { value: "negative", label: "Negative" },
            ],
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>
            {stock.value.some((s) => s.on_hand !== 0)
              ? "No items match."
              : "No stock yet. Receive stock to get started."}
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Code</th>
                <th class={th}>Name</th>
                {cols.map((l) => (
                  <th key={l.id} class={[th, "text-right"]}>
                    {l.name}
                  </th>
                ))}
                <th class={[th, "text-right"]}>Total</th>
                <th class={[th, "text-right"]}>Reorder at</th>
                <th class={th}>Next expiry</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((s) => (
                <tr key={s.item_id} class={s.active ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/inventory/?item=${s.item_id}`}
                    >
                      {s.code}
                    </Link>
                  </td>
                  <td class={td}>
                    {s.name}
                    {s.low && (
                      <span class="ml-2">
                        <Badge tone="warning">low</Badge>
                      </span>
                    )}
                    {s.negative && (
                      <span class="ml-2">
                        <Badge tone="danger">negative</Badge>
                      </span>
                    )}
                  </td>
                  {cols.map((l) => {
                    const q = s.by_location.find((b) => b.location_id === l.id)?.qty ?? 0;
                    return (
                      <td
                        key={l.id}
                        class={[td, "text-right tabular-nums", q < 0 && "text-danger", q === 0 && "text-ink-muted"]}
                      >
                        {q === 0 ? "—" : formatQty(q)}
                      </td>
                    );
                  })}
                  <td class={[td, "text-right font-medium whitespace-nowrap tabular-nums"]}>
                    {formatQty(s.on_hand, s.unit)}
                    {s.serial_code && (s.full_boxes > 0 || s.open_boxes > 0) && (
                      <span class={[hint, "block font-normal"]}>
                        {boxSummary(s.full_boxes, s.open_boxes, s.open_pcs)}
                      </span>
                    )}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {s.reorder_level == null ? "—" : formatQty(s.reorder_level)}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {s.next_expiry ? formatDay(s.next_expiry) : "—"}
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

const MOVE_LABEL: Record<StockKind, string> = {
  receipt: "Receipt",
  adjustment: "Adjustment",
  transfer: "Transfer",
  delivery: "Delivered DR",
  po_receipt: "Received on PO",
};

const ItemStockDetail = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const item = useSignal<ItemStock | null>(null);
  const missing = useSignal(false);
  const lots = useSignal<StockLot[]>([]);
  const moves = useSignal<StockMove[]>([]);
  const boxes = useSignal<BoxStock[]>([]);
  const error = useSignal("");

  useLiveLoad(
    [
      "items",
      "locations",
      "stock_docs",
      "stock_doc_lines",
      "po_receipts",
      "delivery_receipts",
      "boxes",
      "dr_line_boxes",
      "doc_numbers",
    ],
    $(async () => {
      try {
        const all = unwrap(await commands.stockOverview());
        item.value = all.find((s) => s.item_id === id) ?? null;
        missing.value = !item.value;
        lots.value = unwrap(await commands.stockLots(id));
        moves.value = unwrap(await commands.itemStockMoves(id));
        boxes.value = item.value?.serial_code ? unwrap(await commands.boxStock(id)) : [];
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  if (missing.value) {
    return (
      <>
        <PageHeader title="Item not found" />
        <p class="text-sm">
          It may have been deleted on another PC.{" "}
          <Link class="text-brand-700 hover:underline" href="/inventory/">
            Back to stock
          </Link>
        </p>
      </>
    );
  }
  const s = item.value;
  if (!s) return error.value ? <Alert>{error.value}</Alert> : null;

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/inventory/">
          ← Stock
        </Link>
      </p>
      <PageHeader
        title={`${s.code} · ${s.name}`}
        subtitle={`${formatQty(s.on_hand, s.unit)} on hand${s.full_boxes || s.open_boxes ? ` (${boxSummary(s.full_boxes, s.open_boxes, s.open_pcs)})` : ""}${s.reorder_level == null ? "" : ` · reorder at ${formatQty(s.reorder_level)}`}${s.serial_code ? ` · serial ${s.serial_code}${s.pcs_per_box ? `, ${formatQty(s.pcs_per_box)} a box` : ""}` : ""}`}
      >
        <div q:slot="actions" class="flex flex-wrap gap-2">
          {can(session.user, "items") && (
            <Link class={btnSecondary} href={`/items/?id=${id}`}>
              Item details
            </Link>
          )}
          <NewDocButtons item_id={id} />
        </div>
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 space-y-2">
        {s.negative && (
          <Alert tone="warning">
            Some stock is negative, usually because two PCs took the same stock while offline.
            Count it and post an adjustment.
          </Alert>
        )}
        {s.low && <Alert tone="warning">This item is at or below its reorder level.</Alert>}
      </div>
      <div class="grid gap-4 xl:grid-cols-[26rem_minmax(0,1fr)]">
        <div class="space-y-4 self-start">
          <section class={card}>
            <h2 class="border-b border-line px-4 py-3 font-semibold">On hand</h2>
            {lots.value.length === 0 ? (
              <Empty>None in stock.</Empty>
            ) : (
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>Location</th>
                    <th class={th}>Expiry</th>
                    <th class={[th, "text-right"]}>Qty</th>
                  </tr>
                </thead>
                <tbody>
                  {lots.value.map((l) => (
                    <tr key={`${l.location_id}/${l.expiry}`}>
                      <td class={td}>{l.location_name ?? "Deleted location"}</td>
                      <td class={[td, "whitespace-nowrap"]}>
                        {l.expiry ? (
                          <>
                            {formatDay(l.expiry)} <ExpiryBadge days={l.days_left} />
                          </>
                        ) : (
                          <span class="text-ink-muted">No expiry</span>
                        )}
                      </td>
                      <td class={[td, "text-right whitespace-nowrap tabular-nums", l.qty < 0 && "text-danger"]}>
                        {formatQty(l.qty, l.unit)}
                        {(l.full_boxes > 0 || l.open_boxes > 0) && (
                          <span class={[hint, "block"]}>
                            {boxSummary(l.full_boxes, l.open_boxes, l.open_pcs)}
                          </span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>
          {s.serial_code && (
            <section class={[card, "overflow-x-auto"]}>
              <div class="border-b border-line px-4 py-3">
                <h2 class="font-semibold">Boxes</h2>
                <p class={hint}>In stock, lowest serial first.</p>
              </div>
              {boxes.value.length === 0 ? (
                <Empty>No boxes in stock.</Empty>
              ) : (
                <table class={table}>
                  <thead>
                    <tr>
                      <th class={th}>Serial</th>
                      <th class={th}>Location</th>
                      <th class={[th, "text-right"]}>Left</th>
                      <th class={th}>Expiry</th>
                      <th class={th}>PO</th>
                    </tr>
                  </thead>
                  <tbody>
                    {boxes.value.map((b) => (
                      <tr key={b.box_id}>
                        <td class={[td, "whitespace-nowrap font-medium"]}>
                          {b.serial ?? "—"}
                          {b.open && (
                            <span class="ml-2">
                              <Badge>open</Badge>
                            </span>
                          )}
                        </td>
                        <td class={td}>{b.location_name ?? "Deleted location"}</td>
                        <td class={[td, "text-right whitespace-nowrap tabular-nums", b.qty < 0 && "text-danger"]}>
                          {formatQty(b.qty)} / {formatQty(b.pcs)}
                        </td>
                        <td class={[td, "whitespace-nowrap"]}>
                          {b.expiry ? (
                            <>
                              {formatDay(b.expiry)} <ExpiryBadge days={b.days_left} />
                            </>
                          ) : (
                            <span class="text-ink-muted">—</span>
                          )}
                        </td>
                        <td class={[td, "whitespace-nowrap"]}>
                          {b.po_id ? (
                            <Link class="text-brand-700 hover:underline" href={`/orders/?id=${b.po_id}`}>
                              {b.po_no ?? "PO"}
                            </Link>
                          ) : (
                            <span class="text-ink-muted">—</span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
          )}
        </div>
        <section class={card}>
          <div class="border-b border-line px-4 py-3">
            <h2 class="font-semibold">Movements</h2>
            <p class={hint}>Posted documents only; voided ones no longer count.</p>
          </div>
          {moves.value.length === 0 ? (
            <Empty>No movements yet.</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>Date</th>
                  <th class={th}>Document</th>
                  <th class={th}>Location</th>
                  <th class={th}>Expiry</th>
                  {s.serial_code && <th class={th}>Box</th>}
                  <th class={[th, "text-right"]}>Change</th>
                </tr>
              </thead>
              <tbody>
                {moves.value.map((m, i) => (
                  <tr key={`${m.doc_id}/${i}`}>
                    <td class={[td, "whitespace-nowrap"]}>{formatDay(m.date)}</td>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link
                        class="text-brand-700 hover:underline"
                        href={
                          m.kind === "delivery"
                            ? `/accounting/?id=${m.doc_id}`
                            : m.kind === "po_receipt"
                              ? `/orders/receiving/?id=${m.doc_id}`
                              : `/inventory/movements/?id=${m.doc_id}`
                        }
                      >
                        {m.no ?? KIND_LABEL[m.kind]}
                      </Link>{" "}
                      <span class={hint}>{MOVE_LABEL[m.kind]}</span>
                    </td>
                    <td class={td}>{m.location_name ?? "Deleted location"}</td>
                    <td class={[td, "whitespace-nowrap"]}>{m.expiry ? formatDay(m.expiry) : "—"}</td>
                    {s.serial_code && <td class={[td, "whitespace-nowrap"]}>{m.serial ?? "—"}</td>}
                    <td
                      class={[
                        td,
                        "text-right whitespace-nowrap tabular-nums",
                        m.qty < 0 ? "text-danger" : "text-brand-700",
                      ]}
                    >
                      {m.qty > 0 ? "+" : "−"}
                      {formatQty(Math.abs(m.qty), s.unit)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      </div>
    </>
  );
});

export default component$(() => {
  const loc = useLocation();
  const id = loc.url.searchParams.get("item");
  return id ? <ItemStockDetail key={id} id={id} /> : <StockList />;
});

export const head: DocumentHead = { title: "Stock · Sanco" };
