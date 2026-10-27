import {
  $,
  component$,
  useSignal,
  useStore,
  useVisibleTask$,
} from "@builder.io/qwik";
import {
  commands,
  type ChargeMode,
  type CostSheetInfo,
  type SupplierPrice,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatDateTime,
  formatDay,
  formatSince,
  formatMinor,
  formatPeso,
  formatPesoOr,
  moneyText,
  parseCount,
  parseMoney,
  parsePct,
  pctText,
} from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Badge, Field } from "~/components/ui/ui";
import {
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  inputBase,
  table,
  td,
  th,
} from "~/components/ui/styles";

export const CURRENCIES = ["PHP", "USD", "CNY", "TWD"];

interface Line {
  charge_type_id: string;
  name: string;
  mode: ChargeMode;
  value: string;
  active: boolean;
}

interface Form {
  currency: string;
  price: string;
  fx: string;
  qty: string;
  notes: string;
  lines: Line[];
}

const valueText = (mode: ChargeMode, v: number | null) =>
  mode === "pct" ? pctText(v) : moneyText(v);

const parseValue = (mode: ChargeMode, text: string, name: string) =>
  mode === "pct"
    ? parsePct(text, `${name} percentage`)
    : parseMoney(text, `${name} amount`);

function fill(form: Form, s: CostSheetInfo) {
  form.currency = s.currency;
  form.price = moneyText(s.supplier_price_minor);
  form.fx = s.fx_rate ?? "";
  form.qty = s.shipment_qty?.toString() ?? "";
  form.notes = s.notes ?? "";
  form.lines = s.lines.map((l) => ({
    charge_type_id: l.charge_type_id,
    name: l.name,
    mode: l.mode,
    value: valueText(l.mode, l.value),
    active: l.active,
  }));
}

const safeMoney = (text: string) => {
  try {
    return parseMoney(text);
  } catch {
    return null;
  }
};

/** Same math as the backend, for a live preview while typing. */
function preview(form: Form) {
  try {
    const price = parseMoney(form.price);
    const fx = form.currency === "PHP" ? 1 : Number(form.fx.replace(/,/g, ""));
    if (price == null || !(fx > 0)) return null;
    const base = Math.round(price * fx);
    const qty = parseCount(form.qty);
    let missingQty = false;
    const amounts = form.lines.map((l) => {
      const v = parseValue(l.mode, l.value, l.name);
      if (v == null) return null;
      if (l.mode === "pct") return Math.round((base * v) / 10000);
      if (l.mode === "shipment") {
        if (!qty) {
          missingQty = true;
          return null;
        }
        return Math.round(v / qty);
      }
      return v;
    });
    const charges = amounts.reduce<number>((a, b) => a + (b ?? 0), 0);
    return { base, amounts, total: missingQty ? null : base + charges };
  } catch {
    return null;
  }
}

/** "US$12.50 × 56.50" */
const priceText = (minor: number | null, currency: string, fx: string | null) =>
  `${formatMinor(minor, currency)}${currency !== "PHP" && fx ? ` × ${fx}` : ""}`;

const poText = (sp: SupplierPrice) =>
  `${sp.po_no ?? "a purchase order"}${sp.po_date ? ` (${formatDay(sp.po_date)})` : ""}`;

/**
 * An item's cost sheet: supplier price × FX plus charge lines = landed cost.
 * Only shown to users who see costs; the backend refuses everyone else.
 */
export const CostSheet = component$<{ itemId: string; editable: boolean }>(
  ({ itemId, editable }) => {
    const sheet = useSignal<CostSheetInfo | null>(null);
    const form = useStore<Form>({
      currency: "PHP",
      price: "",
      fx: "",
      qty: "",
      notes: "",
      lines: [],
    });
    const dirty = useSignal(false);
    const changedElsewhere = useSignal(false);
    const error = useSignal("");
    const saved = useSignal(false);
    const busy = useSignal(false);
    const applied = useSignal("");
    // Keeps "last updated … ago" current while the sheet stays open.
    const now = useSignal(Date.now());
    // eslint-disable-next-line qwik/no-use-visible-task
    useVisibleTask$(({ cleanup }) => {
      now.value = Date.now();
      const tick = setInterval(() => (now.value = Date.now()), 30_000);
      cleanup(() => clearInterval(tick));
    });

    useLiveLoad(
      ["item_costs", "item_cost_charges", "charge_types", "items", "suppliers"],
      $(async () => {
        try {
          const s = unwrap(await commands.getCostSheet(itemId));
          if (!s) return;
          const before = sheet.value?.changed_at;
          sheet.value = s;
          if (!dirty.value) fill(form, s);
          else if (s.changed_at !== before) changedElsewhere.value = true;
        } catch (e) {
          error.value = errorText(e);
        }
      }),
    );

    const edit = $(() => {
      dirty.value = true;
      saved.value = false;
    });

    const save = $(async () => {
      busy.value = true;
      error.value = "";
      try {
        unwrap(
          await commands.saveCostSheet({
            item_id: itemId,
            currency: form.currency,
            supplier_price_minor: parseMoney(form.price, "supplier price"),
            fx_rate: form.currency === "PHP" ? "" : form.fx,
            shipment_qty: parseCount(form.qty, "shipment quantity"),
            notes: form.notes,
            lines: form.lines.map((l) => ({
              charge_type_id: l.charge_type_id,
              mode: l.mode,
              value: parseValue(l.mode, l.value, l.name),
            })),
          }),
        );
        dirty.value = false;
        changedElsewhere.value = false;
        saved.value = true;
        applied.value = "";
        const s = unwrap(await commands.getCostSheet(itemId));
        if (s) {
          sheet.value = s;
          fill(form, s);
        }
      } catch (e) {
        error.value = errorText(e);
      } finally {
        busy.value = false;
      }
    });

    const applySupplier = $(async () => {
      busy.value = true;
      error.value = "";
      saved.value = false;
      try {
        const used = unwrap(await commands.applySupplierPrice(itemId));
        applied.value = `Supplier price from ${poText(used)} applied.`;
        const s = unwrap(await commands.getCostSheet(itemId));
        if (s) {
          sheet.value = s;
          fill(form, s);
        }
      } catch (e) {
        error.value = errorText(e);
      } finally {
        busy.value = false;
      }
    });

    const s = sheet.value;
    if (!s) return error.value ? <Alert>{error.value}</Alert> : null;
    const sp = s.supplier_price;
    const noFx = !!sp && sp.currency !== "PHP" && !sp.fx_rate;
    const p = preview(form);
    const foreign = form.currency !== "PHP";
    const perShipment =
      form.qty !== "" || form.lines.some((l) => l.mode === "shipment");

    return (
      <section class={card}>
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-3">
          <div>
            <h2 class="font-semibold">Cost sheet</h2>
            <p class={hint}>
              Landed cost = supplier price × FX + charges. Percentages are of
              the PHP base cost; per-shipment amounts are divided by the
              shipment quantity.
            </p>
            <p
              class={[
                "mt-1 text-xs",
                s.price_pending ? "text-amber-700" : "text-ink-muted",
              ]}
              title={s.changed_at ? formatDateTime(s.changed_at) : undefined}
            >
              {s.changed_at ? (
                <>
                  Last updated {formatSince(s.changed_at, now.value)}
                  {s.changed_by && <> by {s.changed_by}</>} ·{" "}
                  {formatDateTime(s.changed_at)}
                </>
              ) : (
                "Never updated"
              )}
            </p>
          </div>
          <div class="text-right">
            <div class={hint}>Landed cost</div>
            <div class="text-lg font-semibold tabular-nums">
              {formatPesoOr(p ? p.total : s.landed?.total_c)}
            </div>
          </div>
        </div>
        <form class="space-y-3 p-4" preventdefault:submit onSubmit$={save}>
          {error.value && <Alert>{error.value}</Alert>}
          {changedElsewhere.value && (
            <Alert tone="warning">
              Someone saved this cost sheet while you were editing. Saving now
              replaces their values.
            </Alert>
          )}
          <div
            class={[
              "flex flex-wrap items-center justify-between gap-2 rounded-md px-3 py-2 text-sm",
              s.price_pending
                ? "bg-amber-50 text-amber-800"
                : "bg-brand-50 text-brand-800",
            ]}
          >
            <div>
              {!sp ? (
                <>No placed purchase order has a price for this item yet.</>
              ) : s.price_pending ? (
                <>
                  <span class="font-semibold">
                    New supplier price not set yet.
                  </span>{" "}
                  {poText(sp)} has{" "}
                  <strong class="tabular-nums">
                    {priceText(sp.price_minor, sp.currency, sp.fx_rate)}
                  </strong>
                  ; this sheet has{" "}
                  <span class="tabular-nums">
                    {priceText(s.supplier_price_minor, s.currency, s.fx_rate)}
                  </span>
                  .{noFx && <> Enter the PO's exchange rate first.</>}
                </>
              ) : (
                <>Matches the latest supplier price on {poText(sp)}.</>
              )}
              {applied.value && !dirty.value && (
                <span class="ml-2 font-medium">{applied.value}</span>
              )}
            </div>
            {editable && (
              <button
                type="button"
                class={s.price_pending ? btnPrimary : btnSecondary}
                disabled={
                  busy.value || !sp || noFx || !s.price_pending || dirty.value
                }
                title={
                  dirty.value
                    ? "Save or discard your changes first."
                    : !sp
                      ? "No placed purchase order has a price for this item yet."
                      : noFx
                        ? "The purchase order has no exchange rate yet."
                        : s.price_pending
                          ? `Copy ${priceText(sp.price_minor, sp.currency, sp.fx_rate)} from ${poText(sp)} into this sheet. Charges are kept.`
                          : "The sheet already has the latest supplier price."
                }
                onClick$={applySupplier}
              >
                {s.price_pending
                  ? "Update to supplier price"
                  : sp
                    ? "Up to date"
                    : "Update to supplier price"}
              </button>
            )}
          </div>
          <fieldset disabled={!editable} class="space-y-3">
            <div class="grid gap-3 sm:grid-cols-4">
              <Field label="Currency">
                <select
                  class={input}
                  onChange$={(_, el) => {
                    form.currency = el.value;
                    edit();
                  }}
                >
                  {CURRENCIES.map((c) => (
                    <option key={c} value={c} selected={c === form.currency}>
                      {c}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label={`Supplier price (${form.currency})`}>
                <input
                  class={input}
                  inputMode="decimal"
                  value={form.price}
                  onInput$={(_, el) => {
                    form.price = el.value;
                    edit();
                  }}
                />
              </Field>
              {foreign && (
                <Field label={`FX (₱ per 1 ${form.currency})`}>
                  <input
                    class={input}
                    inputMode="decimal"
                    value={form.fx}
                    onInput$={(_, el) => {
                      form.fx = el.value;
                      edit();
                    }}
                  />
                </Field>
              )}
              {perShipment && (
                <Field
                  label="Shipment qty"
                  hint="Units the per-shipment charges are divided by."
                >
                  <input
                    class={input}
                    inputMode="numeric"
                    value={form.qty}
                    onInput$={(_, el) => {
                      form.qty = el.value;
                      edit();
                    }}
                  />
                </Field>
              )}
            </div>
            <div class="overflow-x-auto">
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>Charge</th>
                    <th class={th}>Type</th>
                    <th class={th}>Value</th>
                    <th class={[th, "text-right"]}>Per unit</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class={td}>Base cost</td>
                    <td class={[td, hint]} colSpan={2}>
                      {formatMinor(safeMoney(form.price), form.currency)}
                      {foreign && form.fx && <> × {form.fx}</>}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>
                      {formatPesoOr(p?.base)}
                    </td>
                  </tr>
                  {form.lines.map((l, i) => (
                    <tr key={l.charge_type_id}>
                      <td class={td}>
                        {l.name}
                        {!l.active && (
                          <span class="ml-2">
                            <Badge>inactive</Badge>
                          </span>
                        )}
                      </td>
                      <td class={td}>
                        <select
                          class={[inputBase, "w-32"]}
                          aria-label={`${l.name} type`}
                          onChange$={(_, el) => {
                            form.lines[i].mode = el.value as ChargeMode;
                            edit();
                          }}
                        >
                          <option
                            value="per_unit"
                            selected={l.mode === "per_unit"}
                          >
                            ₱ per unit
                          </option>
                          <option value="pct" selected={l.mode === "pct"}>
                            % of base
                          </option>
                          <option
                            value="shipment"
                            selected={l.mode === "shipment"}
                          >
                            ₱ per shipment
                          </option>
                        </select>
                      </td>
                      <td class={td}>
                        <input
                          class={[inputBase, "w-32"]}
                          inputMode="decimal"
                          aria-label={`${l.name} value`}
                          placeholder={
                            l.mode === "pct"
                              ? "%"
                              : l.mode === "shipment"
                                ? "₱ total"
                                : "₱"
                          }
                          value={l.value}
                          onInput$={(_, el) => {
                            form.lines[i].value = el.value;
                            edit();
                          }}
                        />
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>
                        {p?.amounts[i] != null
                          ? formatPeso(p.amounts[i]!)
                          : "—"}
                      </td>
                    </tr>
                  ))}
                  <tr>
                    <td class={[td, "font-semibold"]} colSpan={3}>
                      Landed cost
                    </td>
                    <td class={[td, "text-right font-semibold tabular-nums"]}>
                      {formatPesoOr(p?.total)}
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
            <Field
              label="Notes"
              hint="E.g. which shipment or quotation these figures come from."
            >
              <input
                class={input}
                value={form.notes}
                onInput$={(_, el) => {
                  form.notes = el.value;
                  edit();
                }}
              />
            </Field>
          </fieldset>
          {editable && (
            <div class="flex items-center justify-end gap-2">
              {saved.value && <span class={hint}>Saved.</span>}
              {dirty.value && (
                <button
                  type="button"
                  class={btnSecondary}
                  onClick$={() => {
                    dirty.value = false;
                    changedElsewhere.value = false;
                    if (sheet.value) fill(form, sheet.value);
                  }}
                >
                  Discard changes
                </button>
              )}
              <button class={btnPrimary} disabled={busy.value || !dirty.value}>
                Save cost sheet
              </button>
            </div>
          )}
        </form>
      </section>
    );
  },
);
