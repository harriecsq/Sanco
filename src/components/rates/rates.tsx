import {
  $,
  component$,
  useSignal,
  useVisibleTask$,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import {
  commands,
  type Choice,
  type PriceQuote,
  type PriceRule,
  type RateInfo,
} from "~/lib/bindings";
import {
  formatAgo,
  formatPct,
  formatPeso,
  formatPesoOr,
  moneyText,
  parseMoney,
} from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Badge, Empty, Field, Modal } from "~/components/ui/ui";
import {
  btnGhost,
  btnPrimary,
  btnSecondary,
  hint,
  input,
  table,
  td,
  th,
} from "~/components/ui/styles";

export const RULE_LABELS: Record<PriceRule, string> = {
  manual: "Client rate",
  client_tier: "Client's margin tier",
  item_default: "Item's default margin",
  none: "No price yet",
};

export interface RateForm {
  client_id: string;
  item_id: string;
  /** Net price as typed. */
  price: string;
  vat: boolean;
  notes: string;
  /** Editing a rate that is set (enables "Clear rate"). */
  existing: boolean;
}

export const blankRate = (client_id = "", item_id = ""): RateForm => ({
  client_id,
  item_id,
  price: "",
  vat: true,
  notes: "",
  existing: false,
});

export const rateForm = (r: RateInfo): RateForm => ({
  client_id: r.client_id,
  item_id: r.item_id,
  price: moneyText(r.price_c),
  vat: r.vat,
  notes: r.notes ?? "",
  existing: true,
});

/** One rate per row; `edit$` opens it when the user may edit. */
export const RatesTable = component$<{
  rates: RateInfo[];
  showClient?: boolean;
  showItem?: boolean;
  edit$?: QRL<(r: RateInfo) => void>;
}>(({ rates, showClient, showItem, edit$ }) => {
  if (rates.length === 0) return <Empty>No client rates yet.</Empty>;
  const costs = rates.some((r) => r.cost_c != null);
  const margins = rates.some((r) => r.margin_bp != null);
  return (
    <div class="overflow-x-auto">
      <table class={table}>
        <thead>
          <tr>
            {showClient && <th class={th}>Client</th>}
            {showItem && <th class={th}>Item</th>}
            <th class={[th, "text-right"]}>Net</th>
            <th class={[th, "text-right"]}>VAT</th>
            <th class={[th, "text-right"]}>Gross</th>
            {costs && <th class={[th, "text-right"]}>Landed cost</th>}
            {margins && <th class={[th, "text-right"]}>Margin</th>}
            <th class={th}>Notes</th>
            <th class={th}>Changed</th>
            {edit$ && <th class={th} />}
          </tr>
        </thead>
        <tbody>
          {rates.map((r) => (
            <tr key={r.id}>
              {showClient && (
                <td class={td}>
                  <Link
                    class="text-brand-700 hover:underline"
                    href={`/clients/?id=${r.client_id}`}
                  >
                    {r.client_name}
                  </Link>
                  {r.client_code && (
                    <span class={["ml-1", hint]}>{r.client_code}</span>
                  )}
                </td>
              )}
              {showItem && (
                <td class={td}>
                  <Link
                    class="text-brand-700 hover:underline"
                    href={`/items/?id=${r.item_id}`}
                  >
                    {r.item_code}
                  </Link>{" "}
                  {r.item_name}
                </td>
              )}
              <td class={[td, "text-right tabular-nums"]}>
                {formatPeso(r.price_c)}
                {r.unit && <span class={hint}> /{r.unit}</span>}
              </td>
              <td class={[td, "text-right tabular-nums"]}>
                {r.vat ? (
                  formatPeso(r.vat_amounts.vat_c)
                ) : (
                  <span class={hint}>no VAT</span>
                )}
              </td>
              <td class={[td, "text-right tabular-nums font-medium"]}>
                {formatPeso(r.vat_amounts.gross_c)}
              </td>
              {costs && (
                <td class={[td, "text-right tabular-nums"]}>
                  {formatPesoOr(r.cost_c)}
                </td>
              )}
              {margins && (
                <td class={[td, "text-right tabular-nums"]}>
                  {formatPct(r.margin_bp)}
                </td>
              )}
              <td class={td}>
                {r.below_cost && <Badge tone="danger">below cost</Badge>}{" "}
                {r.notes}
              </td>
              <td class={[td, "whitespace-nowrap text-ink-muted"]}>
                {formatAgo(r.changed_at)}
              </td>
              {edit$ && (
                <td class={[td, "text-right"]}>
                  <button class={btnGhost} onClick$={() => edit$(r)}>
                    Edit
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
});

/** What the client pays without (or with) a manual rate. */
export const QuoteSummary = component$<{ quote: PriceQuote }>(
  ({ quote: q }) => (
    <div class="rounded-md bg-surface-muted px-3 py-2 text-sm">
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <span class="text-ink-muted">{RULE_LABELS[q.rule]}</span>
        <span class="font-medium tabular-nums">
          {formatPesoOr(q.net_c)} net
        </span>
      </div>
      {q.vat_amounts && (
        <div class={["mt-0.5 text-right tabular-nums", hint]}>
          + VAT {formatPeso(q.vat_amounts.vat_c)} ={" "}
          {formatPeso(q.vat_amounts.gross_c)}
        </div>
      )}
      {(q.cost_c != null || q.margin_bp != null) && (
        <div class={["mt-0.5 text-right", hint]}>
          {q.cost_c != null && <>Landed cost {formatPeso(q.cost_c)}</>}
          {q.margin_bp != null && <> · margin {formatPct(q.margin_bp)}</>}
        </div>
      )}
      {q.below_cost && (
        <div class="mt-1">
          <Badge tone="danger">below cost</Badge>
        </div>
      )}
    </div>
  ),
);

/**
 * Sets, changes or clears one client's rate for one item. The parent owns
 * the form; `lock` fixes the client or the item (on their detail pages).
 */
export const RateModal = component$<{
  open: Signal<boolean>;
  form: RateForm;
  lock?: "client" | "item";
}>(({ open, form, lock }) => {
  const clients = useSignal<Choice[]>([]);
  const items = useSignal<Choice[]>([]);
  const vatBp = useSignal(1200);
  const quote = useSignal<PriceQuote | null>(null);
  const error = useSignal("");
  const busy = useSignal(false);

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      if (!track(() => open.value)) return;
      error.value = "";
      try {
        const [c, i, p] = await Promise.all([
          commands.clientOptions(),
          commands.itemOptions(),
          commands.pricingSettings(),
        ]);
        clients.value = unwrap(c);
        items.value = unwrap(i);
        vatBp.value = unwrap(p).vat_bp;
      } catch (e) {
        error.value = errorText(e);
      }
    },
    { strategy: "document-ready" },
  );

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      const isOpen = track(() => open.value);
      const c = track(() => form.client_id);
      const i = track(() => form.item_id);
      quote.value = null;
      if (!isOpen || !c || !i) return;
      const res = await commands.priceQuote(c, i);
      if (res.status === "ok") quote.value = res.data;
    },
    { strategy: "document-ready" },
  );

  let preview: { vat: number; gross: number; belowCost: boolean } | null = null;
  try {
    const net = parseMoney(form.price);
    if (net != null) {
      const vat = form.vat ? Math.round((net * vatBp.value) / 10000) : 0;
      const cost = quote.value?.cost_c;
      preview = {
        vat,
        gross: net + vat,
        belowCost: cost != null && net < cost,
      };
    }
  } catch {
    preview = null;
  }

  const save = $(async (clear: boolean) => {
    busy.value = true;
    error.value = "";
    try {
      const price = clear ? null : parseMoney(form.price, "price");
      if (!clear && price == null)
        throw new Error("enter a price, or clear the rate");
      unwrap(
        await commands.saveRate({
          client_id: form.client_id,
          item_id: form.item_id,
          price_c: price,
          vat: form.vat,
          notes: form.notes,
        }),
      );
      open.value = false;
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const active = (list: Choice[], current: string) =>
    list.filter((o) => o.active || o.id === current);

  return (
    <Modal
      open={open}
      title={form.existing ? "Edit client rate" : "New client rate"}
    >
      <form
        class="space-y-3"
        preventdefault:submit
        onSubmit$={() => save(false)}
      >
        {error.value && <Alert>{error.value}</Alert>}
        <Field label="Client">
          <select
            class={input}
            required
            disabled={lock === "client" || form.existing}
            onChange$={(_, el) => (form.client_id = el.value)}
          >
            <option value="">Choose a client</option>
            {active(clients.value, form.client_id).map((c) => (
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
        <Field label="Item">
          <select
            data-search="on"
            class={input}
            required
            disabled={lock === "item" || form.existing}
            onChange$={(_, el) => (form.item_id = el.value)}
          >
            <option value="">Choose an item</option>
            {active(items.value, form.item_id).map((i) => (
              <option key={i.id} value={i.id} selected={i.id === form.item_id}>
                {i.label}
              </option>
            ))}
          </select>
        </Field>
        {quote.value && <QuoteSummary quote={quote.value} />}
        <Field label="Price per unit, before VAT (₱)">
          <input
            class={input}
            inputMode="decimal"
            placeholder="0.00"
            value={form.price}
            onInput$={(_, el) => (form.price = el.value)}
          />
        </Field>
        <label class="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            class="accent-brand-500"
            checked={form.vat}
            onChange$={(_, el) => (form.vat = el.checked)}
          />
          VAT ({formatPct(vatBp.value)})
        </label>
        {preview && (
          <p class={hint}>
            VAT {formatPeso(preview.vat)} · gross {formatPeso(preview.gross)}
            {preview.belowCost && (
              <span class="ml-2">
                <Badge tone="danger">below landed cost</Badge>
              </span>
            )}
          </p>
        )}
        <Field label="Notes">
          <input
            class={input}
            value={form.notes}
            onInput$={(_, el) => (form.notes = el.value)}
          />
        </Field>
        <div class="flex justify-between gap-2 pt-2">
          <div>
            {form.existing && (
              <button
                type="button"
                class={btnGhost}
                disabled={busy.value}
                onClick$={() => save(true)}
              >
                Clear rate
              </button>
            )}
          </div>
          <div class="flex gap-2">
            <button
              type="button"
              class={btnSecondary}
              onClick$={() => (open.value = false)}
            >
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Save
            </button>
          </div>
        </div>
      </form>
    </Modal>
  );
});
