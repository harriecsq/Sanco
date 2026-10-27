import { $, component$, useSignal, type QRL, type Signal } from "@builder.io/qwik";
import {
  commands,
  type Choice,
  type LocationInfo,
  type PoDetail,
  type PoStatus,
  type ReceiveProgress,
  type SupplierChoice,
  type TtInfo,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatMinor,
  formatPeso,
  formatQty,
  moneyText,
  parseCount,
  parseMoney,
  todayManila,
} from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Badge, Field, Modal } from "~/components/ui/ui";
import { BankAccountSelect } from "~/components/accounting/bank-account-select";
import { btnGhost, btnPrimary, btnSecondary, hint, input, inputBase } from "~/components/ui/styles";

type Tone = "brand" | "muted" | "danger" | "warning";

export const PO_STATUS: Record<PoStatus, { label: string; tone: Tone }> = {
  draft: { label: "Draft", tone: "warning" },
  ordered: { label: "Ordered", tone: "brand" },
  shipped: { label: "Shipped", tone: "brand" },
  arrived: { label: "Arrived", tone: "brand" },
  closed: { label: "Closed", tone: "muted" },
  cancelled: { label: "Cancelled", tone: "muted" },
};

export const RECEIVE_LABEL: Record<ReceiveProgress, string> = {
  none: "Not received",
  partial: "Partly received",
  full: "Received",
};

export const CURRENCIES = ["USD", "CNY", "TWD", "PHP"] as const;

/** Tables whose changes affect Order screens. */
export const ORDER_TABLES = [
  "purchase_orders",
  "po_lines",
  "supplier_payments",
  "po_receipts",
  "po_receipt_lines",
  "suppliers",
  "items",
  "locations",
  "doc_numbers",
  "users",
];

export const PoStatusBadge = component$<{ status: PoStatus }>(({ status }) => (
  <Badge tone={PO_STATUS[status].tone}>{PO_STATUS[status].label}</Badge>
));

/** "56.5" → PHP per unit, or null when blank or not a number. */
export const fxOf = (text: string | null | undefined): number | null => {
  const t = (text ?? "").trim().replace(/,/g, "");
  if (!t) return null;
  const v = Number(t);
  return Number.isFinite(v) && v > 0 ? v : null;
};

/** Minor units × FX in centavos, like the backend (half away from zero). */
export const phpOf = (minor: number, currency: string, fx: number | null) =>
  currency === "PHP" ? minor : fx == null ? null : Math.sign(minor) * Math.round(Math.abs(minor) * fx);

// PO form --------------------------------------------------------------------

/** What the PO form needs to know about an item that comes in boxes. */
interface BoxMeta {
  serial_code: string | null;
  pcs_per_box: number | null;
  unit: string | null;
}

interface FormLine {
  key: number;
  id: string | null;
  item_id: string;
  /** The item's serial code: set = entered as boxes + loose pieces. */
  serial_code: string | null;
  unit: string | null;
  /** Total pieces, for items without boxes. */
  qty: string;
  boxes: string;
  per_box: string;
  loose: string;
  price: string;
  notes: string;
}

export interface PoForm {
  id: string;
  supplier_id: string;
  date: string;
  currency: string;
  fx_rate: string;
  expected_date: string;
  supplier_ref: string;
  notes: string;
  /** `YYYY` in the box serials. */
  serial_year: string;
  /** Boxes already given serials; the year is fixed then. */
  box_count: number;
  lines: FormLine[];
}

let nextKey = 1;
const newKey = () => nextKey++;

const thisYear = () => todayManila().slice(0, 4);

const blankLine = (): FormLine => ({
  key: newKey(),
  id: null,
  item_id: "",
  serial_code: null,
  unit: null,
  qty: "",
  boxes: "",
  per_box: "",
  loose: "",
  price: "",
  notes: "",
});

/** Fill a line's quantity fields with `qty` pieces at `per` a box. */
const setQty = (line: FormLine, qty: number | null, per: number | null) => {
  line.per_box = per ? String(per) : "";
  if (qty == null) {
    line.boxes = line.loose = "";
    return;
  }
  const full = per ? Math.floor(qty / per) : 0;
  const rest = per ? qty % per : qty;
  line.qty = String(qty);
  line.boxes = full ? String(full) : "";
  line.loose = rest ? String(rest) : "";
};

/** Total pieces of a line in either entry mode; null when nothing is entered. */
const lineQty = (l: FormLine, what = "an item"): number | null => {
  if (!l.serial_code) return parseCount(l.qty, `quantity for ${what}`);
  const boxes = parseCount(l.boxes, `number of boxes for ${what}`);
  const per = parseCount(l.per_box, `pcs/box for ${what}`);
  const loose = parseCount(l.loose, `loose pieces for ${what}`);
  if (boxes == null && loose == null) return null;
  if (boxes && !per) throw new Error(`enter the pcs/box for ${what}`);
  return (boxes ?? 0) * (per ?? 0) + (loose ?? 0);
};

/** Change a line's item, keeping the quantity in pieces and switching
 *  between box and plain entry. */
const pickItem = (line: FormLine, id: string, meta: BoxMeta | undefined) => {
  let qty: number | null = null;
  try {
    qty = lineQty(line);
  } catch {
    qty = null;
  }
  line.item_id = id;
  line.serial_code = meta?.serial_code ?? null;
  line.unit = meta?.unit ?? null;
  setQty(line, qty, meta?.pcs_per_box ?? null);
};

export const blankPo = (): PoForm => ({
  id: "",
  supplier_id: "",
  date: todayManila(),
  currency: "USD",
  fx_rate: "",
  expected_date: "",
  supplier_ref: "",
  notes: "",
  serial_year: thisYear(),
  box_count: 0,
  lines: [blankLine()],
});

export const poToForm = (d: PoDetail): PoForm => ({
  id: d.po.id,
  supplier_id: d.po.supplier_id ?? "",
  date: d.po.date,
  currency: d.po.currency,
  fx_rate: d.po.fx_rate ?? "",
  expected_date: d.po.expected_date ?? "",
  supplier_ref: d.po.supplier_ref ?? "",
  notes: d.po.notes ?? "",
  serial_year: d.po.serial_year ?? thisYear(),
  box_count: d.po.box_count,
  lines: d.lines.map((l) => {
    const line: FormLine = {
      ...blankLine(),
      id: l.id,
      item_id: l.item_id,
      serial_code: l.serial_code,
      unit: l.unit,
      qty: String(l.qty),
      price: moneyText(l.price_minor),
      notes: l.notes ?? "",
    };
    if (l.serial_code) setQty(line, l.qty, l.pcs_per_box ?? l.item_pcs_per_box);
    return line;
  }),
});

/** Serial years to offer: ten back, five ahead, plus `keep`. */
const serialYears = (keep: string) => {
  const now = Number(thisYear());
  const years = Array.from({ length: 16 }, (_, i) => String(now - 10 + i));
  if (/^\d{4}$/.test(keep) && !years.includes(keep)) years.push(keep);
  return years.sort();
};

const pickable = <T extends { id: string; active: boolean }>(list: T[], current: string) =>
  list.filter((o) => o.active || o.id === current);

/** New or edit PO. `costs`: the user may see and set prices and FX. */
export const PoFormModal = component$<{
  open: Signal<boolean>;
  form: PoForm;
  costs: boolean;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, costs, saved$ }) => {
  const suppliers = useSignal<SupplierChoice[]>([]);
  const items = useSignal<Choice[]>([]);
  /** Box details by item id. */
  const boxMeta = useSignal<Record<string, BoxMeta>>({});
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["suppliers", "items", "doc_numbers"],
    $(async () => {
      try {
        suppliers.value = unwrap(await commands.supplierOptions());
        const list = unwrap(await commands.itemOptions());
        items.value = list;
        boxMeta.value = Object.fromEntries(
          list.map((i) => [
            i.id,
            { serial_code: i.serial_code, pcs_per_box: i.pcs_per_box, unit: i.unit },
          ]),
        );
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const lines = form.lines
        .filter((l) => l.item_id || l.qty.trim() || l.boxes.trim() || l.loose.trim())
        .map((l) => {
          const what = items.value.find((i) => i.id === l.item_id)?.label ?? "an item";
          if (!l.item_id) throw new Error("choose an item on every line, or remove it");
          const qty = lineQty(l, what);
          if (!qty) throw new Error(`enter a quantity for ${what}`);
          return {
            id: l.id,
            item_id: l.item_id,
            qty,
            price_minor: costs ? parseMoney(l.price, `price for ${what}`) : null,
            notes: l.notes,
            pcs_per_box: l.serial_code ? parseCount(l.per_box) || null : null,
          };
        });
      const id = unwrap(
        await commands.savePo({
          id: form.id || null,
          supplier_id: form.supplier_id,
          date: form.date,
          currency: form.currency,
          fx_rate: form.currency === "PHP" ? "" : form.fx_rate,
          expected_date: form.expected_date,
          supplier_ref: form.supplier_ref,
          notes: form.notes,
          serial_year: form.serial_year,
          lines,
        }),
      );
      busy.value = false;
      open.value = false;
      if (saved$) await saved$(id);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  let total = 0;
  for (const l of form.lines) {
    try {
      total += (lineQty(l) ?? 0) * (parseMoney(l.price) ?? 0);
    } catch {
      /* shown on save */
    }
  }
  const php = phpOf(total, form.currency, fxOf(form.fx_rate));
  const boxedCode = form.lines.find((l) => l.serial_code)?.serial_code;
  const yearFixed = !!form.id && form.box_count > 0;

  return (
    <Modal open={open} title={form.id ? "Edit purchase order" : "New purchase order"} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Supplier">
            <select
              class={input}
              required
              onChange$={(_, el) => {
                form.supplier_id = el.value;
                const s = suppliers.value.find((x) => x.id === el.value);
                if (s && !form.id) form.currency = s.currency;
              }}
            >
              <option value="">Choose a supplier…</option>
              {pickable(suppliers.value, form.supplier_id).map((s) => (
                <option key={s.id} value={s.id} selected={s.id === form.supplier_id}>
                  {s.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="PO date">
            <input
              type="date"
              class={input}
              required
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label="Supplier's PI / order no.">
            <input
              class={input}
              value={form.supplier_ref}
              onInput$={(_, el) => (form.supplier_ref = el.value)}
            />
          </Field>
          <Field label="Expected arrival">
            <input
              type="date"
              class={input}
              value={form.expected_date}
              onChange$={(_, el) => (form.expected_date = el.value)}
            />
          </Field>
          {costs && (
            <Field label="Currency">
              <select class={input} onChange$={(_, el) => (form.currency = el.value)}>
                {CURRENCIES.map((c) => (
                  <option key={c} value={c} selected={c === form.currency}>
                    {c}
                  </option>
                ))}
              </select>
            </Field>
          )}
          {costs && form.currency !== "PHP" && (
            <Field label={`Exchange rate (₱ per ${form.currency})`} hint="Can be filled in later.">
              <input
                class={input}
                inputMode="decimal"
                placeholder="e.g. 56.50"
                value={form.fx_rate}
                onInput$={(_, el) => (form.fx_rate = el.value)}
              />
            </Field>
          )}
          {boxedCode && (
            <Field label="Serial year">
              <select
                class={input}
                disabled={yearFixed}
                onChange$={(_, el) => (form.serial_year = el.value)}
              >
                {serialYears(form.serial_year).map((y) => (
                  <option key={y} value={y} selected={y === form.serial_year}>
                    {y}
                  </option>
                ))}
              </select>
              {/* In the slot, not the `hint` prop: a prop computed from the
                  form here could be treated as constant and go stale. */}
              <span class={[hint, "block"]}>
                {yearFixed
                  ? "Boxes have serials; put the PO back to draft to change the year."
                  : `Box serials read ${boxedCode}${form.serial_year.slice(2)}-…, given when the PO is marked ordered.`}
              </span>
            </Field>
          )}
        </div>

        <div>
          <div class="mb-1 flex items-center justify-between">
            <span class="text-sm font-medium">Items</span>
            {costs && <span class={hint}>Unit prices in {form.currency}; blank if not known yet.</span>}
          </div>
          <div class="space-y-2">
            {form.lines.map((line, idx) => {
              let pieces: number | null = null;
              try {
                pieces = line.serial_code ? lineQty(line) : null;
              } catch {
                /* shown on save */
              }
              return (
                <div
                  key={line.key}
                  class={[
                    "grid items-start gap-2",
                    costs ? "grid-cols-[minmax(0,1fr)_6rem_9rem_auto]" : "grid-cols-[minmax(0,1fr)_6rem_auto]",
                  ]}
                >
                  <select
                    data-search="on"
                    class={input}
                    aria-label="Item"
                    onChange$={(_, el) => pickItem(line, el.value, boxMeta.value[el.value])}
                  >
                    <option value="">Choose an item…</option>
                    {pickable(items.value, line.item_id).map((i) => (
                      <option key={i.id} value={i.id} selected={i.id === line.item_id}>
                        {i.label}
                      </option>
                    ))}
                  </select>
                  {line.serial_code ? (
                    <span class="pt-1.5 text-right text-sm tabular-nums" title="Total pieces">
                      {pieces == null ? "—" : formatQty(pieces, line.unit)}
                    </span>
                  ) : (
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label="Quantity"
                      inputMode="numeric"
                      placeholder="Qty"
                      value={line.qty}
                      onInput$={(_, el) => (line.qty = el.value)}
                    />
                  )}
                  {costs && (
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label="Unit price"
                      inputMode="decimal"
                      placeholder={`${form.currency} price`}
                      value={line.price}
                      onInput$={(_, el) => (line.price = el.value)}
                    />
                  )}
                  <button
                    type="button"
                    class={btnGhost}
                    aria-label="Remove line"
                    disabled={form.lines.length === 1}
                    onClick$={() => form.lines.splice(idx, 1)}
                  >
                    ✕
                  </button>
                  {line.serial_code && (
                    <div class="col-span-full flex flex-wrap items-center gap-2 pb-1 pl-3 text-sm">
                      <input
                        class={[inputBase, "w-20 text-right"]}
                        aria-label="Boxes"
                        inputMode="numeric"
                        placeholder="0"
                        value={line.boxes}
                        onInput$={(_, el) => (line.boxes = el.value)}
                      />
                      <span>boxes of</span>
                      <input
                        class={[inputBase, "w-20 text-right"]}
                        aria-label="Pieces per box"
                        inputMode="numeric"
                        placeholder="pcs"
                        value={line.per_box}
                        onInput$={(_, el) => (line.per_box = el.value)}
                      />
                      <span>pcs, plus</span>
                      <input
                        class={[inputBase, "w-20 text-right"]}
                        aria-label="Loose pieces"
                        inputMode="numeric"
                        placeholder="0"
                        value={line.loose}
                        onInput$={(_, el) => (line.loose = el.value)}
                      />
                      <span>loose</span>
                      <span class={hint}>
                        {line.serial_code} serials; loose pieces make one more box.
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

        {costs && (
          <dl class="ml-auto grid w-72 grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm">
            <dt class="font-semibold">Total</dt>
            <dd class="text-right font-semibold tabular-nums">{formatMinor(total, form.currency)}</dd>
            {form.currency !== "PHP" && (
              <>
                <dt class="text-ink-muted">In pesos</dt>
                <dd class="text-right tabular-nums">{php == null ? "needs FX" : formatPeso(php)}</dd>
              </>
            )}
          </dl>
        )}
        <Field label="Notes">
          <textarea class={input} rows={2} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
        </Field>
        <p class={hint}>
          {form.id
            ? "Received lines can be raised but not removed or lowered below what came in."
            : "Saved as a draft with a new PO number. Mark it ordered once sent to the supplier."}
          {!costs && " Prices and exchange rates are entered by someone who can see costs."}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value}>
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
});

// TT payment form ------------------------------------------------------------

export interface TtForm {
  id: string;
  po_id: string;
  date: string;
  currency: string;
  amount: string;
  fx_rate: string;
  reference: string;
  /** Paid from. */
  bank_account_id: string;
  notes: string;
}

export const blankTt = (poId: string, currency: string, fx: string | null): TtForm => ({
  id: "",
  po_id: poId,
  date: todayManila(),
  currency,
  amount: "",
  fx_rate: fx ?? "",
  reference: "",
  bank_account_id: "",
  notes: "",
});

export const ttToForm = (t: TtInfo): TtForm => ({
  id: t.id,
  po_id: t.po_id,
  date: t.date,
  currency: t.currency,
  amount: moneyText(t.amount_minor),
  fx_rate: t.fx_rate ?? "",
  reference: t.reference ?? "",
  bank_account_id: t.bank_account_id ?? "",
  notes: t.notes ?? "",
});

export const TtModal = component$<{ open: Signal<boolean>; form: TtForm }>(({ open, form }) => {
  const error = useSignal("");
  const busy = useSignal(false);
  let php: number | null = null;
  try {
    const amount = parseMoney(form.amount);
    php = amount == null ? null : phpOf(amount, form.currency, fxOf(form.fx_rate));
  } catch {
    php = null;
  }
  return (
    <Modal open={open} title={form.id ? "Edit TT payment" : "Add TT payment"}>
      <form
        class="space-y-3"
        preventdefault:submit
        onSubmit$={async () => {
          busy.value = true;
          error.value = "";
          try {
            const amount = parseMoney(form.amount, "amount");
            if (!amount || amount <= 0) throw new Error("enter the amount paid");
            unwrap(
              await commands.savePoPayment({
                id: form.id || null,
                po_id: form.po_id,
                date: form.date,
                currency: form.currency,
                amount_minor: amount,
                fx_rate: form.currency === "PHP" ? "" : form.fx_rate,
                reference: form.reference,
                bank_account_id: form.bank_account_id || null,
                notes: form.notes,
              }),
            );
            open.value = false;
          } catch (e) {
            error.value = errorText(e);
          } finally {
            busy.value = false;
          }
        }}
      >
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid grid-cols-2 gap-3">
          <Field label="Date paid">
            <input
              type="date"
              class={input}
              required
              max={todayManila()}
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label="Currency">
            <select class={input} onChange$={(_, el) => (form.currency = el.value)}>
              {CURRENCIES.map((c) => (
                <option key={c} value={c} selected={c === form.currency}>
                  {c}
                </option>
              ))}
            </select>
          </Field>
          <Field label={`Amount (${form.currency})`}>
            <input
              class={[input, "text-right"]}
              inputMode="decimal"
              required
              value={form.amount}
              onInput$={(_, el) => (form.amount = el.value)}
            />
          </Field>
          {form.currency !== "PHP" && (
            <Field label={`Rate (₱ per ${form.currency})`}>
              <input
                class={[input, "text-right"]}
                inputMode="decimal"
                required
                value={form.fx_rate}
                onInput$={(_, el) => (form.fx_rate = el.value)}
              />
            </Field>
          )}
        </div>
        {form.currency !== "PHP" && (
          <p class={hint}>In pesos: {php == null ? "—" : formatPeso(php)}</p>
        )}
        <div class="grid grid-cols-2 gap-3">
          <Field label="Paid from">
            <BankAccountSelect
              value={form.bank_account_id}
              onPick$={(id) => {
                form.bank_account_id = id;
              }}
            />
          </Field>
          <Field label="Bank reference">
            <input class={input} value={form.reference} onInput$={(_, el) => (form.reference = el.value)} />
          </Field>
        </div>
        <Field label="Notes">
          <input class={input} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
        </Field>
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value}>
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
});

// Receiving -------------------------------------------------------------------

interface ReceiveRow {
  key: number;
  po_line_id: string;
  /** Set for a line with boxes: this row is that one box. */
  box_id: string | null;
  serial: string;
  /** What the box holds when full. */
  pcs: number;
  /** Box rows only: receive this box now. */
  include: boolean;
  qty: string;
  expiry: string;
}

export interface ReceiveForm {
  location_id: string;
  date: string;
  notes: string;
  rows: ReceiveRow[];
}

const plainRow = (poLineId: string, qty: string): ReceiveRow => ({
  key: newKey(),
  po_line_id: poLineId,
  box_id: null,
  serial: "",
  pcs: 0,
  include: true,
  qty,
  expiry: "",
});

/** A fresh receiving form: one row per line still to come, prefilled with
 *  what is left, or for a line with boxes one row per box not received yet. */
export const receiveForm = (d: PoDetail): ReceiveForm => ({
  location_id: "",
  date: todayManila(),
  notes: "",
  rows: d.lines.flatMap((l) =>
    l.boxes.length
      ? l.boxes
          .filter((b) => b.received <= 0)
          .map((b) => ({
            ...plainRow(l.id, String(b.pcs)),
            box_id: b.id,
            serial: b.serial ?? "",
            pcs: b.pcs,
          }))
      : l.remaining > 0
        ? [plainRow(l.id, String(l.remaining))]
        : [],
  ),
});

const addLot = (form: ReceiveForm, idx: number) => {
  form.rows.splice(idx + 1, 0, plainRow(form.rows[idx].po_line_id, ""));
};

/** Tick or untick every box of a PO line. */
const includeAll = (form: ReceiveForm, poLineId: string, on: boolean) => {
  for (const r of form.rows) if (r.po_line_id === poLineId) r.include = on;
};

/** One expiry date for every ticked box of a PO line. */
const expiryAll = (form: ReceiveForm, poLineId: string, date: string) => {
  for (const r of form.rows) if (r.po_line_id === poLineId && r.include) r.expiry = date;
};

/** Receive goods of a PO into one location. Each PO line can be split over
 *  several expiry dates; a line with boxes is received box by box. */
export const ReceiveModal = component$<{
  open: Signal<boolean>;
  detail: PoDetail;
  form: ReceiveForm;
  saved$?: QRL<(id: string) => void>;
}>(({ open, detail, form, saved$ }) => {
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["locations"],
    $(async () => {
      try {
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const lines = form.rows
        .filter((r) => (r.box_id ? r.include : r.qty.trim() && r.qty.trim() !== "0"))
        .map((r) => {
          const l = detail.lines.find((x) => x.id === r.po_line_id);
          const what = r.box_id ? `box ${r.serial}` : (l?.item_code ?? "an item");
          const qty = parseCount(r.qty, `quantity of ${what}`);
          if (r.box_id && !qty) throw new Error(`enter the quantity in ${what}, or untick it`);
          if (r.box_id && qty && qty > r.pcs) throw new Error(`${what} holds only ${r.pcs}`);
          if (l?.tracks_expiry && !r.expiry) throw new Error(`enter the expiry date of ${what}`);
          return { po_line_id: r.po_line_id, box_id: r.box_id, qty: qty ?? 0, expiry: r.expiry || null };
        });
      const id = unwrap(
        await commands.receivePo({
          po_id: detail.po.id,
          date: form.date,
          location_id: form.location_id,
          notes: form.notes,
          lines,
        }),
      );
      busy.value = false;
      open.value = false;
      if (saved$) await saved$(id);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  return (
    <Modal open={open} title={`Receive goods · PO ${detail.po.no ?? ""}`} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Receive into">
            <select class={input} required onChange$={(_, el) => (form.location_id = el.value)}>
              <option value="">Choose a location…</option>
              {pickable(locations.value, form.location_id).map((l) => (
                <option key={l.id} value={l.id} selected={l.id === form.location_id}>
                  {l.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Date received">
            <input
              type="date"
              class={input}
              required
              max={todayManila()}
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
        </div>
        {form.rows.length === 0 ? (
          <Alert tone="brand">Everything on this PO has been received.</Alert>
        ) : (
          <div class="space-y-2">
            <div class={["grid grid-cols-[minmax(0,1fr)_6rem_10rem_auto] gap-2", hint]}>
              <span>Item</span>
              <span class="text-right">Qty received</span>
              <span>Expiry</span>
              <span />
            </div>
            {form.rows.map((r, idx) => {
              const l = detail.lines.find((x) => x.id === r.po_line_id);
              const first = idx === 0 || form.rows[idx - 1].po_line_id !== r.po_line_id;
              if (r.box_id) {
                const mine = first ? form.rows.filter((x) => x.po_line_id === r.po_line_id) : [];
                return (
                  <div key={r.key} class="space-y-2">
                    {first && (
                      <div class="flex flex-wrap items-end justify-between gap-2 pt-1 text-sm">
                        <div>
                          <label class="flex items-center gap-2">
                            <input
                              type="checkbox"
                              class="accent-brand-500"
                              aria-label="All boxes of this item"
                              checked={mine.every((x) => x.include)}
                              onChange$={(_, el) => includeAll(form, r.po_line_id, el.checked)}
                            />
                            <span>
                              <span class="font-medium">{l?.item_code}</span> {l?.item_name}
                            </span>
                          </label>
                          <span class={[hint, "block pl-6"]}>
                            Ordered {formatQty(l?.qty ?? 0, l?.unit)} in {l?.boxes.length ?? 0} boxes, received{" "}
                            {l?.received ?? 0}; {mine.length} {mine.length === 1 ? "box" : "boxes"} to receive
                          </span>
                        </div>
                        {l?.tracks_expiry && (
                          <label class="flex items-center gap-2">
                            <span class={hint}>Expiry for all ticked</span>
                            <input
                              type="date"
                              class={inputBase}
                              onChange$={(_, el) => expiryAll(form, r.po_line_id, el.value)}
                            />
                          </label>
                        )}
                      </div>
                    )}
                    <div class="grid grid-cols-[minmax(0,1fr)_6rem_10rem_auto] items-start gap-2">
                      <label class="flex items-center gap-2 pt-1.5 pl-6 text-sm">
                        <input
                          type="checkbox"
                          class="accent-brand-500"
                          checked={r.include}
                          onChange$={(_, el) => (r.include = el.checked)}
                        />
                        <span class="font-mono">{r.serial || "no serial"}</span>
                        <span class={hint}>holds {r.pcs}</span>
                      </label>
                      <input
                        class={[inputBase, "w-full text-right"]}
                        aria-label={`Quantity in box ${r.serial}`}
                        inputMode="numeric"
                        disabled={!r.include}
                        value={r.qty}
                        onInput$={(_, el) => (r.qty = el.value)}
                      />
                      {l?.tracks_expiry ? (
                        <input
                          type="date"
                          class={input}
                          aria-label={`Expiry date of box ${r.serial}`}
                          disabled={!r.include}
                          value={r.expiry}
                          onChange$={(_, el) => (r.expiry = el.value)}
                        />
                      ) : (
                        <span class={[hint, "pt-2"]}>no expiry</span>
                      )}
                      <span />
                    </div>
                  </div>
                );
              }
              return (
                <div key={r.key} class="grid grid-cols-[minmax(0,1fr)_6rem_10rem_auto] items-start gap-2">
                  <div class="pt-1.5 text-sm">
                    {first ? (
                      <>
                        <span class="font-medium">{l?.item_code}</span> {l?.item_name}
                        <span class={[hint, "block"]}>
                          Ordered {formatQty(l?.qty ?? 0, l?.unit)}, received {l?.received ?? 0}, left{" "}
                          {l?.remaining ?? 0}
                        </span>
                      </>
                    ) : (
                      <span class={hint}>another lot</span>
                    )}
                  </div>
                  <input
                    class={[inputBase, "w-full text-right"]}
                    aria-label="Quantity received"
                    inputMode="numeric"
                    value={r.qty}
                    onInput$={(_, el) => (r.qty = el.value)}
                  />
                  {l?.tracks_expiry ? (
                    <input
                      type="date"
                      class={input}
                      aria-label="Expiry date"
                      value={r.expiry}
                      onChange$={(_, el) => (r.expiry = el.value)}
                    />
                  ) : (
                    <span class={[hint, "pt-2"]}>no expiry</span>
                  )}
                  <div class="flex">
                    {l?.tracks_expiry && (
                      <button
                        type="button"
                        class={btnGhost}
                        title="Split into another expiry date"
                        onClick$={() => addLot(form, idx)}
                      >
                        + lot
                      </button>
                    )}
                    {!first && (
                      <button
                        type="button"
                        class={btnGhost}
                        aria-label="Remove lot"
                        onClick$={() => form.rows.splice(idx, 1)}
                      >
                        ✕
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
        <Field label="Notes">
          <input class={input} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
        </Field>
        <p class={hint}>
          Posting adds the goods to stock with a new receiving report number. Leave a quantity blank to skip
          that item{form.rows.some((r) => r.box_id) && ", and untick a box to receive it later"}. A wrong
          report is voided and entered again.
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value || form.rows.length === 0}>
            Post receipt
          </button>
        </div>
      </form>
    </Modal>
  );
});
