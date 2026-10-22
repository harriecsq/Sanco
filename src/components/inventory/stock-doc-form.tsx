import { $, component$, useSignal, type QRL, type Signal } from "@builder.io/qwik";
import {
  commands,
  type ItemStock,
  type LocationInfo,
  type StockKind,
  type StockLot,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatQty, parseCount, parseSigned, todayManila } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Field, Modal } from "~/components/ui/ui";
import { btnGhost, btnPrimary, btnSecondary, hint, input, inputBase } from "~/components/ui/styles";

export const KIND_LABEL: Record<StockKind, string> = {
  receipt: "Stock receipt",
  adjustment: "Adjustment",
  transfer: "Transfer",
  delivery: "Delivery",
};

/** Lot picker values besides an expiry date. */
const NO_EXPIRY = "-";
const NEW_EXPIRY = "+new";

const REASONS = ["Stock count", "Damaged", "Expired", "Sample", "Found", "Returned"];

export interface DocLine {
  key: number;
  item_id: string;
  /** Receipts: unused. Others: an expiry date, NO_EXPIRY or NEW_EXPIRY. */
  lot: string;
  /** Receipts and new adjustment lots. */
  expiry: string;
  qty: string;
}

export interface DocForm {
  kind: StockKind;
  date: string;
  location_id: string;
  to_location_id: string;
  reason: string;
  notes: string;
  lines: DocLine[];
}

let nextKey = 1;
const blankLine = (item_id = ""): DocLine => ({
  key: nextKey++,
  item_id,
  lot: "",
  expiry: "",
  qty: "",
});

/** A new document. `location_id` empty means the first active location. */
export const blankDoc = (
  kind: StockKind,
  opts: { item_id?: string; location_id?: string } = {},
): DocForm => ({
  kind,
  date: todayManila(),
  location_id: opts.location_id ?? "",
  to_location_id: "",
  reason: "",
  notes: "",
  lines: [blankLine(opts.item_id)],
});

const lotKey = (expiry: string | null) => expiry ?? NO_EXPIRY;

/** Qwik input typings lack `list`; this links an input to a datalist. */
const datalist = (id: string) => ({ list: id }) as Record<string, string>;

interface Data {
  items: ItemStock[];
  lots: StockLot[];
  locations: LocationInfo[];
}

/** The location stock comes from (or into): the pick, else the first active one. */
const fromOf = (d: Data, form: DocForm) =>
  form.location_id || d.locations.find((l) => l.active)?.id || "";

/** Lots a line can take from: in stock (any non-zero lot for adjustments). */
const lotsAt = (d: Data, form: DocForm, item: string) => {
  const from = fromOf(d, form);
  return d.lots.filter(
    (l) =>
      l.item_id === item &&
      l.location_id === from &&
      (form.kind === "adjustment" ? l.qty !== 0 : l.qty > 0),
  );
};

/** The lot a line points at: its pick, else the first one available. */
const lotOf = (d: Data, form: DocForm, line: DocLine) => {
  const item = d.items.find((i) => i.item_id === line.item_id);
  const available = lotsAt(d, form, line.item_id);
  if (
    line.lot &&
    (line.lot === NEW_EXPIRY || available.some((l) => lotKey(l.expiry) === line.lot))
  )
    return line.lot;
  if (available.length) return lotKey(available[0].expiry);
  return form.kind === "adjustment" && item?.tracks_expiry ? NEW_EXPIRY : NO_EXPIRY;
};

const lotLabel = (l: StockLot) =>
  `${l.expiry ? `Exp. ${formatDay(l.expiry)}` : "No expiry"} · ${formatQty(l.qty, l.unit)}`;

/** Receive, adjust or transfer stock. */
export const StockDocModal = component$<{
  open: Signal<boolean>;
  form: DocForm;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const data = useSignal<Data>({ items: [], lots: [], locations: [] });
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["items", "locations", "stock_docs", "stock_doc_lines", "delivery_receipts"],
    $(async () => {
      try {
        data.value = {
          items: unwrap(await commands.stockOverview()),
          lots: unwrap(await commands.stockLots(null)),
          locations: unwrap(await commands.listLocations()),
        };
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );


  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const d = data.value;
      const kind = form.kind;
      const lines = form.lines
        .filter((l) => l.item_id || l.qty.trim())
        .map((l) => {
          const item = d.items.find((i) => i.item_id === l.item_id);
          if (!item) throw new Error("choose an item on every line");
          const qty =
            kind === "adjustment"
              ? parseSigned(l.qty, `change for ${item.code}`)
              : parseCount(l.qty, `quantity for ${item.code}`);
          if (!qty) throw new Error(`enter a quantity for ${item.code}`);
          let expiry: string | null;
          if (kind === "receipt") expiry = l.expiry || null;
          else {
            const lot = lotOf(d, form, l);
            expiry = lot === NEW_EXPIRY ? l.expiry || null : lot === NO_EXPIRY ? null : lot;
          }
          return { item_id: l.item_id, expiry, qty };
        });
      const id = unwrap(
        await commands.postStockDoc({
          kind,
          date: form.date,
          location_id: fromOf(d, form),
          to_location_id: kind === "transfer" ? form.to_location_id || null : null,
          reason: kind === "adjustment" ? form.reason : "",
          notes: form.notes,
          lines,
        }),
      );
      open.value = false;
      if (saved$) await saved$(id);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const d = data.value;
  const kind = form.kind;
  const from = fromOf(d, form);
  const shownLocations = d.locations.filter(
    (l) => l.active || l.id === from || l.id === form.to_location_id,
  );
  const pickable = d.items.filter(
    (i) => i.active || form.lines.some((l) => l.item_id === i.item_id),
  );

  return (
    <Modal open={open} title={`New ${KIND_LABEL[kind].toLowerCase()}`} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid gap-3 sm:grid-cols-3">
          <Field label="Date">
            <input
              type="date"
              class={input}
              required
              max={todayManila()}
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label={kind === "receipt" ? "Received at" : kind === "transfer" ? "From" : "Location"}>
            <select
              class={input}
              required
              onChange$={(_, el) => {
                form.location_id = el.value;
                form.lines.forEach((l) => (l.lot = ""));
              }}
            >
              {shownLocations.map((l) => (
                <option key={l.id} value={l.id} selected={l.id === from}>
                  {l.name}
                </option>
              ))}
            </select>
          </Field>
          {kind === "transfer" && (
            <Field label="To">
              <select
                class={input}
                required
                onChange$={(_, el) => (form.to_location_id = el.value)}
              >
                <option value="">Choose…</option>
                {shownLocations
                  .filter((l) => l.id !== from)
                  .map((l) => (
                    <option key={l.id} value={l.id} selected={l.id === form.to_location_id}>
                      {l.name}
                    </option>
                  ))}
              </select>
            </Field>
          )}
          {kind === "adjustment" && (
            <Field label="Reason">
              <input
                class={input}
                required
                {...datalist("stock-reasons")}
                value={form.reason}
                onInput$={(_, el) => (form.reason = el.value)}
              />
              <datalist id="stock-reasons">
                {REASONS.map((r) => (
                  <option key={r} value={r} />
                ))}
              </datalist>
            </Field>
          )}
        </div>

        <div>
          <div class="mb-1 flex items-center justify-between">
            <span class="text-sm font-medium">Items</span>
            <span class={hint}>
              {kind === "adjustment"
                ? "Enter the change: +5 adds, −5 removes."
                : kind === "transfer"
                  ? "Choose the lot (expiry date) to move."
                  : "Quantities are in each item's base unit."}
            </span>
          </div>
          <div class="space-y-2">
            {form.lines.map((line, idx) => {
              const item = d.items.find((i) => i.item_id === line.item_id);
              const available = lotsAt(d, form, line.item_id);
              const lot = lotOf(d, form, line);
              const onHand =
                lot === NEW_EXPIRY
                  ? 0
                  : (available.find((l) => lotKey(l.expiry) === lot)?.qty ?? 0);
              let after: number | null = null;
              try {
                const q =
                  kind === "adjustment" ? parseSigned(line.qty) : kind === "transfer" ? parseCount(line.qty) : null;
                if (q != null) after = kind === "adjustment" ? onHand + q : onHand - q;
              } catch {
                after = null;
              }
              return (
                <div
                  key={line.key}
                  class="grid grid-cols-[minmax(0,1fr)_14rem_7rem_auto] items-start gap-2"
                >
                  <select
                    class={input}
                    aria-label="Item"
                    onChange$={(_, el) => {
                      line.item_id = el.value;
                      line.lot = "";
                      line.expiry = "";
                    }}
                  >
                    <option value="">Choose an item…</option>
                    {pickable.map((i) => (
                      <option key={i.item_id} value={i.item_id} selected={i.item_id === line.item_id}>
                        {`${i.code} · ${i.name}`}
                      </option>
                    ))}
                  </select>
                  <div class="space-y-1">
                    {kind === "receipt" ? (
                      item?.tracks_expiry ? (
                        <input
                          type="date"
                          class={input}
                          aria-label="Expiry date"
                          required
                          value={line.expiry}
                          onChange$={(_, el) => (line.expiry = el.value)}
                        />
                      ) : (
                        <span class={[hint, "block pt-2"]}>{item ? "No expiry" : ""}</span>
                      )
                    ) : (
                      <>
                        <select
                          class={input}
                          aria-label="Lot"
                          disabled={!item}
                          onChange$={(_, el) => (line.lot = el.value)}
                        >
                          {available.map((l) => (
                            <option key={lotKey(l.expiry)} value={lotKey(l.expiry)} selected={lotKey(l.expiry) === lot}>
                              {lotLabel(l)}
                            </option>
                          ))}
                          {kind === "adjustment" && item?.tracks_expiry && (
                            <option value={NEW_EXPIRY} selected={lot === NEW_EXPIRY}>
                              New expiry date…
                            </option>
                          )}
                          {kind === "adjustment" && item && !item.tracks_expiry && !available.length && (
                            <option value={NO_EXPIRY}>No expiry · 0</option>
                          )}
                          {kind === "transfer" && item && !available.length && (
                            <option value="">None here</option>
                          )}
                        </select>
                        {lot === NEW_EXPIRY && (
                          <input
                            type="date"
                            class={input}
                            aria-label="Expiry date"
                            required
                            value={line.expiry}
                            onChange$={(_, el) => (line.expiry = el.value)}
                          />
                        )}
                      </>
                    )}
                  </div>
                  <div class="space-y-1">
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label={kind === "adjustment" ? "Change" : "Quantity"}
                      inputMode={kind === "adjustment" ? "text" : "numeric"}
                      placeholder={kind === "adjustment" ? "+/−" : "Qty"}
                      value={line.qty}
                      onInput$={(_, el) => (line.qty = el.value)}
                    />
                    {item && after != null && (
                      <span class={["block text-right text-xs", after < 0 ? "text-danger" : "text-ink-muted"]}>
                        {formatQty(onHand)} → {formatQty(after, item.unit)}
                      </span>
                    )}
                  </div>
                  <button
                    type="button"
                    class={btnGhost}
                    aria-label="Remove line"
                    disabled={form.lines.length === 1}
                    onClick$={() => form.lines.splice(idx, 1)}
                  >
                    ✕
                  </button>
                </div>
              );
            })}
          </div>
          <button type="button" class={[btnGhost, "mt-2"]} onClick$={() => form.lines.push(blankLine())}>
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
        <p class={hint}>
          Posted documents can't be edited. To correct one, void it and enter it again.
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value}>
            Post
          </button>
        </div>
      </form>
    </Modal>
  );
});
