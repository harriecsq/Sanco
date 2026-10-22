import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import { commands, type ChargeMode, type CostSheetInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatAgo,
  formatMinor,
  formatPeso,
  formatPesoOr,
  moneyText,
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
    const amounts = form.lines.map((l) => {
      const v = parseValue(l.mode, l.value, l.name);
      if (v == null) return null;
      return l.mode === "pct" ? Math.round((base * v) / 10000) : v;
    });
    const charges = amounts.reduce<number>((a, b) => a + (b ?? 0), 0);
    return { base, amounts, total: base + charges };
  } catch {
    return null;
  }
}

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
      notes: "",
      lines: [],
    });
    const dirty = useSignal(false);
    const changedElsewhere = useSignal(false);
    const error = useSignal("");
    const saved = useSignal(false);
    const busy = useSignal(false);

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
    const p = preview(form);
    const foreign = form.currency !== "PHP";

    return (
      <section class={card}>
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-3">
          <div>
            <h2 class="font-semibold">Cost sheet</h2>
            <p class={hint}>
              Landed cost = supplier price × FX + charges. Percentages are of
              the PHP base cost.
              {s.changed_at && <> Saved {formatAgo(s.changed_at)}.</>}
            </p>
          </div>
          <div class="text-right">
            <div class={hint}>Landed cost</div>
            <div class="text-lg font-semibold tabular-nums">
              {formatPesoOr(p?.total ?? s.landed?.total_c)}
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
          <fieldset disabled={!editable} class="space-y-3">
            <div class="grid gap-3 sm:grid-cols-3">
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
                        </select>
                      </td>
                      <td class={td}>
                        <input
                          class={[inputBase, "w-32"]}
                          inputMode="decimal"
                          aria-label={`${l.name} value`}
                          placeholder={l.mode === "pct" ? "%" : "₱"}
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
