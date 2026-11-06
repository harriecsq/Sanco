// DRs (no VAT) and invoices (VAT): their lists and pages, and where every
// client order starts. The routes bookings/ and bookings/invoices/ show one
// type each.
import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, useNavigate } from "@builder.io/qwik-city";
import {
  commands,
  type Choice,
  type DrDetail,
  type DrInfo,
  type LocationInfo,
  type ReceiptKind,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDateTime, formatDay, formatMonth, formatPeso, formatQty, serialSpan, todayManila } from "~/lib/format";
import { discountAs, discountOf } from "~/lib/discount";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { ReceiptFormModal } from "~/components/sales/receipt-form";
import { DR_STATUS, DrNo, DrStatusBadge, RECEIPT, receiptHref, Totals } from "~/components/sales/sales";
import { Alert, Badge, Empty, Field, InfoList, Modal, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { CHECK_STATUS, METHOD_LABEL } from "~/components/accounting/accounting";
import {
  btnDanger,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  table,
  td,
  th,
} from "~/components/ui/styles";

const TABLES = [
  "payments",
  "payment_allocations",
  "delivery_receipts",
  "dr_lines",
  "bookings",
  "clients",
  "items",
  "locations",
  "doc_numbers",
  "users",
  "settings",
];

/** The date that matters: delivered on, else planned for. */
const dayOf = (d: DrInfo) => d.delivered_date ?? d.date;

/** Booking and Accounting users may both work on DRs and invoices. */
const mayDo = (user: Parameters<typeof can>[0], action: "create" | "edit" | "delete") =>
  can(user, "bookings", action) || can(user, "accounting", action);

const DrList = component$<{ kind: ReceiptKind }>(({ kind }) => {
  const k = RECEIPT[kind];
  const session = useSession();
  const nav = useNavigate();
  const newOpen = useSignal(false);
  const loc = useLocation();
  const drs = useSignal<DrInfo[]>([]);
  const clients = useSignal<Choice[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");

  useLiveLoad(
    TABLES,
    $(async () => {
      try {
        drs.value = unwrap(await commands.listDrs()).filter((d) => d.kind === kind);
        clients.value = unwrap(await commands.clientOptions());
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status", "client", "location"]);
    const q = f.q.toLowerCase();
    return drs.value.filter(
      (d) =>
        (!q ||
          [d.no, d.booking_no, d.client_name, d.client_ref, d.summary, d.received_by, d.notes].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.from || dayOf(d) >= f.from) &&
        (!f.to || dayOf(d) <= f.to) &&
        (!f.client || d.client_id === f.client) &&
        (!f.location || d.from_location_id === f.location) &&
        (!f.status ||
          (f.status === "no_number"
            ? !d.no && d.status !== "void"
            : f.status === "clash"
              ? d.no_clash
              : d.status === f.status)),
    );
  });
  const pg = usePage(shown);

  return (
    <>
      <PageHeader
        title={k.list}
        subtitle={
          kind === "dr"
            ? "Every client order starts here: enter the DR, then mark it delivered (stock goes out)."
            : "Every client order that needs an invoice starts here: enter it, then mark it delivered (stock goes out)."
        }
      >
        {mayDo(session.user, "create") && (
          <button q:slot="actions" class={btnPrimary} onClick$={() => (newOpen.value = true)}>
            New {k.name}
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search={`Search ${k.name} no., client, PO, items`}
        dates
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              ...(["new", "delivered", "void"] as const).map((s) => ({
                value: s,
                label: DR_STATUS[s].label,
              })),
              { value: "no_number", label: `No ${k.name} no. yet` },
              { value: "clash", label: "Number used twice" },
            ],
          },
          {
            key: "client",
            label: "Clients",
            options: clients.value.map((c) => ({ value: c.id, label: c.label })),
          },
          {
            key: "location",
            label: "Locations",
            options: locations.value.map((l) => ({ value: l.id, label: l.name })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>
            {drs.value.length
              ? `No ${k.plural} match.`
              : `No ${k.plural} yet. Enter the first one with "New ${k.name}".`}
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>{k.title} no.</th>
                <th class={th}>Date</th>
                <th class={th}>Client</th>
                <th class={th}>Client's PO</th>
                <th class={th}>From</th>
                <th class={th}>Items</th>
                <th class={[th, "text-right"]}>Gross</th>
                <th class={[th, "text-right"]}>Balance</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((d) => (
                <tr key={d.id} class={d.status === "void" ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={receiptHref(d.kind, d.id)}>
                      <DrNo d={d} />
                    </Link>
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {formatDay(dayOf(d))}
                    {!d.delivered_date && <span class={[hint, "block"]}>planned</span>}
                  </td>
                  <td class={td}>{d.client_name ?? "—"}</td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {d.client_ref ?? "—"}
                    {d.booking_no && <span class={[hint, "block"]}>old booking {d.booking_no}</span>}
                  </td>
                  <td class={td}>{d.from_location_name ?? "—"}</td>
                  <td class={td}>{d.summary}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.totals.gross_c)}</td>
                  <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                    {d.status !== "delivered" ? (
                      "—"
                    ) : d.balance_c === 0 ? (
                      <Badge tone="brand">Paid</Badge>
                    ) : (
                      <span class={d.balance_c < 0 ? "text-amber-800" : d.due_date && d.due_date < todayManila() ? "text-danger" : ""}>
                        {formatPeso(d.balance_c)}
                      </span>
                    )}
                  </td>
                  <td class={td}>
                    <DrStatusBadge d={d} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
      <ReceiptFormModal
        open={newOpen}
        kind={kind}
        saved$={$((id: string, saved: ReceiptKind) => nav(receiptHref(saved, id)))}
      />
    </>
  );
});

type Step = "deliver" | "number" | "delete" | "void" | "restore";

const stepTitle = (step: Step, name: string) =>
  ({
    deliver: "Mark delivered",
    number: `${name === "DR" ? "DR" : "Invoice"} number`,
    delete: `Delete ${name}`,
    void: `Void delivered ${name}`,
    restore: `Restore ${name}`,
  })[step];

const DrView = component$<{ id: string; kind: ReceiptKind }>(({ id, kind }) => {
  const session = useSession();
  const nav = useNavigate();
  const detail = useSignal<DrDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const step = useSignal<Step>("deliver");
  const open = useSignal(false);
  const no = useSignal("");
  const date = useSignal("");
  const receivedBy = useSignal("");
  const reason = useSignal("");
  const stepError = useSignal("");
  const busy = useSignal(false);
  const editOpen = useSignal(false);

  useLiveLoad(
    TABLES,
    $(async () => {
      try {
        const d = unwrap(await commands.getDr(id));
        // Shown under its own type (it may have just switched).
        if (d && d.dr.kind !== kind) {
          await nav(receiptHref(d.dr.kind, id), { replaceState: true });
          return;
        }
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
        <PageHeader title={`${RECEIPT[kind].title} not found`} />
        <Link class="text-sm text-brand-700 hover:underline" href={RECEIPT[kind].route}>
          Back to {RECEIPT[kind].list.toLowerCase()}
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const d = det.dr;
  const k = RECEIPT[d.kind];
  const manual = det.manual_numbers;
  const user = session.user;

  const begin = $((s: Step) => {
    step.value = s;
    no.value = detail.value?.dr.no ?? "";
    date.value = todayManila();
    receivedBy.value = "";
    reason.value = "";
    stepError.value = "";
    open.value = true;
  });

  const submit = $(async () => {
    busy.value = true;
    stepError.value = "";
    const typed = manual ? no.value.trim() || null : null;
    try {
      switch (step.value) {
        case "deliver":
          unwrap(await commands.deliverDr(id, { date: date.value, received_by: receivedBy.value }, typed));
          break;
        case "number":
          unwrap(await commands.setDrNumber(id, no.value));
          break;
        case "delete":
          unwrap(await commands.deleteDr(id));
          open.value = false;
          await nav(RECEIPT[kind].route);
          return;
        case "void":
          unwrap(await commands.voidDr(id, reason.value));
          break;
        case "restore":
          unwrap(await commands.restoreDr(id));
          break;
      }
      open.value = false;
    } catch (e) {
      stepError.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const rows: [string, string][] = [
    ["Type", d.kind === "invoice" ? "Invoice (with VAT)" : "DR (no VAT)"],
    ["Client", d.client_name ? `${d.client_name}${d.client_code ? ` (${d.client_code})` : ""}` : "—"],
    ["From", d.from_location_name ?? "—"],
    ["Planned for", formatDay(d.date)],
    ["Client's PO", d.client_ref ?? ""],
    ["Deliver to", d.deliver_to ?? ""],
    ["Notes", d.notes ?? ""],
    ["Entered", `${formatDateTime(d.created_at)} by ${d.created_by_name ?? "—"}`],
  ];
  if (d.delivered_at) {
    rows.push(["Delivered on", formatDay(d.delivered_date)]);
    rows.push(["Received by", d.received_by ?? ""]);
    rows.push(["Marked by", `${d.delivered_by_name ?? "—"}, ${formatDateTime(d.delivered_at)}`]);
  }
  if (d.status === "void") {
    rows.push(["Voided", `${formatDateTime(d.voided_at)} by ${d.voided_by_name ?? "—"}`]);
    rows.push(["Reason", d.void_reason ?? ""]);
  }

  const canEdit = mayDo(user, "edit");
  const needsNo = manual && (step.value === "deliver" || step.value === "number");

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href={k.route}>
          ← {k.list}
        </Link>
      </p>
      <PageHeader title={d.no ? `${k.title} ${d.no}` : `${k.title} (no number yet)`} subtitle={d.client_name ?? undefined}>
        {d.status === "delivered" && mayDo(user, "delete") && (
          <button q:slot="actions" class={btnDanger} onClick$={() => begin("void")}>
            Void
          </button>
        )}
        {d.status === "void" && mayDo(user, "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => begin("restore")}>
            Restore
          </button>
        )}
        {mayDo(user, "delete") && (
          <button q:slot="actions" class={btnDanger} onClick$={() => begin("delete")}>
            Delete
          </button>
        )}
        {canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (editOpen.value = true)}>
            Edit
          </button>
        )}
        {manual && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => begin("number")}>
            {d.no ? `Correct ${k.name} no.` : `Enter ${k.name} no.`}
          </button>
        )}
        {d.status === "new" && canEdit && (
          <button q:slot="actions" class={btnPrimary} onClick$={() => begin("deliver")}>
            Mark delivered
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 space-y-2">
        <div class="flex flex-wrap items-center gap-3">
          <DrStatusBadge d={d} />
          <span class="text-sm">
            <DrNo d={d} />
          </span>
          {d.booking_no && <span class="text-sm text-ink-muted">From old booking {d.booking_no}</span>}
        </div>
        {d.no_clash && (
          <Alert>
            Another {k.name} has number {d.no}: two PCs entered it while they couldn't reach each
            other. Check the booklet and correct the wrong one.
          </Alert>
        )}
        {d.status === "void" && <Alert tone="warning">This {k.name} is void: its stock came back.</Alert>}
        {d.kind === "invoice" && d.no && d.no_series === "DR" && (
          <p class={hint}>Made before invoices had their own booklet: its number is from the DR booklet.</p>
        )}
      </div>
      <div class="grid gap-4 xl:grid-cols-[24rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <InfoList rows={rows} />
        </section>
        <section class={[card, "overflow-x-auto"]}>
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Item</th>
                <th class={th}>Lot</th>
                <th class={[th, "text-right"]}>Qty</th>
                <th class={[th, "text-right"]}>Price</th>
                <th class={[th, "text-right"]}>Amount</th>
              </tr>
            </thead>
            <tbody>
              {det.lines.map((l) => (
                <tr key={l.id}>
                  <td class={td}>
                    <span class="font-medium">{l.item_code ?? "Deleted item"}</span> {l.item_name}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {formatMonth(l.month)}
                    {serialSpan(l.serial_from, l.serial_to) && (
                      <span class={[hint, "block"]}>{serialSpan(l.serial_from, l.serial_to)}</span>
                    )}
                    {l.expiry && <span class={[hint, "block"]}>Exp. {formatDay(l.expiry)}</span>}
                  </td>
                  <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                    {formatQty(l.qty, l.unit)}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(l.price_c)}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(l.amount_c)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div class="border-t border-line px-3 py-3">
            <Totals totals={d.totals} vat={d.vat} vatBp={d.vat_bp} />
          </div>
        </section>
      </div>
      {(d.status === "delivered" || det.payments.length > 0) && (
        <section class={[card, "mt-4 overflow-x-auto"]}>
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-3">
            <div>
              <h2 class="font-semibold">Payments</h2>
              {d.due_date && (
                <p class={hint}>
                  Due {formatDay(d.due_date)}
                  {d.balance_c > 0 && d.due_date < todayManila() && " · overdue"}
                </p>
              )}
            </div>
            {d.status === "delivered" && (
              <div class="text-right text-sm">
                <span class="text-ink-muted">Paid</span>{" "}
                <span class="tabular-nums">{formatPeso(d.paid_c)}</span>
                {d.discount_c > 0 && (
                  <>
                    <span class="mx-2 text-line">|</span>
                    <span class="text-ink-muted">Discount</span>{" "}
                    <span class="tabular-nums">{formatPeso(d.discount_c)}</span>
                  </>
                )}
                <span class="mx-2 text-line">|</span>
                <span class="text-ink-muted">{d.balance_c < 0 ? "Overpaid" : "Balance"}</span>{" "}
                <span class={["font-semibold tabular-nums", d.balance_c < 0 && "text-amber-800"]}>
                  {formatPeso(Math.abs(d.balance_c))}
                </span>
              </div>
            )}
          </div>
          {det.payments.length === 0 ? (
            <Empty>No payments applied yet.</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>Payment</th>
                  <th class={th}>Received</th>
                  <th class={th}>Method</th>
                  <th class={[th, "text-right"]}>Applied</th>
                  {d.kind === "invoice" && <th class={[th, "text-right"]}>Discount</th>}
                </tr>
              </thead>
              <tbody>
                {det.payments.map((p) => (
                  <tr key={p.payment_id} class={p.counts ? "" : "text-ink-muted"}>
                    <td class={[td, "whitespace-nowrap"]}>
                      {can(user, "accounting") ? (
                        <Link class="font-medium text-brand-700 hover:underline" href={`/accounting/payments/?id=${p.payment_id}`}>
                          {p.no ?? "—"}
                        </Link>
                      ) : (
                        (p.no ?? "—")
                      )}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>{formatDay(p.date)}</td>
                    <td class={td}>
                      {METHOD_LABEL[p.method]}
                      {p.check_status && <span class="ml-2 text-xs">({CHECK_STATUS[p.check_status].label.toLowerCase()})</span>}
                      {!p.counts && <span class={[hint, "block"]}>doesn't count</span>}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPeso(p.amount_c)}</td>
                    {d.kind === "invoice" && (
                      <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                        {p.discount ? (
                          <>
                            −{formatPeso(discountOf(p.discount, d.totals.gross_c))}
                            <span class={[hint, "block"]}>{discountAs(p.discount)}</span>
                          </>
                        ) : (
                          "—"
                        )}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      )}

      <ReceiptFormModal open={editOpen} dr={det} />
      <Modal open={open} title={stepTitle(step.value, k.name)}>
        <form class="space-y-3" preventdefault:submit onSubmit$={submit}>
          {stepError.value && <Alert>{stepError.value}</Alert>}
          {step.value === "deliver" && (
            <>
              <p class="text-sm">
                Stock goes out of {d.from_location_name ?? "the location"} on every PC.
              </p>
              <Field label="Delivered on">
                <input
                  type="date"
                  class={input}
                  required
                  max={todayManila()}
                  value={date.value}
                  onChange$={(_, el) => (date.value = el.value)}
                />
              </Field>
              <Field label="Received by" hint="Who signed for it at the client.">
                <input class={input} value={receivedBy.value} onInput$={(_, el) => (receivedBy.value = el.value)} />
              </Field>
            </>
          )}
          {needsNo && (
            <Field
              label={`${d.kind === "dr" ? "DR" : "Invoice"} number (BIR booklet)`}
              hint={
                step.value === "number"
                  ? "Digits only, as printed. A correction keeps the old number in the audit log."
                  : "Digits only, as printed."
              }
            >
              <input
                class={input}
                inputMode="numeric"
                required
                value={no.value}
                onInput$={(_, el) => (no.value = el.value)}
              />
            </Field>
          )}
          {step.value === "restore" && (
            <p class="text-sm">The {k.name} is delivered again: its stock goes out again.</p>
          )}
          {step.value === "delete" && (
            <p class="text-sm">
              The {k.name} is removed on every PC. This can't be undone.
              {d.status === "delivered" && " Its stock comes back."}
              {d.paid_c > 0 && ` The ${formatPeso(d.paid_c)} paid on it becomes the client's credit.`}
              {d.no && d.no_manual && ` Number ${d.no} can be used again.`}
            </p>
          )}
          {step.value === "void" && (
            <>
              <p class="text-sm">
                Its stock comes back on every PC. The {k.name} stays in the list, marked void. It can be restored later.
              </p>
              <Field label="Reason">
                <input class={input} required value={reason.value} onInput$={(_, el) => (reason.value = el.value)} />
              </Field>
            </>
          )}
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
              Back
            </button>
            <button
              class={step.value === "delete" || step.value === "void" ? btnDanger : btnPrimary}
              disabled={busy.value}
            >
              {step.value === "deliver"
                ? "Mark delivered"
                : step.value === "number"
                  ? "Save number"
                  : step.value === "restore"
                    ? "Restore"
                    : step.value === "void"
                      ? `Void ${k.name}`
                      : `Delete ${k.name}`}
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

/** The list of DRs or of invoices, or one of them (`?id=`). */
export const ReceiptsPage = component$<{ kind: ReceiptKind }>(({ kind }) => {
  const loc = useLocation();
  const id = loc.url.searchParams.get("id");
  return id ? <DrView key={id} id={id} kind={kind} /> : <DrList kind={kind} />;
});
