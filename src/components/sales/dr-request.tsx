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
  type BookingDetail,
  type BookingLine,
  type DrDetail,
  type LocationInfo,
  type ReceiptKind,
  type StockLot,
} from "~/lib/bindings";
import { formatQty, lotText, parseCount, todayManila } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Field, Modal } from "~/components/ui/ui";
import { RECEIPT, Totals } from "~/components/sales/sales";
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
  /** The DR line being edited, if any. */
  id: string | null;
  line_id: string;
  /** The lot, picked by hand. */
  lot_id: string;
  qty: string;
}

interface Form {
  /** DR (no VAT) or invoice (VAT). */
  kind: ReceiptKind;
  date: string;
  location_id: string;
  deliver_to: string;
  notes: string;
  delivered_date: string;
  received_by: string;
  rows: Row[];
}

/** A stock lot; `own` marks a lot the edited DR already uses. */
type Lot = StockLot & { own?: boolean };

let nextKey = 1;
const newKey = () => nextKey++;

const newRow = (line_id: string, qty = ""): Row => ({
  key: newKey(),
  id: null,
  line_id,
  lot_id: "",
  qty,
});

/** A typed count, or 0 while it isn't one. */
const countOf = (text: string) => {
  try {
    return parseCount(text) ?? 0;
  } catch {
    return 0;
  }
};

/** Lots of an item at a location, oldest first. */
const lotsFor = (lots: Lot[], line: BookingLine, location: string) =>
  lots.filter(
    (s) =>
      s.item_id === line.item_id &&
      s.location_id === location &&
      (s.qty > 0 || s.own),
  );

const lotLabel = (l: StockLot) => `${lotText(l)} · ${formatQty(l.qty, l.unit)}`;

/** Qty the edited DR has on each booking line. */
const ownQty = (dr: DrDetail | null | undefined, lineId: string) =>
  (dr?.lines ?? [])
    .filter((l) => l.booking_line_id === lineId)
    .reduce((n, l) => n + l.qty, 0);

/**
 * Requests a DR for some or all of a booking's remaining items, or (with
 * `dr`) edits one: its lines, lots and quantities, also after delivery.
 */
export const DrRequestModal = component$<{
  open: Signal<boolean>;
  bookingId: string;
  dr?: DrDetail | null;
  saved$?: QRL<(id: string, kind: ReceiptKind) => void>;
}>(({ open, bookingId, dr, saved$ }) => {
  const booking = useSignal<BookingDetail | null>(null);
  const locations = useSignal<LocationInfo[]>([]);
  const lots = useSignal<Lot[]>([]);
  const error = useSignal("");
  const busy = useSignal(false);
  const form = useStore<Form>({
    kind: "dr",
    date: "",
    location_id: "",
    deliver_to: "",
    notes: "",
    delivered_date: "",
    received_by: "",
    rows: [],
  });

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      if (!track(() => open.value)) return;
      error.value = "";
      try {
        const [b, l, s] = await Promise.all([
          commands.getBooking(bookingId),
          commands.listLocations(),
          commands.drLots(bookingId),
        ]);
        booking.value = unwrap(b);
        locations.value = unwrap(l).filter(
          (x) => x.active || x.id === dr?.dr.from_location_id,
        );
        const stock: Lot[] = unwrap(s);
        // An edited DR's own lots stay pickable; a delivered one's stock
        // counts as available again.
        if (dr) {
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
          stock.sort((a, b) => (a.month ?? "").localeCompare(b.month ?? ""));
        }
        lots.value = stock;
      } catch (e) {
        error.value = errorText(e);
        return;
      }
      const lines = booking.value?.lines ?? [];
      const left = (l: BookingLine) => l.remaining + ownQty(dr, l.id);
      if (dr) {
        form.kind = dr.dr.kind;
        form.date = dr.dr.date;
        form.location_id = dr.dr.from_location_id ?? "";
        form.deliver_to = dr.dr.deliver_to ?? "";
        form.notes = dr.dr.notes ?? "";
        form.delivered_date = dr.dr.delivered_date ?? todayManila();
        form.received_by = dr.dr.received_by ?? "";
        const rows: Row[] = dr.lines.map((l) => ({
          ...newRow(l.booking_line_id ?? "", String(l.qty)),
          id: l.id,
          lot_id: l.lot_id ?? "",
        }));
        // Other booking lines with something left can be added.
        for (const l of lines) {
          if (
            left(l) > ownQty(dr, l.id) &&
            !rows.some((r) => r.line_id === l.id)
          ) {
            rows.push(newRow(l.id));
          }
        }
        form.rows = rows;
        return;
      }
      const open_lines = lines.filter((l) => l.remaining > 0);
      // Default to the location holding the most of what's left.
      const stockAt = (loc: string) =>
        lots.value
          .filter(
            (s) =>
              s.location_id === loc &&
              s.qty > 0 &&
              open_lines.some((l) => l.item_id === s.item_id),
          )
          .reduce((n, s) => n + s.qty, 0);
      const best = [...locations.value].sort(
        (a, b) => stockAt(b.id) - stockAt(a.id),
      )[0];
      form.kind = booking.value?.booking.vat ? "invoice" : "dr";
      form.date = todayManila();
      form.location_id = best?.id ?? "";
      form.deliver_to = booking.value?.booking.deliver_to ?? "";
      form.notes = "";
      form.rows = open_lines.map((l) => newRow(l.id, String(l.remaining)));
    },
    { strategy: "document-ready" },
  );

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const b = booking.value;
      if (!b) throw new Error("the booking could not be loaded");
      const lines = form.rows
        .filter((r) => r.qty.trim())
        .map((r) => {
          const line = b.lines.find((l) => l.id === r.line_id);
          if (!line) throw new Error("reload the booking: a line changed");
          const qty = parseCount(r.qty, `quantity for ${line.item_code}`) ?? 0;
          return {
            id: r.id,
            booking_line_id: r.line_id,
            lot_id: r.lot_id || null,
            qty,
          };
        })
        .filter((l) => l.qty > 0);
      const request = {
        booking_id: bookingId,
        kind: form.kind,
        date: form.date,
        from_location_id: form.location_id,
        deliver_to: form.deliver_to,
        notes: form.notes,
        lines,
      };
      let id: string;
      if (dr) {
        id = dr.dr.id;
        const delivery =
          dr.dr.status === "delivered"
            ? { date: form.delivered_date, received_by: form.received_by }
            : null;
        unwrap(await commands.updateDr(id, request, delivery));
      } else {
        id = unwrap(await commands.requestDr(request));
      }
      open.value = false;
      if (saved$) await saved$(id, form.kind);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const b = booking.value;
  const delivered = dr?.dr.status === "delivered";
  const k = RECEIPT[form.kind];
  const title = dr
    ? `Edit ${RECEIPT[dr.dr.kind].title} ${dr.dr.no ?? ""}`
    : `Request a DR or invoice for ${b?.booking.no ?? "this booking"}`;
  const vatBp = dr?.dr.vat_bp || b?.booking.vat_bp || 0;
  // Kept lines keep their price; new ones take the booking's.
  const net = form.rows.reduce((n, r) => {
    const kept = dr?.lines.find((l) => l.id === r.id);
    const price =
      kept?.price_c ?? b?.lines.find((l) => l.id === r.line_id)?.price_c ?? 0;
    return n + countOf(r.qty) * price;
  }, 0);
  // No VAT is added on top any more; only an older delivered invoice keeps its own.
  const vat =
    delivered && form.kind === "invoice"
      ? Math.round((net * (dr?.dr.vat_bp ?? 0)) / 10000)
      : 0;
  return (
    <Modal open={open} title={title} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        {delivered && (
          <Alert tone="warning">
            This {k.name} was delivered: changing its items or quantities moves
            the difference in or out of stock.
          </Alert>
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
                  delivered && "cursor-not-allowed opacity-60",
                ]}
              >
                <input
                  type="radio"
                  name="receipt-kind"
                  class="accent-brand-500"
                  checked={form.kind === kind}
                  disabled={delivered}
                  onChange$={() => (form.kind = kind)}
                />
                <span class="font-medium">
                  {kind === "dr" ? "Delivery Receipt" : "Sales Invoice"}
                </span>
              </label>
            ))}
          </div>
          <p class={[hint, "mt-1"]}>
            {delivered
              ? "A delivered receipt keeps its type. To change it, void it and enter it again."
              : dr &&
                  dr.dr.no &&
                  form.kind !== dr.dr.kind &&
                  dr.dr.no_series !== (form.kind === "dr" ? "DR" : "SI")
                ? `Its number ${dr.dr.no} is freed: enter the number from the ${k.booklet} instead.`
                : b && (form.kind === "invoice") !== b.booking.vat
                  ? `The booking asks for a ${b.booking.vat ? "Sales Invoice" : "Delivery Receipt"}.`
                  : ""}
          </p>
        </fieldset>
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label={delivered ? "Planned for" : "Deliver on"}>
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
          {delivered && (
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
          {delivered && (
            <Field label="Received by">
              <input
                class={input}
                value={form.received_by}
                onInput$={(_, el) => (form.received_by = el.value)}
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
              {dr
                ? `Clear a quantity to take the item off this ${k.name}.`
                : "Leave a quantity blank to deliver it later."}
            </span>
          </div>
          {form.rows.length === 0 ? (
            <p class="text-sm text-ink-muted">
              {b
                ? "Everything on this booking is already on a DR or invoice."
                : "Loading…"}
            </p>
          ) : (
            <div class="space-y-2">
              {form.rows.map((row, idx) => {
                const line = b?.lines.find((l) => l.id === row.line_id);
                if (!line) return null;
                const avail = lotsFor(lots.value, line, form.location_id);
                const onHand =
                  avail.find((s) => s.lot_id === row.lot_id)?.qty ?? 0;
                const short = !!row.lot_id && countOf(row.qty) > onHand;
                return (
                  <div
                    key={row.key}
                    class="grid grid-cols-[minmax(0,1fr)_18rem_6rem_auto] items-start gap-2"
                  >
                    <div class="pt-1.5 text-sm">
                      <span class="font-medium">{line.item_code}</span>{" "}
                      {line.item_name}
                      <span class={[hint, "block"]}>
                        {formatQty(
                          line.remaining + ownQty(dr, line.id),
                          line.unit,
                        )}{" "}
                        left on the booking
                        {dr ? ` (with this ${k.name}'s)` : ""}
                      </span>
                    </div>
                    <select
                      class={input}
                      aria-label="Lot"
                      required={!!row.qty.trim()}
                      onChange$={(_, el) => (row.lot_id = el.value)}
                    >
                      <option value="" selected={!row.lot_id}>
                        {avail.length ? "Choose a lot…" : "None in stock here"}
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
                    <div class="flex">
                      <button
                        type="button"
                        class={btnGhost}
                        title="Take part from another lot"
                        onClick$={() =>
                          form.rows.splice(idx + 1, 0, newRow(line.id))
                        }
                      >
                        + lot
                      </button>
                      <button
                        type="button"
                        class={btnGhost}
                        aria-label="Remove line"
                        onClick$={() => form.rows.splice(idx, 1)}
                      >
                        ✕
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <Field label="Notes">
          <textarea
            class={input}
            rows={2}
            value={form.notes}
            onInput$={(_, el) => (form.notes = el.value)}
          />
        </Field>
        <Totals
          totals={{ net_c: net, vat_c: vat, gross_c: net + vat }}
          vat={form.kind === "invoice"}
          vatBp={vatBp}
        />
        {!dr && (
          <p class={hint}>
            The {k.name} number from the {k.booklet} is entered when it is
            approved or delivered.
          </p>
        )}
        <div class="flex justify-end gap-2">
          <button
            type="button"
            class={btnSecondary}
            onClick$={() => (open.value = false)}
          >
            Cancel
          </button>
          <button
            class={btnPrimary}
            disabled={busy.value || form.rows.length === 0}
          >
            {dr ? "Save" : `Request ${k.name}`}
          </button>
        </div>
      </form>
    </Modal>
  );
});
