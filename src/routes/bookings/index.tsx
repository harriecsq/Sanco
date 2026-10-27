import {
  $,
  component$,
  useComputed$,
  useSignal,
  useStore,
  useVisibleTask$,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type BookingAction,
  type BookingDetail,
  type BookingInfo,
  type Choice,
  type PriceQuote,
  type PriceSource,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  boxEquiv,
  formatDateTime,
  formatDay,
  formatPct,
  formatPeso,
  formatQty,
  moneyText,
  parseCount,
  parseMoney,
  todayManila,
} from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { RULE_LABELS } from "~/components/rates/rates";
import { DrRequestModal } from "~/components/sales/dr-request";
import {
  BOOKING_STATUS,
  DrNo,
  DrStatusBadge,
  OfflineBadge,
  PRICE_SOURCE_LABEL,
  PROGRESS_LABEL,
  RECEIPT,
  receiptHref,
  StatusBadge,
  Totals,
} from "~/components/sales/sales";
import { CreditWarning } from "~/components/accounting/accounting";
import {
  Alert,
  Badge,
  Confirm,
  Empty,
  Field,
  InfoList,
  Modal,
  NoAccess,
  PageHeader,
} from "~/components/ui/ui";
import {
  btnGhost,
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

const TABLES = [
  "bookings",
  "booking_lines",
  "delivery_receipts",
  "dr_lines",
  "clients",
  "items",
  "item_costs",
  "item_cost_charges",
  "doc_numbers",
  "users",
];

interface FormLine {
  key: number;
  id: string | null;
  item_id: string;
  qty: string;
  /** Net unit price as typed; blank uses the client's price. */
  price: string;
  notes: string;
  /** Saved price and its source, while editing a line. */
  saved: { price: number; rule: PriceSource } | null;
  /** The saved item's box size, for items in boxes. */
  per_box: number | null;
}

interface Form {
  id: string;
  client_id: string;
  date: string;
  vat: boolean;
  client_ref: string;
  deliver_to: string;
  notes: string;
  lines: FormLine[];
}

let nextKey = 1;
const blankLine = (): FormLine => ({
  key: nextKey++,
  id: null,
  item_id: "",
  qty: "",
  price: "",
  notes: "",
  saved: null,
  per_box: null,
});

const blank = (): Form => ({
  id: "",
  client_id: "",
  date: todayManila(),
  vat: true,
  client_ref: "",
  deliver_to: "",
  notes: "",
  lines: [blankLine()],
});

const toForm = (d: BookingDetail): Form => ({
  id: d.booking.id,
  client_id: d.booking.client_id ?? "",
  date: d.booking.date,
  vat: d.booking.vat,
  client_ref: d.booking.client_ref ?? "",
  deliver_to: d.booking.deliver_to ?? "",
  notes: d.booking.notes ?? "",
  lines: d.lines.map((l) => ({
    key: nextKey++,
    id: l.id,
    item_id: l.item_id,
    qty: String(l.qty),
    price: moneyText(l.price_c),
    notes: l.notes ?? "",
    saved: { price: l.price_c, rule: l.price_rule },
    per_box: l.pcs_per_box,
  })),
});

const quoteKey = (client: string, item: string) => `${client}|${item}`;

const BookingFormModal = component$<{
  open: Signal<boolean>;
  form: Form;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const clients = useSignal<Choice[]>([]);
  const items = useSignal<Choice[]>([]);
  /** Box size of each item that comes in boxes (needs the items list). */
  const boxSizes = useSignal<Record<string, number>>({});
  const vatBp = useSignal(1200);
  const quotes = useStore<Record<string, PriceQuote | null>>({});
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["clients", "items", "doc_numbers", "settings"],
    $(async () => {
      try {
        clients.value = unwrap(await commands.clientOptions());
        const list = unwrap(await commands.itemOptions());
        items.value = list;
        boxSizes.value = Object.fromEntries(
          list.flatMap((i) => (i.serial_code && i.pcs_per_box ? [[i.id, i.pcs_per_box]] : [])),
        );
        vatBp.value = unwrap(await commands.pricingSettings()).vat_bp;
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  // Fetch the client's price for every item on the form.
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      if (!track(() => open.value)) return;
      const client = track(() => form.client_id);
      const wanted = track(() => form.lines.map((l) => l.item_id)).filter(Boolean);
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
            price_c: parseMoney(l.price, `price for ${what}`),
            notes: l.notes,
          };
        });
      const id = unwrap(
        await commands.saveBooking({
          id: form.id || null,
          client_id: form.client_id,
          date: form.date,
          vat: form.vat,
          client_ref: form.client_ref,
          deliver_to: form.deliver_to,
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

  const pickable = (list: Choice[], current: string) =>
    list.filter((o) => o.active || o.id === current);

  let net = 0;
  for (const l of form.lines) {
    try {
      const qty = parseCount(l.qty) ?? 0;
      const typed = parseMoney(l.price);
      const quoted = quotes[quoteKey(form.client_id, l.item_id)]?.net_c ?? 0;
      net += qty * (typed ?? quoted);
    } catch {
      /* shown on save */
    }
  }
  const vat = form.vat ? Math.round((net * vatBp.value) / 10000) : 0;

  return (
    <Modal open={open} title={form.id ? "Edit booking" : "New booking"} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        {form.client_id && <CreditWarning clientId={form.client_id} extraC={form.id ? 0 : net + vat} />}
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Client">
            <select class={input} required onChange$={(_, el) => (form.client_id = el.value)}>
              <option value="">Choose a client…</option>
              {pickable(clients.value, form.client_id).map((c) => (
                <option key={c.id} value={c.id} selected={c.id === form.client_id}>
                  {c.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Booking date">
            <input
              type="date"
              class={input}
              required
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label="Client's PO / reference">
            <input
              class={input}
              value={form.client_ref}
              onInput$={(_, el) => (form.client_ref = el.value)}
            />
          </Field>
          <Field label="Deliver to">
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
            <span class={hint}>Prices are per unit, before VAT. Blank uses the client's price.</span>
          </div>
          <div class="space-y-2">
            {form.lines.map((line, idx) => {
              const q = form.client_id && line.item_id ? quotes[quoteKey(form.client_id, line.item_id)] : null;
              let typed: number | null = null;
              try {
                typed = parseMoney(line.price);
              } catch {
                typed = null;
              }
              const kept = typed != null && line.saved?.price === typed ? line.saved.rule : null;
              const custom = typed != null && q?.net_c !== typed;
              const below = typed != null && q?.cost_c != null && typed < q.cost_c;
              let equiv = "";
              try {
                equiv = boxEquiv(parseCount(line.qty) ?? 0, boxSizes.value[line.item_id] ?? line.per_box);
              } catch {
                equiv = "";
              }
              return (
                <div
                  key={line.key}
                  class="grid grid-cols-[minmax(0,1fr)_6rem_9rem_auto] items-start gap-2"
                >
                  <select
                    data-search="on"
                    class={input}
                    aria-label="Item"
                    onChange$={(_, el) => {
                      line.item_id = el.value;
                      line.saved = null;
                      line.per_box = null;
                    }}
                  >
                    <option value="">Choose an item…</option>
                    {pickable(items.value, line.item_id).map((i) => (
                      <option key={i.id} value={i.id} selected={i.id === line.item_id}>
                        {i.label}
                      </option>
                    ))}
                  </select>
                  <div class="space-y-0.5">
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label="Quantity"
                      inputMode="numeric"
                      placeholder="Qty"
                      value={line.qty}
                      onInput$={(_, el) => (line.qty = el.value)}
                    />
                    {equiv && <span class={[hint, "block text-right"]}>{equiv}</span>}
                  </div>
                  <div class="space-y-0.5">
                    <input
                      class={[inputBase, "w-full text-right"]}
                      aria-label="Unit price"
                      inputMode="decimal"
                      placeholder={q?.net_c != null ? moneyText(q.net_c) : "₱ price"}
                      value={line.price}
                      onInput$={(_, el) => (line.price = el.value)}
                    />
                    {q && (
                      <span class={["block text-right text-xs", below || q.below_cost ? "text-danger" : "text-ink-muted"]}>
                        {kept ? PRICE_SOURCE_LABEL[kept] : custom ? "Typed in" : RULE_LABELS[q.rule]}
                        {(below || (!custom && q.below_cost)) && " · below cost"}
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

        <div class="flex flex-wrap items-start justify-between gap-4">
          <label class="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              class="accent-brand-500"
              checked={form.vat}
              onChange$={(_, el) => (form.vat = el.checked)}
            />
            VAT ({formatPct(vatBp.value)})
          </label>
          <Totals totals={{ net_c: net, vat_c: vat, gross_c: net + vat }} vat={form.vat} vatBp={vatBp.value} />
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
          {form.id
            ? "Lines on a DR or invoice can be changed but not removed; those keep their own prices."
            : "Saved as a draft with a new booking number. Confirm it to request DRs or invoices."}
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

const BookingList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const bookings = useSignal<BookingInfo[]>([]);
  const clients = useSignal<Choice[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const open = useSignal(false);

  useLiveLoad(
    TABLES,
    $(async () => {
      try {
        bookings.value = unwrap(await commands.listBookings());
        clients.value = unwrap(await commands.clientOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status", "client"]);
    const q = f.q.toLowerCase();
    return bookings.value.filter(
      (b) =>
        (!q ||
          [b.no, b.client_name, b.client_ref, b.summary, b.notes].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.from || b.date >= f.from) &&
        (!f.to || b.date <= f.to) &&
        (!f.client || b.client_id === f.client) &&
        (!f.status ||
          (f.status === "to_deliver"
            ? b.status === "confirmed" && b.progress !== "full"
            : b.status === f.status)),
    );
  });

  return (
    <>
      <PageHeader title="Bookings" subtitle="Confirmed client orders, delivered through DRs or invoices.">
        {can(session.user, "bookings", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blank());
              open.value = true;
            }}
          >
            New booking
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search number, client, PO, items"
        dates
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              ...(["draft", "confirmed", "closed", "cancelled"] as const).map((s) => ({
                value: s,
                label: BOOKING_STATUS[s].label,
              })),
              { value: "to_deliver", label: "Still to deliver" },
            ],
          },
          {
            key: "client",
            label: "Clients",
            options: clients.value.map((c) => ({ value: c.id, label: c.label })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{bookings.value.length ? "No bookings match." : "No bookings yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Number</th>
                <th class={th}>Date</th>
                <th class={th}>Client</th>
                <th class={th}>Items</th>
                <th class={[th, "text-right"]}>Gross</th>
                <th class={th}>Delivery</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((b) => (
                <tr key={b.id} class={b.status === "cancelled" ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/bookings/?id=${b.id}`}>
                      {b.no ?? "—"}
                    </Link>
                    <OfflineBadge show={!b.no_confirmed} />
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(b.date)}</td>
                  <td class={td}>
                    {b.client_name ?? "Deleted client"}
                    {b.client_ref && <span class={[hint, "block"]}>PO {b.client_ref}</span>}
                  </td>
                  <td class={td}>{b.summary}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(b.totals.gross_c)}</td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {b.status === "draft" || b.status === "cancelled" ? "—" : PROGRESS_LABEL[b.progress]}
                    {b.open_drs > 0 && <span class={[hint, "block"]}>{b.open_drs} open DR(s) or invoice(s)</span>}
                  </td>
                  <td class={td}>
                    <StatusBadge s={BOOKING_STATUS[b.status]} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <BookingFormModal open={open} form={form} saved$={$((id: string) => nav(`/bookings/?id=${id}`))} />
    </>
  );
});

const ACTION_TEXT: Record<BookingAction, { title: string; body: string; label: string }> = {
  confirm: {
    title: "Confirm this booking?",
    body: "Its lines are locked and DRs or invoices can be requested.",
    label: "Confirm",
  },
  back_to_draft: {
    title: "Move back to draft?",
    body: "It is no longer confirmed: confirm it again before requesting more DRs or invoices.",
    label: "Back to draft",
  },
  close: {
    title: "Close this booking?",
    body: "Nothing more will be delivered on it. It can be reopened later.",
    label: "Close",
  },
  reopen: {
    title: "Reopen this booking?",
    body: "It becomes confirmed again, so more DRs or invoices can be requested.",
    label: "Reopen",
  },
  cancel: {
    title: "Cancel this booking?",
    body: "It stays in the list, marked cancelled. Only bookings without DRs or invoices can be cancelled.",
    label: "Cancel booking",
  },
};

const BookingView = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const nav = useNavigate();
  const detail = useSignal<BookingDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const editOpen = useSignal(false);
  const deleteOpen = useSignal(false);
  const drOpen = useSignal(false);
  const action = useSignal<BookingAction>("confirm");
  const actionOpen = useSignal(false);
  // Through a computed signal: a prop computed inline from `action` is
  // treated as constant by the optimizer, so the title would go stale.
  const actionTitle = useComputed$(() => ACTION_TEXT[action.value].title);
  const reason = useSignal("");
  const actionError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    TABLES,
    $(async () => {
      try {
        const d = unwrap(await commands.getBooking(id));
        detail.value = d;
        missing.value = !d;
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  if (missing.value) {
    return (
      <>
        <PageHeader title="Booking not found" />
        <Link class="text-sm text-brand-700 hover:underline" href="/bookings/">
          Back to bookings
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const b = det.booking;
  const user = session.user;
  const costs = det.lines.some((l) => l.cost_c != null);
  const margins = det.lines.some((l) => l.margin_bp != null);
  const delivering = b.status === "confirmed" || b.status === "closed";
  const canRequest =
    b.status === "confirmed" &&
    det.lines.some((l) => l.remaining > 0) &&
    (can(user, "bookings", "create") || can(user, "accounting", "create"));

  const ask = $((a: BookingAction) => {
    action.value = a;
    reason.value = "";
    actionError.value = "";
    actionOpen.value = true;
  });

  const rows: [string, string][] = [
    ["Client", b.client_name ? `${b.client_name}${b.client_code ? ` (${b.client_code})` : ""}` : "Deleted client"],
    ["Date", formatDay(b.date)],
    ["Client's PO", b.client_ref ?? ""],
    ["Deliver to", b.deliver_to ?? ""],
    ["VAT", b.vat ? formatPct(b.vat_bp) : "No VAT"],
    ["Notes", b.notes ?? ""],
    ["Entered", `${formatDateTime(b.created_at)} by ${b.created_by_name ?? "—"}`],
  ];
  if (b.confirmed_at) rows.push(["Confirmed", `${formatDateTime(b.confirmed_at)} by ${b.confirmed_by_name ?? "—"}`]);
  if (b.status === "closed") rows.push(["Closed", `${formatDateTime(b.closed_at)} by ${b.closed_by_name ?? "—"}`]);
  if (b.status === "cancelled") {
    rows.push(["Cancelled", `${formatDateTime(b.closed_at)} by ${b.closed_by_name ?? "—"}`]);
    rows.push(["Reason", b.cancel_reason ?? ""]);
  }

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/bookings/">
          ← Bookings
        </Link>
      </p>
      <PageHeader title={`Booking ${b.no ?? ""}`} subtitle={b.client_name ?? undefined}>
        {b.status === "draft" && can(user, "bookings", "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (deleteOpen.value = true)}>
            Delete
          </button>
        )}
        {(b.status === "draft" || b.status === "confirmed") && can(user, "bookings", "edit") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("cancel")}>
            Cancel booking
          </button>
        )}
        {b.status === "confirmed" && can(user, "bookings", "edit") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("back_to_draft")}>
            Back to draft
          </button>
        )}
        {b.status === "confirmed" && can(user, "bookings", "edit") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("close")}>
            Close
          </button>
        )}
        {b.status === "closed" && can(user, "bookings", "edit") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("reopen")}>
            Reopen
          </button>
        )}
        {b.status !== "cancelled" && can(user, "bookings", "edit") && (
          <button
            q:slot="actions"
            class={btnSecondary}
            onClick$={() => {
              Object.assign(form, toForm(det));
              editOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
        {b.status === "draft" && can(user, "bookings", "approve") && (
          <button q:slot="actions" class={btnPrimary} onClick$={() => ask("confirm")}>
            Confirm
          </button>
        )}
        {canRequest && (
          <button q:slot="actions" class={btnPrimary} onClick$={() => (drOpen.value = true)}>
            Request DR or invoice
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 flex flex-wrap items-center gap-3">
        <StatusBadge s={BOOKING_STATUS[b.status]} />
        {delivering && <span class="text-sm">{PROGRESS_LABEL[b.progress]}</span>}
        {b.bd_request_id && (
          <span class="text-sm">
            From request{" "}
            <Link class="text-brand-700 hover:underline" href={`/bd/?id=${b.bd_request_id}`}>
              {b.bd_no ?? "—"}
            </Link>
          </span>
        )}
        {b.client_id && (
          <Link class="text-sm text-brand-700 hover:underline" href={`/clients/?id=${b.client_id}`}>
            Client details
          </Link>
        )}
      </div>
      {b.client_id && (b.status === "draft" || b.status === "confirmed") && (
        <div class="mb-4">
          <CreditWarning clientId={b.client_id} extraC={b.status === "draft" ? b.totals.gross_c : 0} />
        </div>
      )}
      <div class="grid gap-4 2xl:grid-cols-[22rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <InfoList rows={rows} />
        </section>
        <div class="min-w-0 space-y-4">
          <section class={[card, "overflow-x-auto"]}>
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>Item</th>
                  <th class={[th, "text-right"]}>Qty</th>
                  <th class={[th, "text-right"]}>Price</th>
                  <th class={[th, "text-right"]}>Amount</th>
                  {costs && <th class={[th, "text-right"]}>Cost</th>}
                  {margins && <th class={[th, "text-right"]}>Margin</th>}
                  {delivering && <th class={[th, "text-right"]}>On receipts</th>}
                  {delivering && <th class={[th, "text-right"]}>Delivered</th>}
                  {delivering && <th class={[th, "text-right"]}>Left</th>}
                </tr>
              </thead>
              <tbody>
                {det.lines.map((l) => (
                  <tr key={l.id}>
                    <td class={[td, "min-w-48"]}>
                      <Link class="whitespace-nowrap text-brand-700 hover:underline" href={`/items/?id=${l.item_id}`}>
                        {l.item_code ?? "Deleted item"}
                      </Link>{" "}
                      {l.item_name}
                      {l.notes && <span class={[hint, "block"]}>{l.notes}</span>}
                    </td>
                    <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                      {formatQty(l.qty, l.unit)}
                      {l.pcs_per_box != null && (
                        <span class={[hint, "block"]}>{boxEquiv(l.qty, l.pcs_per_box)}</span>
                      )}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>
                      {formatPeso(l.price_c)}
                      <span class={[hint, "block whitespace-nowrap"]}>{PRICE_SOURCE_LABEL[l.price_rule]}</span>
                      {l.below_cost && <Badge tone="danger">below cost</Badge>}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPeso(l.amount_c)}</td>
                    {costs && <td class={[td, "text-right tabular-nums"]}>{l.cost_c == null ? "—" : formatPeso(l.cost_c)}</td>}
                    {margins && <td class={[td, "text-right tabular-nums"]}>{formatPct(l.margin_bp)}</td>}
                    {delivering && <td class={[td, "text-right tabular-nums"]}>{l.pending || "—"}</td>}
                    {delivering && <td class={[td, "text-right tabular-nums"]}>{l.delivered || "—"}</td>}
                    {delivering && (
                      <td class={[td, "text-right tabular-nums", l.remaining < 0 && "text-danger"]}>
                        {l.remaining}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
            <div class="border-t border-line px-3 py-3">
              <Totals totals={b.totals} vat={b.vat} vatBp={b.vat_bp} />
            </div>
          </section>
          {delivering && det.drs.length === 0 && (
            <section class={card}>
              <h2 class="border-b border-line px-4 py-3 font-semibold">Delivery receipts and invoices</h2>
              <Empty>No DRs or invoices yet.</Empty>
            </section>
          )}
          {delivering &&
            (["dr", "invoice"] as const).map((kind) => {
              const rows = det.drs.filter((d) => d.kind === kind);
              if (rows.length === 0) return null;
              return (
                <section key={kind} class={[card, "overflow-x-auto"]}>
                  <h2 class="border-b border-line px-4 py-3 font-semibold">{RECEIPT[kind].list}</h2>
                  <table class={table}>
                    <thead>
                      <tr>
                        <th class={th}>{RECEIPT[kind].title} no.</th>
                        <th class={th}>Date</th>
                        <th class={th}>From</th>
                        <th class={th}>Items</th>
                        <th class={[th, "text-right"]}>Gross</th>
                        <th class={th}>Status</th>
                      </tr>
                    </thead>
                    <tbody>
                      {rows.map((d) => (
                        <tr key={d.id} class={d.status === "cancelled" ? "text-ink-muted" : ""}>
                          <td class={[td, "whitespace-nowrap"]}>
                            <Link class="font-medium text-brand-700 hover:underline" href={receiptHref(d.kind, d.id)}>
                              <DrNo d={d} />
                            </Link>
                          </td>
                          <td class={[td, "whitespace-nowrap"]}>{formatDay(d.delivered_date ?? d.date)}</td>
                          <td class={td}>{d.from_location_name ?? "—"}</td>
                          <td class={td}>{d.summary}</td>
                          <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.totals.gross_c)}</td>
                          <td class={td}>
                            <DrStatusBadge d={d} />
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </section>
              );
            })}
        </div>
      </div>

      <BookingFormModal open={editOpen} form={form} />
      <DrRequestModal
        open={drOpen}
        bookingId={id}
        saved$={$((dr: string) => nav(`/accounting/?id=${dr}`))}
      />
      <Confirm
        open={deleteOpen}
        title={`Delete draft ${b.no ?? ""}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteBooking(id));
          await nav("/bookings/");
        })}
      >
        <p>This deletes the draft on every PC. Its number isn't given out again.</p>
      </Confirm>
      <Modal open={actionOpen} title={actionTitle.value}>
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            actionError.value = "";
            try {
              unwrap(await commands.setBookingStatus(id, action.value, reason.value));
              actionOpen.value = false;
            } catch (e) {
              actionError.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          {actionError.value && <Alert>{actionError.value}</Alert>}
          <p class="text-sm">{ACTION_TEXT[action.value].body}</p>
          {action.value === "cancel" && (
            <Field label="Reason">
              <input class={input} value={reason.value} onInput$={(_, el) => (reason.value = el.value)} />
            </Field>
          )}
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (actionOpen.value = false)}>
              Back
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              {ACTION_TEXT[action.value].label}
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  if (!can(session.user, "bookings")) return <NoAccess />;
  const id = loc.url.searchParams.get("id");
  return id ? <BookingView key={id} id={id} /> : <BookingList />;
});

export const head: DocumentHead = { title: "Bookings · Sanco" };
