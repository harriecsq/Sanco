import { $, component$, useSignal, type QRL, type Signal } from "@builder.io/qwik";
import {
  commands,
  type BoxStock,
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
  po_receipt: "PO receipt",
};

/** Lot picker values besides an expiry date. */
const NO_EXPIRY = "-";
const NEW_EXPIRY = "+new";
/** Box picker value for stock outside boxes. */
const LOOSE = "-";

const REASONS = ["Stock count", "Damaged", "Expired", "Sample", "Found", "Returned"];

export interface DocLine {
  key: number;
  item_id: string;
  /** Receipts: unused. Others: an expiry date, NO_EXPIRY or NEW_EXPIRY. */
  lot: string;
  /** Receipts and new adjustment lots. */
  expiry: string;
  /** Pieces. Boxed receipts: the loose pieces on top of the boxes. */
  qty: string;
  /** Boxed items, not receipts: a box id or LOOSE; blank = the first box. */
  box_id: string;
  /** Boxed receipts: full boxes and their size (blank = the item's). */
  boxes: string;
  per_box: string;
}

export interface DocForm {
  kind: StockKind;
  date: string;
  location_id: string;
  to_location_id: string;
  reason: string;
  notes: string;
  /** Receipts of boxed items: the year in the new serials. */
  serial_year: string;
  lines: DocLine[];
}

let nextKey = 1;
const blankLine = (item_id = ""): DocLine => ({
  key: nextKey++,
  item_id,
  lot: "",
  expiry: "",
  qty: "",
  box_id: "",
  boxes: "",
  per_box: "",
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
  serial_year: todayManila().slice(0, 4),
  lines: [blankLine(opts.item_id)],
});

const lotKey = (expiry: string | null) => expiry ?? NO_EXPIRY;

/** Qwik input typings lack `list`; this links an input to a datalist. */
const datalist = (id: string) => ({ list: id }) as Record<string, string>;

interface Data {
  items: ItemStock[];
  lots: StockLot[];
  boxes: BoxStock[];
  locations: LocationInfo[];
}

/** The location stock comes from (or into): the pick, else the first active one. */
const fromOf = (d: Data, form: DocForm) =>
  form.location_id || d.locations.find((l) => l.active)?.id || "";

const boxed = (d: Data, item: string) =>
  !!d.items.find((i) => i.item_id === item)?.serial_code;

/** Boxes of an item at the line's location, lowest serial first. */
const boxesAt = (d: Data, form: DocForm, item: string) => {
  const from = fromOf(d, form);
  return d.boxes.filter((b) => b.item_id === item && b.location_id === from);
};

/** Lots a line can take from: in stock (any non-zero lot for adjustments).
 * For boxed items only the pieces outside boxes count. */
const lotsAt = (d: Data, form: DocForm, item: string) => {
  const from = fromOf(d, form);
  const lots = d.lots.filter((l) => l.item_id === item && l.location_id === from);
  if (boxed(d, item)) {
    const boxes = boxesAt(d, form, item);
    return lots
      .map((l) => ({
        ...l,
        qty: l.qty - boxes.filter((b) => b.expiry === l.expiry).reduce((s, b) => s + b.qty, 0),
      }))
      .filter((l) => l.qty > 0);
  }
  return lots.filter((l) => (form.kind === "adjustment" ? l.qty !== 0 : l.qty > 0));
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

/** The box a non-receipt line of a boxed item points at: its pick, else the
 * first box, else stock outside boxes (LOOSE). Blank when there's neither. */
const boxOf = (d: Data, form: DocForm, line: DocLine) => {
  const boxes = boxesAt(d, form, line.item_id);
  const loose = lotsAt(d, form, line.item_id).length > 0;
  if (line.box_id === LOOSE && loose) return LOOSE;
  if (line.box_id && boxes.some((b) => b.box_id === line.box_id)) return line.box_id;
  if (boxes.length) return boxes[0].box_id;
  return loose ? LOOSE : "";
};

const lotLabel = (l: StockLot) =>
  `${l.expiry ? `Exp. ${formatDay(l.expiry)}` : "No expiry"} · ${formatQty(l.qty, l.unit)}`;

/** "MT26-0002 · 7 of 10 left · exp. Jan 31, 2027" */
const boxLabel = (b: BoxStock) =>
  [
    b.serial ?? "Box",
    b.qty === b.pcs ? `full (${formatQty(b.pcs)})` : `${formatQty(b.qty)} of ${formatQty(b.pcs)} left`,
    b.expiry ? `exp. ${formatDay(b.expiry)}` : "",
  ]
    .filter(Boolean)
    .join(" · ");

/** A boxed receipt line: total pieces and the boxes they make. Null while
 * the input isn't valid or there's no box size. */
const receiptBoxes = (item: ItemStock | undefined, line: DocLine) => {
  try {
    const boxes = parseCount(line.boxes) ?? 0;
    const loose = parseCount(line.qty) ?? 0;
    const per = parseCount(line.per_box) ?? item?.pcs_per_box ?? null;
    if (!per) return null;
    const total = boxes * per + loose;
    return { total, made: Math.ceil(total / per), rest: total % per };
  } catch {
    return null;
  }
};

/** Years offered for new serials: ten back, five ahead. */
const serialYears = () => {
  const y = Number(todayManila().slice(0, 4));
  return Array.from({ length: 16 }, (_, i) => String(y - 10 + i));
};

/** Receive, adjust or transfer stock. */
export const StockDocModal = component$<{
  open: Signal<boolean>;
  form: DocForm;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const data = useSignal<Data>({ items: [], lots: [], boxes: [], locations: [] });
  const error = useSignal("");
  const busy = useSignal(false);

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
        data.value = {
          items: unwrap(await commands.stockOverview()),
          lots: unwrap(await commands.stockLots(null)),
          boxes: unwrap(await commands.boxStock(null)),
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
        .filter((l) => l.item_id || l.qty.trim() || l.boxes.trim())
        .map((l) => {
          const item = d.items.find((i) => i.item_id === l.item_id);
          if (!item) throw new Error("choose an item on every line");
          if (kind === "receipt" && item.serial_code) {
            // Boxes × size + loose pieces; the backend splits them into boxes.
            const boxes = parseCount(l.boxes, `boxes for ${item.code}`) ?? 0;
            const loose = parseCount(l.qty, `loose pieces for ${item.code}`) ?? 0;
            const typed = parseCount(l.per_box, `pcs per box for ${item.code}`);
            if (typed === 0) throw new Error(`pcs per box for ${item.code} must be at least 1`);
            const per = typed ?? item.pcs_per_box;
            if (!per) throw new Error(`enter the pcs per box for ${item.code}`);
            const qty = boxes * per + loose;
            if (!qty) throw new Error(`enter the boxes for ${item.code}`);
            return { item_id: l.item_id, expiry: l.expiry || null, qty, box_id: null, pcs_per_box: per };
          }
          const qty =
            kind === "adjustment"
              ? parseSigned(l.qty, `change for ${item.code}`)
              : parseCount(l.qty, `quantity for ${item.code}`);
          if (!qty) throw new Error(`enter a quantity for ${item.code}`);
          if (kind !== "receipt" && item.serial_code) {
            const box = boxOf(d, form, l);
            if (!box) throw new Error(`${item.code} has no stock here`);
            if (box !== LOOSE) {
              // The backend takes the lot from the box.
              const expiry = d.boxes.find((b) => b.box_id === box)?.expiry ?? null;
              return { item_id: l.item_id, expiry, qty, box_id: box, pcs_per_box: null };
            }
          }
          let expiry: string | null;
          if (kind === "receipt") expiry = l.expiry || null;
          else {
            const lot = lotOf(d, form, l);
            expiry = lot === NEW_EXPIRY ? l.expiry || null : lot === NO_EXPIRY ? null : lot;
          }
          return { item_id: l.item_id, expiry, qty, box_id: null, pcs_per_box: null };
        });
      const id = unwrap(
        await commands.postStockDoc({
          kind,
          date: form.date,
          location_id: fromOf(d, form),
          to_location_id: kind === "transfer" ? form.to_location_id || null : null,
          reason: kind === "adjustment" ? form.reason : "",
          notes: form.notes,
          serial_year: kind === "receipt" ? form.serial_year : "",
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
  const boxedReceipt = kind === "receipt" && form.lines.some((l) => boxed(d, l.item_id));

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
                form.lines.forEach((l) => {
                  l.lot = "";
                  l.box_id = "";
                });
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
          {boxedReceipt && (
            <Field label="Serial year">
              <select class={input} onChange$={(_, el) => (form.serial_year = el.value)}>
                {serialYears().map((y) => (
                  <option key={y} value={y} selected={y === form.serial_year}>
                    {y}
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
                  ? "Choose the box or lot (expiry date) to move."
                  : boxedReceipt
                    ? "Boxed items: enter full boxes, their size and any loose pieces."
                    : "Quantities are in each item's base unit."}
            </span>
          </div>
          <div class="space-y-2">
            {form.lines.map((line, idx) => {
              const item = d.items.find((i) => i.item_id === line.item_id);
              const isBoxed = !!item?.serial_code;
              const boxLine = isBoxed && kind !== "receipt";
              const available = lotsAt(d, form, line.item_id);
              const boxes = boxLine ? boxesAt(d, form, line.item_id) : [];
              const box = boxLine ? boxOf(d, form, line) : "";
              const boxRow = boxes.find((b) => b.box_id === box);
              const left = boxRow?.qty ?? 0;
              const lot = lotOf(d, form, line);
              const onHand = boxRow
                ? boxRow.qty
                : lot === NEW_EXPIRY
                  ? 0
                  : (available.find((l) => lotKey(l.expiry) === lot)?.qty ?? 0);
              const made = kind === "receipt" && isBoxed ? receiptBoxes(item, line) : null;
              let after: number | null = null;
              try {
                const q =
                  kind === "adjustment" ? parseSigned(line.qty) : kind === "transfer" ? parseCount(line.qty) : null;
                if (q != null) after = kind === "adjustment" ? onHand + q : onHand - q;
              } catch {
                after = null;
              }
              const lotPicker = (
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
                    {kind === "adjustment" && item?.tracks_expiry && !isBoxed && (
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
              );
              return (
                <div
                  key={line.key}
                  class="grid grid-cols-[minmax(0,1fr)_14rem_7rem_auto] items-start gap-2"
                >
                  <select
                    data-search="on"
                    class={input}
                    aria-label="Item"
                    onChange$={(_, el) => {
                      line.item_id = el.value;
                      line.lot = "";
                      line.expiry = "";
                      line.box_id = "";
                      const size = data.value.items.find((i) => i.item_id === el.value)?.pcs_per_box;
                      line.per_box = size ? String(size) : "";
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
                    ) : boxLine ? (
                      <>
                        <select class={input} aria-label="Box" onChange$={(_, el) => (line.box_id = el.value)}>
                          {boxes.map((b) => (
                            <option key={b.box_id} value={b.box_id} selected={b.box_id === box}>
                              {boxLabel(b)}
                            </option>
                          ))}
                          {available.length > 0 && (
                            <option value={LOOSE} selected={box === LOOSE}>
                              Not in a box
                            </option>
                          )}
                          {!box && <option value="">No boxes here</option>}
                        </select>
                        {box === LOOSE && lotPicker}
                        {kind === "transfer" && left > 0 && (
                          <button
                            type="button"
                            class="text-xs text-brand-700 hover:underline"
                            onClick$={() => (line.qty = String(left))}
                          >
                            Whole box ({formatQty(left)})
                          </button>
                        )}
                      </>
                    ) : (
                      lotPicker
                    )}
                  </div>
                  <div class="space-y-1">
                    {kind === "receipt" && isBoxed ? (
                      <input
                        class={[inputBase, "w-full text-right"]}
                        aria-label="Boxes"
                        inputMode="numeric"
                        placeholder="Boxes"
                        value={line.boxes}
                        onInput$={(_, el) => (line.boxes = el.value)}
                      />
                    ) : (
                      <input
                        class={[inputBase, "w-full text-right"]}
                        aria-label={kind === "adjustment" ? "Change" : "Quantity"}
                        inputMode={kind === "adjustment" ? "text" : "numeric"}
                        placeholder={kind === "adjustment" ? "+/−" : "Qty"}
                        value={line.qty}
                        onInput$={(_, el) => (line.qty = el.value)}
                      />
                    )}
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
                  {kind === "receipt" && isBoxed && item && (
                    <div class="col-span-3 flex flex-wrap items-center justify-end gap-2">
                      <label class="flex items-center gap-1.5">
                        <span class={hint}>Pcs/box</span>
                        <input
                          class={[inputBase, "w-20 text-right"]}
                          inputMode="numeric"
                          placeholder={item.pcs_per_box ? String(item.pcs_per_box) : "Size"}
                          value={line.per_box}
                          onInput$={(_, el) => (line.per_box = el.value)}
                        />
                      </label>
                      <label class="flex items-center gap-1.5">
                        <span class={hint}>+ loose pcs</span>
                        <input
                          class={[inputBase, "w-20 text-right"]}
                          inputMode="numeric"
                          placeholder="0"
                          value={line.qty}
                          onInput$={(_, el) => (line.qty = el.value)}
                        />
                      </label>
                      <span class={[hint, "w-48 text-right"]}>
                        {made && made.total > 0
                          ? `= ${formatQty(made.total, item.unit)} · makes ${made.made} ${made.made === 1 ? "box" : "boxes"}${made.rest ? ` (last holds ${made.rest})` : ""}`
                          : ""}
                      </span>
                    </div>
                  )}
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
