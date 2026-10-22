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

interface FormLine {
  key: number;
  id: string | null;
  item_id: string;
  qty: string;
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
  lines: FormLine[];
}

let nextKey = 1;
const newKey = () => nextKey++;

const blankLine = (): FormLine => ({
  key: newKey(),
  id: null,
  item_id: "",
  qty: "",
  price: "",
  notes: "",
});

export const blankPo = (): PoForm => ({
  id: "",
  supplier_id: "",
  date: todayManila(),
  currency: "USD",
  fx_rate: "",
  expected_date: "",
  supplier_ref: "",
  notes: "",
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
  lines: d.lines.map((l) => ({
    key: newKey(),
    id: l.id,
    item_id: l.item_id,
    qty: String(l.qty),
    price: moneyText(l.price_minor),
    notes: l.notes ?? "",
  })),
});

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
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["suppliers", "items", "doc_numbers"],
    $(async () => {
      try {
        suppliers.value = unwrap(await commands.supplierOptions());
        items.value = unwrap(await commands.itemOptions());
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
        .filter((l) => l.item_id || l.qty.trim())
        .map((l) => {
          const what = items.value.find((i) => i.id === l.item_id)?.label ?? "an item";
          if (!l.item_id) throw new Error("choose an item on every line, or remove it");
          const qty = parseCount(l.qty, `quantity for ${what}`);
          if (!qty) throw new Error(`enter a quantity for ${what}`);
          return {
            id: l.id,
            item_id: l.item_id,
            qty,
            price_minor: costs ? parseMoney(l.price, `price for ${what}`) : null,
            notes: l.notes,
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
      total += (parseCount(l.qty) ?? 0) * (parseMoney(l.price) ?? 0);
    } catch {
      /* shown on save */
    }
  }
  const php = phpOf(total, form.currency, fxOf(form.fx_rate));

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
        </div>

        <div>
          <div class="mb-1 flex items-center justify-between">
            <span class="text-sm font-medium">Items</span>
            {costs && <span class={hint}>Unit prices in {form.currency}; blank if not known yet.</span>}
          </div>
          <div class="space-y-2">
            {form.lines.map((line, idx) => (
              <div
                key={line.key}
                class={[
                  "grid items-start gap-2",
                  costs ? "grid-cols-[minmax(0,1fr)_6rem_9rem_auto]" : "grid-cols-[minmax(0,1fr)_6rem_auto]",
                ]}
              >
                <select class={input} aria-label="Item" onChange$={(_, el) => (line.item_id = el.value)}>
                  <option value="">Choose an item…</option>
                  {pickable(items.value, line.item_id).map((i) => (
                    <option key={i.id} value={i.id} selected={i.id === line.item_id}>
                      {i.label}
                    </option>
                  ))}
                </select>
                <input
                  class={[inputBase, "w-full text-right"]}
                  aria-label="Quantity"
                  inputMode="numeric"
                  placeholder="Qty"
                  value={line.qty}
                  onInput$={(_, el) => (line.qty = el.value)}
                />
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
              </div>
            ))}
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
        <Field label="Bank reference">
          <input class={input} value={form.reference} onInput$={(_, el) => (form.reference = el.value)} />
        </Field>
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
  qty: string;
  expiry: string;
}

export interface ReceiveForm {
  location_id: string;
  date: string;
  notes: string;
  rows: ReceiveRow[];
}

/** A fresh receiving form: one row per line still to come, prefilled with
 *  what is left. */
export const receiveForm = (d: PoDetail): ReceiveForm => ({
  location_id: "",
  date: todayManila(),
  notes: "",
  rows: d.lines
    .filter((l) => l.remaining > 0)
    .map((l) => ({ key: newKey(), po_line_id: l.id, qty: String(l.remaining), expiry: "" })),
});

const addLot = (form: ReceiveForm, idx: number) => {
  const row = form.rows[idx];
  form.rows.splice(idx + 1, 0, { key: newKey(), po_line_id: row.po_line_id, qty: "", expiry: "" });
};

/** Receive goods of a PO into one location. Each PO line can be split over
 *  several expiry dates. */
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
        .filter((r) => r.qty.trim() && r.qty.trim() !== "0")
        .map((r) => {
          const l = detail.lines.find((x) => x.id === r.po_line_id);
          const what = l?.item_code ?? "an item";
          const qty = parseCount(r.qty, `quantity of ${what}`);
          if (l?.tracks_expiry && !r.expiry) throw new Error(`enter the expiry date of ${what}`);
          return { po_line_id: r.po_line_id, qty: qty ?? 0, expiry: r.expiry || null };
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
          that item. A wrong report is voided and entered again.
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
