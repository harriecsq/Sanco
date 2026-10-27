import { $, component$, useSignal, useStore, useVisibleTask$, type QRL, type Signal } from "@builder.io/qwik";
import {
  commands,
  type BookingDetail,
  type BookingLine,
  type BoxDraw,
  type BoxStock,
  type DrDetail,
  type LocationInfo,
  type ReceiptKind,
  type StockLot,
} from "~/lib/bindings";
import { boxEquiv, boxSummary, formatDay, formatPct, formatQty, parseCount, todayManila } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Field, Modal } from "~/components/ui/ui";
import { RECEIPT, Totals } from "~/components/sales/sales";
import { btnGhost, btnPrimary, btnSecondary, hint, input, inputBase } from "~/components/ui/styles";

interface Row {
  key: number;
  /** The DR line being edited, if any. */
  id: string | null;
  line_id: string;
  /** The lot's expiry date, or "" for the first lot in stock. */
  lot: string;
  qty: string;
  /** Items in boxes: the boxes were chosen by hand (`picks`). */
  manual: boolean;
  picks: { box_id: string; qty: string }[];
  /** The edited line's draws, kept while its qty and location stay. */
  saved: BoxDraw[] | null;
  saved_qty: number;
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
  lot: "",
  qty,
  manual: false,
  picks: [],
  saved: null,
  saved_qty: 0,
});

/** A typed count, or 0 while it isn't one. */
const countOf = (text: string) => {
  try {
    return parseCount(text) ?? 0;
  } catch {
    return 0;
  }
};

/** Items that come in boxes, from whatever knows. */
const boxedItems = (b: BookingDetail | null, dr: DrDetail | null | undefined, boxes: BoxStock[]) =>
  new Set([
    ...(b?.lines ?? []).filter((l) => l.pcs_per_box != null).map((l) => l.item_id),
    ...(dr?.lines ?? []).filter((l) => l.boxed).map((l) => l.item_id),
    ...boxes.map((x) => x.item_id),
  ]);

/** A box of the row's item at the location, with what's left for the row. */
interface BoxLeft {
  box: BoxStock;
  left: number;
}

/** What a row of an item in boxes takes. */
interface BoxPlan {
  /** Chosen by hand, the edited line's saved draws, or picked automatically. */
  mode: "manual" | "saved" | "auto";
  avail: BoxLeft[];
  draws: BoxDraw[];
  /** Pieces in those boxes for this row. */
  onHand: number;
  total: number;
}

/**
 * The backend's automatic pick: started boxes first, then the earliest
 * expiry, then the lowest serial (the order `drBoxes` lists them in).
 */
const autoDraws = (avail: BoxLeft[], qty: number): BoxDraw[] => {
  const order = avail
    .map((a, i) => ({ ...a, i }))
    .filter((a) => a.left > 0)
    .sort(
      (a, b) =>
        Number(b.left < b.box.pcs) - Number(a.left < a.box.pcs) ||
        (a.box.expiry ?? "9999").localeCompare(b.box.expiry ?? "9999") ||
        a.i - b.i,
    );
  const out: BoxDraw[] = [];
  let need = qty;
  for (const a of order) {
    if (need <= 0) break;
    const take = Math.min(a.left, need);
    out.push({ box_id: a.box.box_id, serial: a.box.serial, expiry: a.box.expiry, qty: take });
    need -= take;
  }
  return out;
};

/**
 * What each row of an item in boxes takes from which box (null for other
 * rows). Earlier rows' draws are no longer available to later ones, as in
 * the backend.
 */
const planRows = (
  rows: Row[],
  lines: BookingLine[],
  boxes: BoxStock[],
  location: string,
  sameLocation: boolean,
  boxed: Set<string>,
) => {
  const used = new Map<string, number>();
  return rows.map((row): BoxPlan | null => {
    const line = lines.find((l) => l.id === row.line_id);
    if (!line || !boxed.has(line.item_id)) return null;
    const avail = boxes
      .filter((x) => x.item_id === line.item_id && x.location_id === location)
      .map((box) => ({ box, left: box.qty - (used.get(box.box_id) ?? 0) }));
    const qty = countOf(row.qty);
    let mode: BoxPlan["mode"] = "auto";
    let draws: BoxDraw[];
    if (row.manual) {
      mode = "manual";
      draws = row.picks
        .map((p) => {
          const box = boxes.find((x) => x.box_id === p.box_id);
          const was = row.saved?.find((d) => d.box_id === p.box_id);
          return {
            box_id: p.box_id,
            serial: box?.serial ?? was?.serial ?? null,
            expiry: box?.expiry ?? was?.expiry ?? null,
            qty: countOf(p.qty),
          };
        })
        .filter((d) => d.qty > 0);
    } else if (row.saved && sameLocation && qty === row.saved_qty) {
      mode = "saved";
      draws = row.saved;
    } else {
      draws = autoDraws(avail, qty);
    }
    for (const d of draws) used.set(d.box_id, (used.get(d.box_id) ?? 0) + d.qty);
    return {
      mode,
      avail,
      draws,
      onHand: avail.reduce((n, a) => n + Math.max(a.left, 0), 0),
      total: draws.reduce((n, d) => n + d.qty, 0),
    };
  });
};

const drawsText = (draws: BoxDraw[]) => draws.map((d) => `${d.serial ?? "Box"} ×${d.qty}`).join(" · ");

/** Lots of an item at a location, soonest expiry first. */
const lotsFor = (lots: Lot[], line: BookingLine, location: string) =>
  lots.filter(
    (s) => s.item_id === line.item_id && s.location_id === location && (s.qty > 0 || s.own),
  );

const lotLabel = (l: StockLot) =>
  `${l.expiry ? `Exp. ${formatDay(l.expiry)}` : "No expiry"} · ${formatQty(l.qty, l.unit)}`;

/** Qty the edited DR has on each booking line. */
const ownQty = (dr: DrDetail | null | undefined, lineId: string) =>
  (dr?.lines ?? []).filter((l) => l.booking_line_id === lineId).reduce((n, l) => n + l.qty, 0);

/**
 * Requests a DR for some or all of a booking's remaining items, or (with
 * `dr`) edits one: its lines, lots and quantities, also after delivery.
 */
export const DrRequestModal = component$<{
  open: Signal<boolean>;
  bookingId: string;
  dr?: DrDetail | null;
  saved$?: QRL<(id: string) => void>;
}>(({ open, bookingId, dr, saved$ }) => {
  const booking = useSignal<BookingDetail | null>(null);
  const locations = useSignal<LocationInfo[]>([]);
  const lots = useSignal<Lot[]>([]);
  const boxes = useSignal<BoxStock[]>([]);
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
        const [b, l, s, x] = await Promise.all([
          commands.getBooking(bookingId),
          commands.listLocations(),
          commands.drLots(bookingId),
          commands.drBoxes(bookingId),
        ]);
        booking.value = unwrap(b);
        locations.value = unwrap(l).filter((x) => x.active || x.id === dr?.dr.from_location_id);
        const stock: Lot[] = unwrap(s);
        // An edited DR's own lots stay pickable; a delivered one's stock
        // counts as available again.
        if (dr) {
          const at = dr.dr.from_location_id ?? "";
          for (const l of dr.lines) {
            let lot = stock.find((x) => x.item_id === l.item_id && x.location_id === at && x.expiry === l.expiry);
            if (!lot) {
              lot = {
                item_id: l.item_id,
                item_code: l.item_code ?? "",
                item_name: l.item_name ?? "",
                unit: l.unit,
                category: null,
                location_id: at,
                location_name: dr.dr.from_location_name,
                expiry: l.expiry,
                days_left: null,
                qty: 0,
                full_boxes: 0,
                open_boxes: 0,
                open_pcs: 0,
              };
              stock.push(lot);
            }
            lot.own = true;
            if (dr.dr.status === "delivered") lot.qty += l.qty;
          }
          stock.sort((a, b) => (a.expiry ?? "9999").localeCompare(b.expiry ?? "9999"));
        }
        lots.value = stock;
        // Likewise what a delivered DR took from each box, at its location
        // (the backend adds it back before checking the new draws).
        const inBoxes = unwrap(x);
        if (dr?.dr.status === "delivered") {
          const at = dr.dr.from_location_id ?? "";
          for (const l of dr.lines) {
            for (const d of l.boxes) {
              let box = inBoxes.find((y) => y.box_id === d.box_id && y.location_id === at);
              if (!box) {
                // Emptied by this DR: its size isn't listed, so the preview
                // takes the item's box size.
                box = {
                  box_id: d.box_id,
                  serial: d.serial,
                  item_id: l.item_id,
                  item_code: l.item_code ?? "",
                  unit: l.unit,
                  location_id: at,
                  location_name: dr.dr.from_location_name,
                  expiry: d.expiry,
                  days_left: null,
                  pcs: l.pcs_per_box ?? d.qty,
                  qty: 0,
                  open: false,
                  po_id: null,
                  po_no: null,
                };
                inBoxes.push(box);
              }
              box.qty += d.qty;
            }
          }
        }
        boxes.value = inBoxes;
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
          lot: l.expiry ?? "",
          saved: l.boxes.length ? l.boxes.map((d) => ({ ...d })) : null,
          saved_qty: l.qty,
        }));
        // Other booking lines with something left can be added.
        for (const l of lines) {
          if (left(l) > ownQty(dr, l.id) && !rows.some((r) => r.line_id === l.id)) {
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
          .filter((s) => s.location_id === loc && s.qty > 0 && open_lines.some((l) => l.item_id === s.item_id))
          .reduce((n, s) => n + s.qty, 0);
      const best = [...locations.value].sort((a, b) => stockAt(b.id) - stockAt(a.id))[0];
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
      const plans = planRows(
        form.rows,
        b.lines,
        boxes.value,
        form.location_id,
        !dr || dr.dr.from_location_id === form.location_id,
        boxedItems(b, dr, boxes.value),
      );
      const lines = form.rows
        .map((r, i) => ({ r, plan: plans[i] }))
        .filter(({ r }) => r.qty.trim())
        .map(({ r, plan }) => {
          const line = b.lines.find((l) => l.id === r.line_id);
          if (!line) throw new Error("reload the booking: a line changed");
          const qty = parseCount(r.qty, `quantity for ${line.item_code}`) ?? 0;
          // Boxed items: each box has its own lot; no draws = picked by the backend.
          if (plan) {
            if (plan.mode === "manual" && qty > 0 && plan.total !== qty) {
              throw new Error(`the boxes of ${line.item_code} add up to ${plan.total}, not ${qty}`);
            }
            return {
              id: r.id,
              booking_line_id: r.line_id,
              expiry: null,
              qty,
              boxes: plan.mode === "auto" ? [] : plan.draws.map((d) => ({ box_id: d.box_id, qty: d.qty })),
            };
          }
          const lot = r.lot || lotsFor(lots.value, line, form.location_id)[0]?.expiry || null;
          return {
            id: r.id,
            booking_line_id: r.line_id,
            expiry: line.tracks_expiry ? lot : null,
            qty,
            boxes: [],
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
      if (saved$) await saved$(id);
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
    const price = kept?.price_c ?? b?.lines.find((l) => l.id === r.line_id)?.price_c ?? 0;
    return n + countOf(r.qty) * price;
  }, 0);
  const vat = form.kind === "invoice" ? Math.round((net * vatBp) / 10000) : 0;
  const plans = planRows(
    form.rows,
    b?.lines ?? [],
    boxes.value,
    form.location_id,
    !dr || dr.dr.from_location_id === form.location_id,
    boxedItems(b, dr, boxes.value),
  );

  return (
    <Modal open={open} title={title} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        {delivered && (
          <Alert tone="warning">
            This {k.name} was delivered: changing its items or quantities moves the difference in or
            out of stock.
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
                  form.kind === kind ? "border-brand-500 bg-brand-50" : "border-line",
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
                {kind === "dr" ? (
                  <span>
                    <span class="font-medium">DR</span> <span class="text-ink-muted">no VAT</span>
                  </span>
                ) : (
                  <span>
                    <span class="font-medium">Invoice</span>{" "}
                    <span class="text-ink-muted">VAT {formatPct(vatBp)}</span>
                  </span>
                )}
              </label>
            ))}
          </div>
          <p class={[hint, "mt-1"]}>
            {delivered
              ? "A delivered receipt keeps its type. To change it, void it and enter it again."
              : dr && dr.dr.no && form.kind !== dr.dr.kind && dr.dr.no_series !== (form.kind === "dr" ? "DR" : "SI")
                ? `Its number ${dr.dr.no} is freed: enter the number from the ${k.booklet} instead.`
                : b && (form.kind === "invoice") !== b.booking.vat
                  ? `The booking is ${b.booking.vat ? "with" : "without"} VAT.`
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
                // Lots and boxes chosen at the old location don't apply.
                form.rows.forEach((r) => {
                  r.lot = "";
                  r.manual = false;
                  r.picks = [];
                });
              }}
            >
              {locations.value.map((l) => (
                <option key={l.id} value={l.id} selected={l.id === form.location_id}>
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
              {dr ? `Clear a quantity to take the item off this ${k.name}.` : "Leave a quantity blank to deliver it later."}
            </span>
          </div>
          {form.rows.length === 0 ? (
            <p class="text-sm text-ink-muted">
              {b ? "Everything on this booking is already on a DR or invoice." : "Loading…"}
            </p>
          ) : (
            <div class="space-y-2">
              {form.rows.map((row, idx) => {
                const line = b?.lines.find((l) => l.id === row.line_id);
                if (!line) return null;
                const plan = plans[idx];
                const avail = lotsFor(lots.value, line, form.location_id);
                const lot = row.lot || avail[0]?.expiry || "";
                const onHand = plan
                  ? plan.onHand
                  : line.tracks_expiry
                    ? (avail.find((s) => s.expiry === lot)?.qty ?? 0)
                    : avail.reduce((n, s) => n + s.qty, 0);
                const qty = countOf(row.qty);
                const short = qty > onHand;
                const perBox = line.pcs_per_box ?? dr?.lines.find((l) => l.id === row.id)?.pcs_per_box;
                // "Choose boxes" starts from what would be taken now.
                const prefill = (plan?.draws ?? []).map((d) => ({ box_id: d.box_id, qty: String(d.qty) }));
                // Boxes to type into: those with something left here, plus
                // any already picked that aren't (e.g. a saved draw's box).
                const choices = plan
                  ? [
                      ...plan.avail
                        .filter((a) => a.left > 0 || row.picks.some((p) => p.box_id === a.box.box_id))
                        .map((a) => ({
                          box_id: a.box.box_id,
                          serial: a.box.serial,
                          expiry: a.box.expiry,
                          left: a.left,
                          pcs: a.box.pcs,
                        })),
                      ...row.picks
                        .filter((p) => !plan.avail.some((a) => a.box.box_id === p.box_id))
                        .map((p) => {
                          const was = row.saved?.find((d) => d.box_id === p.box_id);
                          return {
                            box_id: p.box_id,
                            serial: was?.serial ?? null,
                            expiry: was?.expiry ?? null,
                            left: 0,
                            pcs: 0,
                          };
                        }),
                    ]
                  : [];
                return (
                  <div
                    key={row.key}
                    class="grid grid-cols-[minmax(0,1fr)_13rem_6rem_auto] items-start gap-2"
                  >
                    <div class="pt-1.5 text-sm">
                      <span class="font-medium">{line.item_code}</span> {line.item_name}
                      <span class={[hint, "block"]}>
                        {formatQty(line.remaining + ownQty(dr, line.id), line.unit)} left on the booking
                        {dr ? ` (with this ${k.name}'s)` : ""}
                      </span>
                    </div>
                    {plan ? (
                      <span class={[hint, "block pt-2"]}>
                        {formatQty(onHand, line.unit)} in boxes here
                        <span class="block">
                          {boxSummary(
                            plan.avail.filter((a) => a.left > 0 && a.left >= a.box.pcs).length,
                            plan.avail.filter((a) => a.left > 0 && a.left < a.box.pcs).length,
                            plan.avail
                              .filter((a) => a.left > 0 && a.left < a.box.pcs)
                              .reduce((n, a) => n + a.left, 0),
                          )}
                        </span>
                      </span>
                    ) : line.tracks_expiry ? (
                      <select
                        class={input}
                        aria-label="Lot"
                        onChange$={(_, el) => (row.lot = el.value)}
                      >
                        {avail.map((s) => (
                          <option key={s.expiry ?? "-"} value={s.expiry ?? ""} selected={s.expiry === lot}>
                            {lotLabel(s)}
                          </option>
                        ))}
                        {!avail.length && <option value="">None in stock here</option>}
                      </select>
                    ) : (
                      <span class={[hint, "block pt-2"]}>
                        {formatQty(onHand, line.unit)} on hand
                      </span>
                    )}
                    <div>
                      <input
                        class={[inputBase, "w-full text-right"]}
                        aria-label="Quantity"
                        inputMode="numeric"
                        placeholder="Qty"
                        value={row.qty}
                        onInput$={(_, el) => (row.qty = el.value)}
                      />
                      {plan && boxEquiv(qty, perBox) && (
                        <span class={[hint, "block text-right"]}>{boxEquiv(qty, perBox)}</span>
                      )}
                      {short && <span class="block text-right text-xs text-danger">only {onHand}</span>}
                    </div>
                    <div class="flex">
                      {line.tracks_expiry && !plan && (
                        <button
                          type="button"
                          class={btnGhost}
                          title="Take part from another lot"
                          onClick$={() => form.rows.splice(idx + 1, 0, newRow(line.id))}
                        >
                          + lot
                        </button>
                      )}
                      <button
                        type="button"
                        class={btnGhost}
                        aria-label="Remove line"
                        onClick$={() => form.rows.splice(idx, 1)}
                      >
                        ✕
                      </button>
                    </div>
                    {plan && (
                      <div class="col-span-full rounded-md border border-line bg-surface-muted px-3 py-2 text-sm">
                        {plan.mode === "manual" ? (
                          <>
                            <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
                              <span>
                                <span class="font-medium">Boxes</span>{" "}
                                <span class={plan.total === qty ? "text-ink-muted" : "text-danger"}>
                                  {formatQty(plan.total)} of {formatQty(qty, line.unit)}
                                  {plan.total !== qty && " · must add up to the quantity"}
                                </span>
                              </span>
                              <button
                                type="button"
                                class={btnGhost}
                                onClick$={() => {
                                  row.manual = false;
                                  row.picks = [];
                                  row.saved = null;
                                }}
                              >
                                Pick automatically
                              </button>
                            </div>
                            {choices.length ? (
                              <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                                {choices.map((c) => {
                                  const id = c.box_id;
                                  const pick = row.picks.find((p) => p.box_id === id)?.qty ?? "";
                                  const over = countOf(pick) > Math.max(c.left, 0);
                                  return (
                                    <label
                                      key={id}
                                      class="flex items-center justify-between gap-2 rounded-md border border-line bg-white px-2 py-1"
                                    >
                                      <span class="min-w-0">
                                        <span class="font-medium">{c.serial ?? "Box"}</span>
                                        <span class={[hint, "block"]}>
                                          {c.expiry ? `Exp. ${formatDay(c.expiry)} · ` : ""}
                                          {c.pcs ? `${c.left} of ${c.pcs} left` : `${c.left} left`}
                                        </span>
                                      </span>
                                      <span class="text-right">
                                        <input
                                          class={[inputBase, "w-16 text-right"]}
                                          aria-label={`Pieces from ${c.serial ?? "this box"}`}
                                          inputMode="numeric"
                                          value={pick}
                                          onInput$={(_, el) => {
                                            const p = row.picks.find((x) => x.box_id === id);
                                            if (p) p.qty = el.value;
                                            else row.picks.push({ box_id: id, qty: el.value });
                                          }}
                                        />
                                        {over && (
                                          <span class="block text-xs text-danger">only {Math.max(c.left, 0)}</span>
                                        )}
                                      </span>
                                    </label>
                                  );
                                })}
                              </div>
                            ) : (
                              <p class={hint}>No boxes of this item here.</p>
                            )}
                          </>
                        ) : (
                          <div class="flex flex-wrap items-center justify-between gap-2">
                            <span>
                              <span class="font-medium">Boxes</span>{" "}
                              <span class="text-ink-muted">
                                {plan.mode === "saved" ? "as saved" : "picked automatically"}
                              </span>
                              {plan.draws.length > 0 && `: ${drawsText(plan.draws)}`}
                            </span>
                            <button
                              type="button"
                              class={btnGhost}
                              onClick$={() => {
                                row.manual = true;
                                row.picks = prefill;
                              }}
                            >
                              Choose boxes
                            </button>
                          </div>
                        )}
                      </div>
                    )}
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
        <Totals totals={{ net_c: net, vat_c: vat, gross_c: net + vat }} vat={form.kind === "invoice"} vatBp={vatBp} />
        {!dr && (
          <p class={hint}>
            The {k.name} number from the {k.booklet} is entered when it is approved or delivered.
          </p>
        )}
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value || form.rows.length === 0}>
            {dr ? "Save" : `Request ${k.name}`}
          </button>
        </div>
      </form>
    </Modal>
  );
});
