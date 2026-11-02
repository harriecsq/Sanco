import {
  $,
  component$,
  useComputed$,
  useSignal,
  useStore,
} from "@builder.io/qwik";
import {
  Link,
  useLocation,
  useNavigate,
  type DocumentHead,
} from "@builder.io/qwik-city";
import {
  commands,
  type ItemStock,
  type LocationInfo,
  type LocationQty,
  type StockKind,
  type StockLot,
  type StockMove,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatMonth, formatQty, serialSpan } from "~/lib/format";
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
import { receiptHref } from "~/components/sales/sales";
import { Pager, usePage } from "~/components/ui/pager";
import { Alert, Badge, Empty, PageHeader } from "~/components/ui/ui";
import {
  btnPrimary,
  btnSecondary,
  card,
  hint,
  table,
  td,
  th,
} from "~/components/ui/styles";

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

/** An item's stock at one location (zero when it has none there). */
function atLocation(s: ItemStock, location_id: string): LocationQty {
  return (
    s.by_location.find((b) => b.location_id === location_id) ?? {
      location_id,
      qty: 0,
      next_expiry: null,
    }
  );
}

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
      "lots",
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

  // With a location picked, the list shows only what is at that location.
  const at = useComputed$(() => filtersOf(loc.url, ["location"]).location);
  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "location", "category", "status"]);
    const q = f.q.toLowerCase();
    return stock.value.filter((s) => {
      const qty = f.location ? atLocation(s, f.location).qty : s.on_hand;
      const negative = f.location ? qty < 0 : s.negative;
      return (
        (!q ||
          [s.code, s.name, s.category].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.category || s.category === f.category) &&
        (f.status === "all"
          ? true
          : f.status === "low"
            ? s.low
            : f.status === "negative"
              ? negative
              : f.status === "none"
                ? s.active && qty === 0
                : qty !== 0 || negative)
      );
    });
  });
  const pg = usePage(shown);
  const subtitle = useComputed$(() => {
    const l = locations.value.find((l) => l.id === at.value);
    return l
      ? `What is at ${l.name}, by item, from posted documents.`
      : "By item and location, from posted documents.";
  });

  // Active locations, plus inactive ones still holding stock.
  const cols = locations.value.filter(
    (l) =>
      l.active ||
      stock.value.some((s) =>
        s.by_location.some((b) => b.location_id === l.id),
      ),
  );
  const one = cols.find((l) => l.id === at.value);
  const categories = [
    ...new Set(stock.value.map((s) => s.category).filter(Boolean)),
  ] as string[];
  const negatives = stock.value.filter((s) => s.negative).length;

  return (
    <>
      <PageHeader title="Stock on hand" subtitle={subtitle.value}>
        <div q:slot="actions" class="flex flex-wrap gap-2">
          <NewDocButtons />
        </div>
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      {negatives > 0 && (
        <div class="mb-4">
          <Alert tone="warning">
            {negatives} item(s) show negative stock, usually because two PCs
            took the same stock while offline. Count them and post an
            adjustment.
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
                {one ? (
                  <th class={[th, "text-right"]}>At {one.name}</th>
                ) : (
                  cols.map((l) => (
                    <th key={l.id} class={[th, "text-right"]}>
                      {l.name}
                    </th>
                  ))
                )}
                <th class={[th, "text-right"]}>
                  {one ? "All locations" : "Total"}
                </th>
                <th class={[th, "text-right"]}>Reorder at</th>
                <th class={th}>Next expiry</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((s) => {
                const b = one && atLocation(s, one.id);
                const expiry = b ? b.next_expiry : s.next_expiry;
                return (
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
                    {b ? (
                      <td
                        class={[
                          td,
                          "text-right font-medium whitespace-nowrap tabular-nums",
                          b.qty < 0 && "text-danger",
                          b.qty === 0 && "text-ink-muted",
                        ]}
                      >
                        {b.qty === 0 ? "—" : formatQty(b.qty, s.unit)}
                      </td>
                    ) : (
                      cols.map((l) => {
                        const q = atLocation(s, l.id).qty;
                        return (
                          <td
                            key={l.id}
                            class={[
                              td,
                              "text-right tabular-nums",
                              q < 0 && "text-danger",
                              q === 0 && "text-ink-muted",
                            ]}
                          >
                            {q === 0 ? "—" : formatQty(q)}
                          </td>
                        );
                      })
                    )}
                    <td
                      class={[
                        td,
                        "text-right whitespace-nowrap tabular-nums",
                        one ? "text-ink-muted" : "font-medium",
                      ]}
                    >
                      {formatQty(s.on_hand, s.unit)}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>
                      {s.reorder_level == null
                        ? "—"
                        : formatQty(s.reorder_level)}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>
                      {expiry ? formatDay(expiry) : "—"}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
    </>
  );
});

const MOVE_LABEL: Record<StockKind, string> = {
  receipt: "Receipt",
  adjustment: "Adjustment",
  transfer: "Transfer",
  delivery: "Delivered",
  po_receipt: "Received on PO",
};

/** One location's lots of an item, with its total. */
interface LocationLots {
  location_id: string;
  location_name: string | null;
  unit: string | null;
  qty: number;
  lots: StockLot[];
}

const ItemStockDetail = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const item = useSignal<ItemStock | null>(null);
  const missing = useSignal(false);
  const lots = useSignal<StockLot[]>([]);
  const moves = useSignal<StockMove[]>([]);
  const error = useSignal("");

  useLiveLoad(
    [
      "items",
      "locations",
      "stock_docs",
      "stock_doc_lines",
      "po_receipts",
      "delivery_receipts",
      "lots",
      "doc_numbers",
    ],
    $(async () => {
      try {
        const all = unwrap(await commands.stockOverview());
        item.value = all.find((s) => s.item_id === id) ?? null;
        missing.value = !item.value;
        lots.value = unwrap(await commands.stockLots(id));
        moves.value = unwrap(await commands.itemStockMoves(id));
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );
  // Lots arrive in location order (sort, name), so each location's lots
  // are already together.
  const byLocation = useComputed$(() => {
    const out: LocationLots[] = [];
    for (const l of lots.value) {
      let g = out.at(-1);
      if (!g || g.location_id !== l.location_id) {
        g = {
          location_id: l.location_id,
          location_name: l.location_name,
          unit: l.unit,
          qty: 0,
          lots: [],
        };
        out.push(g);
      }
      g.qty += l.qty;
      g.lots.push(l);
    }
    return out;
  });
  // Two paged tables on one screen, so each has its own page key.
  const lotsPg = usePage(byLocation, undefined, "lots_page");
  const movesPg = usePage(moves, undefined, "moves_page");

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
        subtitle={`${formatQty(s.on_hand, s.unit)} on hand${s.reorder_level == null ? "" : ` · reorder at ${formatQty(s.reorder_level)}`}`}
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
            Some stock is negative, usually because two PCs took the same stock
            while offline. Count it and post an adjustment.
          </Alert>
        )}
        {s.low && (
          <Alert tone="warning">
            This item is at or below its reorder level.
          </Alert>
        )}
      </div>
      <div class="grid gap-4 xl:grid-cols-[26rem_minmax(0,1fr)]">
        <div class="space-y-4 self-start">
          <section class={[card, "overflow-x-auto"]}>
            <h2 class="border-b border-line px-4 py-3 font-semibold">
              On hand
            </h2>
            {lots.value.length === 0 ? (
              <Empty>None in stock.</Empty>
            ) : (
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>Location / lot</th>
                    <th class={[th, "text-right"]}>Qty</th>
                  </tr>
                </thead>
                {lotsPg.value.rows.map((g) => (
                  <tbody key={g.location_id}>
                    <tr class="bg-brand-50">
                      <td class={[td, "font-semibold"]}>
                        {g.location_name ?? "Deleted location"} total
                      </td>
                      <td
                        class={[
                          td,
                          "text-right font-semibold whitespace-nowrap tabular-nums",
                          g.qty < 0 && "text-danger",
                        ]}
                      >
                        {formatQty(g.qty, g.unit)}
                      </td>
                    </tr>
                    {g.lots.map((l) => (
                      <tr key={`${l.location_id}/${l.lot_id}`}>
                        <td class={[td, "pl-8"]}>
                          {formatMonth(l.month)}
                          {serialSpan(l.serial_from, l.serial_to) && (
                            <span class={[hint, "block"]}>
                              {serialSpan(l.serial_from, l.serial_to)}
                            </span>
                          )}
                          {l.expiry && (
                            <span class="block whitespace-nowrap text-xs">
                              Exp. {formatDay(l.expiry)}{" "}
                              <ExpiryBadge days={l.days_left} />
                            </span>
                          )}
                        </td>
                        <td
                          class={[
                            td,
                            "text-right whitespace-nowrap tabular-nums",
                            l.qty < 0 && "text-danger",
                          ]}
                        >
                          {formatQty(l.qty, l.unit)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                ))}
              </table>
            )}
            <Pager of={lotsPg} />
          </section>
        </div>
        <section class={[card, "min-w-0 overflow-x-auto"]}>
          <div class="border-b border-line px-4 py-3">
            <h2 class="font-semibold">Movements</h2>
            <p class={hint}>
              Posted documents only; voided ones no longer count.
            </p>
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
                  <th class={th}>Lot</th>
                  <th class={th}>Serial #</th>
                  <th class={th}>Expiry</th>
                  <th class={[th, "text-right"]}>Change</th>
                </tr>
              </thead>
              <tbody>
                {movesPg.value.rows.map((m, i) => (
                  <tr key={`${m.doc_id}/${i}`}>
                    <td class={[td, "whitespace-nowrap"]}>
                      {formatDay(m.date)}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link
                        class="text-brand-700 hover:underline"
                        href={
                          m.kind === "delivery"
                            ? receiptHref(
                                m.invoice ? "invoice" : "dr",
                                m.doc_id,
                              )
                            : m.kind === "po_receipt"
                              ? `/orders/receiving/?id=${m.doc_id}`
                              : `/inventory/movements/?id=${m.doc_id}`
                        }
                      >
                        {m.no ?? KIND_LABEL[m.kind]}
                      </Link>
                      <span class={[hint, "block"]}>
                        {m.kind === "delivery"
                          ? m.invoice
                            ? "Delivered invoice"
                            : "Delivered DR"
                          : MOVE_LABEL[m.kind]}
                      </span>
                    </td>
                    <td class={td}>
                      {m.location_name ?? "Deleted location"}
                      {m.kind === "transfer" && (
                        <> → {m.to_location_name ?? "Deleted location"}</>
                      )}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>
                      {formatMonth(m.month)}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>
                      {serialSpan(m.serial_from, m.serial_to) || "—"}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>
                      {m.expiry ? formatDay(m.expiry) : "—"}
                    </td>
                    <td
                      class={[
                        td,
                        "text-right whitespace-nowrap tabular-nums",
                        m.kind === "transfer"
                          ? ""
                          : m.qty < 0
                            ? "text-danger"
                            : "text-brand-700",
                      ]}
                    >
                      {m.kind === "transfer" ? "" : m.qty > 0 ? "+" : "−"}
                      {formatQty(Math.abs(m.qty), s.unit)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <Pager of={movesPg} />
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
