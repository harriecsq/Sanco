// One form for entering a DR or invoice and for editing one in any status:
// client, items, quantity, price and lot. A client's order starts here.
import {
  $,
  component$,
  useSignal,
  useStore,
  useVisibleTask$,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import {
  commands,
  type Choice,
  type DrDetail,
  type ItemChoice,
  type LocationInfo,
  type NumberModes,
  type PriceQuote,
  type PriceSource,
  type ReceiptKind,
  type StockLot,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatPeso,
  formatQty,
  lotText,
  moneyText,
  parseCount,
  parseMoney,
  todayManila,
} from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { RULE_LABELS } from "~/components/rates/rates";
import { CreditWarning } from "~/components/accounting/accounting";
import { PRICE_SOURCE_LABEL, RECEIPT, Totals } from "~/components/sales/sales";
import { Alert, Field, Modal } from "~/components/ui/ui";
import {
  btnGhost,
  btnPrimary,
  btnSecondary,
  hint,
  input,
  inputBase,
} from "~/components/ui/styles";

interface Row {
  key: number;
  /** The receipt line being edited, if any. */
  id: string | null;
  item_id: string;
  qty: string;
  /** Unit price as typed; blank uses the client's price. */
  price: string;
  notes: string;
  /** The lot, picked by hand. Needed once the receipt is delivered. */
  lot_id: string;
  /** Saved price and its source, while editing a line. */
  saved: { price: number; rule: PriceSource } | null;
}

interface Form {
  kind: ReceiptKind;
  client_id: string;
  date: string;
  location_id: string;
  client_ref: string;
  deliver_to: string;
  notes: string;
  /** The booklet number as typed. */
  no: string;
  delivered_date: string;
  received_by: string;
  rows: Row[];
}

/** A stock lot; `own` marks a lot the edited receipt already uses. */
type Lot = StockLot & { own?: boolean };

let nextKey = 1;
const newKey = () => nextKey++;

const blankRow = (item_id = "", price = ""): Row => ({
  key: newKey(),
  id: null,
  item_id,
  qty: "",
  price,
  notes: "",
  lot_id: "",
  saved: null,
});

const quoteKey = (client: string, item: string) => `${client}|${item}`;

/** A typed count, or 0 while it isn't one. */
const countOf = (text: string) => {
  try {
    return parseCount(text) ?? 0;
  } catch {
    return 0;
  }
};

/** A typed price, or null while blank or not one. */
const priceOf = (text: string) => {
  try {
    return parseMoney(text);
  } catch {
    return null;
  }
};

/** Lots of an item at a location, oldest first. */
const lotsFor = (lots: Lot[], item: string, location: string) =>
  lots
    .filter(
      (s) =>
        s.item_id === item && s.location_id === location && (s.qty > 0 || s.own),
    )
    .sort((a, b) => (a.month ?? "").localeCompare(b.month ?? ""));

const lotLabel = (l: StockLot) => `${lotText(l)} · ${formatQty(l.qty, l.unit)}`;

/**
 * Enters a DR or invoice (without `dr`) or edits one (with it), in any
 * status: also after delivery, and when void.
 */
export const ReceiptFormModal = component$<{
  open: Signal<boolean>;
  /** The type a new receipt starts as. */
  kind?: ReceiptKind;
  dr?: DrDetail | null;
  saved$?: QRL<(id: string, kind: ReceiptKind) => void>;
}>(({ open, kind: startKind, dr, saved$ }) => {
  const clients = useSignal<Choice[]>([]);
  const items = useSignal<ItemChoice[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const lots = useSignal<Lot[]>([]);
  const modes = useSignal<NumberModes>({
    dr_manual: true,
    invoice_manual: true,
  });
  const quotes = useStore<Record<string, PriceQuote | null>>({});
  const error = useSignal("");
  const busy = useSignal(false);
  const form = useStore<Form>({
    kind: "dr",
    client_id: "",
    date: "",
    location_id: "",
    client_ref: "",
    deliver_to: "",
    notes: "",
    no: "",
    delivered_date: "",
    received_by: "",
    rows: [],
  });

  useLiveLoad(
    ["clients", "items", "locations", "doc_numbers", "settings"],
    $(async () => {
      try {
        clients.value = unwrap(await commands.clientOptions());
        items.value = unwrap(await commands.itemOptions());
        modes.value = unwrap(await commands.receiptNumberModes());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  // Fill the form each time it opens.
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      if (!track(() => open.value)) return;
      error.value = "";
      let stock: Lot[] = [];
      let locs: LocationInfo[] = [];
      try {
        locs = unwrap(await commands.listLocations()).filter(
          (x) => x.active || x.id === dr?.dr.from_location_id,
        );
        locations.value = locs;
        stock = unwrap(await commands.receiptLots());
      } catch (e) {
        error.value = errorText(e);
      }
      if (dr) {
        // The receipt's own lots stay pickable; a delivered one's stock
        // counts as available again.
        const at = dr.dr.from_location_id ?? "";
        for (const l of dr.lines) {
          let lot = stock.find(
            (x) => x.lot_id === l.lot_id && x.location_id === at,
          );
          if (!lot) {
            lot = {
              item_id: l.item_id,
              item_code: l.item_code ?? "",
              item_name: l.item_name ?? "",
              unit: l.unit,
              category: null,
              location_id: at,
              location_name: dr.dr.from_location_name,
              lot_id: l.lot_id,
              month: l.month,
              expiry: l.expiry,
              days_left: null,
              serial_from: l.serial_from,
              serial_to: l.serial_to,
              qty: 0,
            };
            stock.push(lot);
          }
          lot.own = true;
          if (dr.dr.status === "delivered") lot.qty += l.qty;
        }
      }
      lots.value = stock;
      if (dr) {
        form.kind = dr.dr.kind;
        form.client_id = dr.dr.client_id ?? "";
        form.date = dr.dr.date;
        form.location_id = dr.dr.from_location_id ?? "";
        form.client_ref = dr.dr.client_ref ?? "";
        form.deliver_to = dr.dr.deliver_to ?? "";
        form.notes = dr.dr.notes ?? "";
        form.no = dr.dr.no ?? "";
        form.delivered_date = dr.dr.delivered_date ?? todayManila();
        form.received_by = dr.dr.received_by ?? "";
        form.rows = dr.lines.map((l) => ({
          key: newKey(),
          id: l.id,
          item_id: l.item_id,
          qty: String(l.qty),
          price: moneyText(l.price_c),
          notes: l.notes ?? "",
          lot_id: l.lot_id ?? "",
          saved: { price: l.price_c, rule: l.price_rule },
        }));
        return;
      }
      // Stock comes from the Office first.
      const first = locs.find((x) => x.is_office) ?? locs[0];
      form.kind = startKind ?? "dr";
      form.client_id = "";
      form.date = todayManila();
      form.location_id = first?.id ?? "";
      form.client_ref = "";
      form.deliver_to = "";
      form.notes = "";
      form.no = "";
      form.delivered_date = "";
      form.received_by = "";
      form.rows = [blankRow()];
    },
    { strategy: "document-ready" },
  );

  // Fetch the client's price for every item on the form.
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      if (!track(() => open.value)) return;
      const client = track(() => form.client_id);
      const wanted = track(() => form.rows.map((r) => r.item_id)).filter(
        Boolean,
      );
      if (!client) return;
      for (const item of wanted) {
        const k = quoteKey(client, item);
        if (k in quotes) continue;
        quotes[k] = null;
        const q = await commands.priceQuote(client, item);
        if (q.status === "ok") quotes[k] = q.data;
      }
    },
    { strategy: "document-ready" },
  );

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const lines = form.rows
        .filter((r) => r.item_id || r.qty.trim())
        .map((r) => {
          const what =
            items.value.find((i) => i.id === r.item_id)?.label ?? "an item";
          if (!r.item_id)
            throw new Error("choose an item on every line, or remove it");
          const qty = parseCount(r.qty, `quantity for ${what}`);
          if (!qty) throw new Error(`enter a quantity for ${what}`);
          return {
            id: r.id,
            item_id: r.item_id,
            qty,
            price_c: parseMoney(r.price, `price for ${what}`),
            lot_id: r.lot_id || null,
            notes: r.notes,
          };
        });
      const delivery = dr?.dr.delivered_at
        ? { date: form.delivered_date, received_by: form.received_by }
        : null;
      const id = unwrap(
        await commands.saveReceipt(
          dr?.dr.id ?? null,
          {
            client_id: form.client_id,
            kind: form.kind,
            date: form.date,
            from_location_id: form.location_id,
            client_ref: form.client_ref,
            deliver_to: form.deliver_to,
            notes: form.notes,
            lines,
          },
          delivery,
          form.no.trim() || null,
        ),
      );
      open.value = false;
      if (saved$) await saved$(id, form.kind);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const pickable = <T extends { id: string; active: boolean }>(
    list: T[],
    current: string,
  ) => list.filter((o) => o.active || o.id === current);

  const k = RECEIPT[form.kind];
  const delivered = dr?.dr.status === "delivered";
  const wasDelivered = !!dr?.dr.delivered_at;
  const manual =
    form.kind === "dr" ? modes.value.dr_manual : modes.value.invoice_manual;
  const wantedSeries = form.kind === "dr" ? "DR" : "SI";
  // A switched type frees the number it had, unless it was in this booklet.
  const freed =
    !!dr && !!dr.dr.no && dr.dr.no_series !== wantedSeries && form.kind !== dr.dr.kind;
  const clientChanged =
    !!dr &&
    dr.payments.length > 0 &&
    form.client_id !== (dr.dr.client_id ?? "");

  let net = 0;
  for (const r of form.rows) {
    const typed = priceOf(r.price);
    const quoted = quotes[quoteKey(form.client_id, r.item_id)]?.net_c ?? 0;
    net += countOf(r.qty) * (typed ?? quoted);
  }
  // The price typed is final. Only an older delivered invoice keeps VAT.
  const vat =
    wasDelivered && form.kind === "invoice" && dr
      ? Math.round((net * dr.dr.vat_bp) / 10000)
      : 0;
  const title = dr
    ? `Edit ${RECEIPT[dr.dr.kind].title} ${dr.dr.no ?? ""}`.trim()
    : `New ${RECEIPT[startKind ?? "dr"].name}`;

  return (
    <Modal open={open} title={title} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        {delivered && (
          <Alert tone="warning">
            This {k.name} was delivered: changing its items, lots or quantities
            moves the difference in or out of stock.
          </Alert>
        )}
        {dr?.dr.status === "void" && (
          <Alert tone="warning">
            This {k.name} is void. Edits are saved; restore it to put it back
            in use.
          </Alert>
        )}
        {clientChanged && (
          <Alert tone="warning">
            Payments are applied to this {k.name}. If you change the client,
            they stay on it and the Dashboard flags them until they are fixed.
          </Alert>
        )}
        {form.client_id && !dr && (
          <CreditWarning clientId={form.client_id} extraC={net + vat} />
        )}

        <fieldset>
          <legend class="mb-1 text-sm font-medium">The client gets</legend>
          <div class="flex flex-wrap gap-2">
            {(["dr", "invoice"] as const).map((kind) => (
              <label
                key={kind}
                class={[
                  "flex cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-sm",
                  form.kind === kind
                    ? "border-brand-500 bg-brand-50"
                    : "border-line",
                ]}
              >
                <input
                  type="radio"
                  name="receipt-kind"
                  class="accent-brand-500"
                  checked={form.kind === kind}
                  onChange$={() => {
                    form.kind = kind;
                    // The number belongs to one booklet: it is freed when the
                    // type moves to the other, and back if it moves back.
                    if (dr?.dr.no)
                      form.no =
                        dr.dr.no_series === (kind === "dr" ? "DR" : "SI")
                          ? dr.dr.no
                          : "";
                  }}
                />
                <span class="font-medium">
                  {kind === "dr" ? "Delivery Receipt" : "Sales Invoice"}
                </span>
              </label>
            ))}
          </div>
          {freed && (
            <p class={[hint, "mt-1"]}>
              Its number {dr?.dr.no} is freed: enter the number from the{" "}
              {k.booklet}
              {delivered ? " below" : " when you have it"}.
            </p>
          )}
        </fieldset>

        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Client">
            <select
              class={input}
              required
              onChange$={(_, el) => (form.client_id = el.value)}
            >
              <option value="">Choose a client…</option>
              {pickable(clients.value, form.client_id).map((c) => (
                <option
                  key={c.id}
                  value={c.id}
                  selected={c.id === form.client_id}
                >
                  {c.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label={wasDelivered ? "Planned for" : "Deliver on"}>
            <input
              type="date"
              class={input}
              required
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label="From">
            <select
              class={input}
              required
              onChange$={(_, el) => {
                form.location_id = el.value;
                // Lots chosen at the old location don't apply.
                form.rows.forEach((r) => (r.lot_id = ""));
              }}
            >
              {locations.value.map((l) => (
                <option
                  key={l.id}
                  value={l.id}
                  selected={l.id === form.location_id}
                >
                  {l.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Client's PO / reference">
            <input
              class={input}
              value={form.client_ref}
              onInput$={(_, el) => (form.client_ref = el.value)}
            />
          </Field>
          {wasDelivered && (
            <Field label="Delivered on">
              <input
                type="date"
                class={input}
                required
                max={todayManila()}
                value={form.delivered_date}
                onChange$={(_, el) => (form.delivered_date = el.value)}
              />
            </Field>
          )}
          {wasDelivered && (
            <Field label="Received by">
              <input
                class={input}
                value={form.received_by}
                onInput$={(_, el) => (form.received_by = el.value)}
              />
            </Field>
          )}
          {manual && (
            <Field
              label={`${k.title} number (BIR booklet)`}
              hint={
                delivered
                  ? "Needed on a delivered receipt. Digits only, as printed."
                  : "Digits only, as printed. Can be entered later, but is needed to deliver."
              }
            >
              <input
                class={input}
                inputMode="numeric"
                required={delivered}
                value={form.no}
                onInput$={(_, el) => (form.no = el.value)}
              />
            </Field>
          )}
          <Field label="Deliver to" class="sm:col-span-2">
            <input
              class={input}
              placeholder="Client's address"
              value={form.deliver_to}
              onInput$={(_, el) => (form.deliver_to = el.value)}
            />
          </Field>
        </div>

        <div>
          <div class="mb-1 flex items-center justify-between">
            <span class="text-sm font-medium">Items</span>
            <span class={hint}>
              Prices are per unit and final. Blank uses the client's price.{" "}
              {delivered ? "A lot is needed on every line." : "Pick the lot when you have it; it is needed to deliver."}
            </span>
          </div>
          <div class="space-y-2">
            {form.rows.map((row, idx) => {
              const q =
                form.client_id && row.item_id
                  ? quotes[quoteKey(form.client_id, row.item_id)]
                  : null;
              const typed = priceOf(row.price);
              const kept =
                typed != null && row.saved?.price === typed
                  ? row.saved.rule
                  : null;
              const custom = typed != null && q?.net_c !== typed;
              const below =
                typed != null && q?.cost_c != null && typed < q.cost_c;
              const avail = lotsFor(lots.value, row.item_id, form.location_id);
              const onHand =
                avail.find((s) => s.lot_id === row.lot_id)?.qty ?? 0;
              const short = !!row.lot_id && countOf(row.qty) > onHand;
              return (
                <div
                  key={row.key}
                  class="grid grid-cols-[minmax(0,1fr)_14rem_5rem_8rem_auto] items-start gap-2"
                >
                  <select
                    data-search="on"
                    class={input}
                    aria-label="Item"
                    onChange$={(_, el) => {
                      row.item_id = el.value;
                      row.lot_id = "";
                      row.saved = null;
                    }}
                  >
                    <option value="">Choose an item…</option>
                    {pickable(items.value, row.item_id).map((i) => (
                      <option
                        key={i.id}
                        value={i.id}
                        selected={i.id === row.item_id}
                      >
                        {i.label}
                      </option>
                    ))}
                  </select>
                  <select
                    class={input}
                    aria-label="Lot"
                    required={delivered}
                    onChange$={(_, el) => (row.lot_id = el.value)}
                  >
                    <option value="" selected={!row.lot_id}>
                      {!row.item_id
                        ? "Lot"
                        : avail.length
                          ? "Choose a lot…"
                          : "None in stock here"}
                    </option>
                    {avail.map((s) => (
                      <option
                        key={s.lot_id ?? ""}
                        value={s.lot_id ?? ""}
                        selected={s.lot_id === row.lot_id}
                      >
                        {lotLabel(s)}
                      </option>
                    ))}
                  </select>
                  <div>
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label="Quantity"
                      inputMode="numeric"
                      placeholder="Qty"
                      value={row.qty}
                      onInput$={(_, el) => (row.qty = el.value)}
                    />
                    {short && (
                      <span class="block text-right text-xs text-danger">
                        only {onHand}
                      </span>
                    )}
                  </div>
                  <div class="space-y-0.5">
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label="Unit price"
                      inputMode="decimal"
                      placeholder={
                        q?.net_c != null ? moneyText(q.net_c) : "₱ price"
                      }
                      value={row.price}
                      onInput$={(_, el) => (row.price = el.value)}
                    />
                    {q && (
                      <span
                        class={[
                          "block text-right text-xs",
                          below || (!custom && q.below_cost)
                            ? "text-danger"
                            : "text-ink-muted",
                        ]}
                      >
                        {kept
                          ? PRICE_SOURCE_LABEL[kept]
                          : custom
                            ? "Typed in"
                            : RULE_LABELS[q.rule]}
                        {(below || (!custom && q.below_cost)) &&
                          " · below cost"}
                      </span>
                    )}
                  </div>
                  <div class="flex">
                    <button
                      type="button"
                      class={btnGhost}
                      title="Take part from another lot"
                      disabled={!row.item_id}
                      onClick$={() =>
                        form.rows.splice(
                          idx + 1,
                          0,
                          blankRow(row.item_id, row.price),
                        )
                      }
                    >
                      + lot
                    </button>
                    <button
                      type="button"
                      class={btnGhost}
                      aria-label="Remove line"
                      disabled={form.rows.length === 1}
                      onClick$={() => form.rows.splice(idx, 1)}
                    >
                      ✕
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
          <button
            type="button"
            class={[btnGhost, "mt-2"]}
            onClick$={() => form.rows.push(blankRow())}
          >
            + Add item
          </button>
        </div>

        <Field label="Notes">
          <textarea
            class={input}
            rows={2}
            value={form.notes}
            onInput$={(_, el) => (form.notes = el.value)}
          />
        </Field>
        <div class="flex flex-wrap items-start justify-between gap-4">
          <p class={hint}>
            {formatPeso(net + vat)} in total.{" "}
            {!dr && !manual
              ? `The ${k.name} number is given automatically.`
              : ""}
          </p>
          <Totals
            totals={{ net_c: net, vat_c: vat, gross_c: net + vat }}
            vat={form.kind === "invoice"}
            vatBp={dr?.dr.vat_bp ?? 0}
          />
        </div>
        <div class="flex justify-end gap-2">
          <button
            type="button"
            class={btnSecondary}
            onClick$={() => (open.value = false)}
          >
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value}>
            {dr ? "Save" : `Enter ${k.name}`}
          </button>
        </div>
      </form>
    </Modal>
  );
});
