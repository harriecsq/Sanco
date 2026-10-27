import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type PoAction,
  type PoDetail,
  type PoInfo,
  type SupplierChoice,
  type TtInfo,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  boxEquiv,
  formatDateTime,
  formatDay,
  formatMinor,
  formatPeso,
  formatQty,
  serialRange,
  todayManila,
} from "~/lib/format";
import { can, sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { OfflineBadge } from "~/components/sales/sales";
import {
  blankPo,
  blankTt,
  ORDER_TABLES,
  PO_STATUS,
  PoFormModal,
  poToForm,
  PoStatusBadge,
  RECEIVE_LABEL,
  receiveForm,
  ReceiveModal,
  TtModal,
  ttToForm,
  type PoForm,
  type ReceiveForm,
  type TtForm,
} from "~/components/orders/orders";
import { Alert, Badge, Confirm, Empty, Field, InfoList, Modal, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { btnGhost, btnPrimary, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

const PoList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const pos = useSignal<PoInfo[]>([]);
  const suppliers = useSignal<SupplierChoice[]>([]);
  const error = useSignal("");
  const form = useStore<PoForm>(blankPo());
  const open = useSignal(false);

  useLiveLoad(
    ORDER_TABLES,
    $(async () => {
      try {
        pos.value = unwrap(await commands.listPos());
        suppliers.value = unwrap(await commands.supplierOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status", "supplier"]);
    const q = f.q.toLowerCase();
    return pos.value.filter(
      (p) =>
        (!q ||
          [p.no, p.supplier_name, p.supplier_ref, p.summary, p.notes].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.from || p.date >= f.from) &&
        (!f.to || p.date <= f.to) &&
        (!f.supplier || p.supplier_id === f.supplier) &&
        (!f.status ||
          (f.status === "to_receive"
            ? ["ordered", "shipped", "arrived"].includes(p.status) && p.progress !== "full"
            : f.status === "late"
              ? (p.status === "ordered" || p.status === "shipped") && !!p.expected_date && p.expected_date < todayManila()
              : f.status === "over"
                ? p.over_received
                : p.status === f.status)),
    );
  });
  const pg = usePage(shown);
  const costs = pos.value.some((p) => p.total_minor != null);
  const today = todayManila();

  return (
    <>
      <PageHeader title="Purchase orders" subtitle="Orders to suppliers, their TT payments and receiving.">
        {can(session.user, "orders", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blankPo());
              open.value = true;
            }}
          >
            New PO
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search number, supplier, PI, items"
        dates
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              ...(["draft", "ordered", "shipped", "arrived", "closed", "cancelled"] as const).map((s) => ({
                value: s,
                label: PO_STATUS[s].label,
              })),
              { value: "to_receive", label: "Still to receive" },
              { value: "late", label: "Late (past expected date)" },
              { value: "over", label: "Received more than ordered" },
            ],
          },
          {
            key: "supplier",
            label: "Suppliers",
            options: suppliers.value.map((s) => ({ value: s.id, label: s.label })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{pos.value.length ? "No purchase orders match." : "No purchase orders yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Number</th>
                <th class={th}>Date</th>
                <th class={th}>Supplier</th>
                <th class={th}>Items</th>
                {costs && <th class={[th, "text-right"]}>Total</th>}
                <th class={th}>Expected</th>
                <th class={th}>Received</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((p) => {
                const open = ["ordered", "shipped", "arrived"].includes(p.status);
                const late = open && p.progress !== "full" && !!p.expected_date && p.expected_date < today;
                return (
                  <tr key={p.id} class={p.status === "cancelled" ? "text-ink-muted" : ""}>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link class="font-medium text-brand-700 hover:underline" href={`/orders/?id=${p.id}`}>
                        {p.no ?? "—"}
                      </Link>
                      <OfflineBadge show={!p.no_confirmed} />
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>{formatDay(p.date)}</td>
                    <td class={td}>
                      {p.supplier_name ?? "Deleted supplier"}
                      {p.supplier_ref && <span class={[hint, "block"]}>PI {p.supplier_ref}</span>}
                    </td>
                    <td class={td}>
                      {p.summary}
                      {p.box_count > 0 && (
                        <span class={[hint, "block"]}>
                          {p.box_count} {p.box_count === 1 ? "box" : "boxes"}
                        </span>
                      )}
                    </td>
                    {costs && (
                      <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                        {formatMinor(p.total_minor, p.currency)}
                        {p.unpriced_lines > 0 && <span class={[hint, "block"]}>{p.unpriced_lines} unpriced</span>}
                      </td>
                    )}
                    <td class={[td, "whitespace-nowrap"]}>
                      {formatDay(p.expected_date)}
                      {late && (
                        <span class="ml-2">
                          <Badge tone="danger">late</Badge>
                        </span>
                      )}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>
                      {p.status === "draft" || p.status === "cancelled" ? "—" : RECEIVE_LABEL[p.progress]}
                    </td>
                    <td class={td}>
                      <PoStatusBadge status={p.status} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
      <PoFormModal
        open={open}
        form={form}
        costs={sees(session.user, "costs")}
        saved$={$((id: string) => nav(`/orders/?id=${id}`))}
      />
    </>
  );
});

const ACTION_TEXT: Record<PoAction, { title: string; body: string; label: string }> = {
  order: {
    title: "Mark as ordered?",
    body: "The PO was sent to the supplier. Goods can be received from now on.",
    label: "Mark ordered",
  },
  back_to_draft: {
    title: "Move back to draft?",
    body: "Only possible before anything was received.",
    label: "Back to draft",
  },
  ship: {
    title: "Mark as shipped?",
    body: "The supplier shipped the goods.",
    label: "Mark shipped",
  },
  arrive: {
    title: "Mark as arrived?",
    body: "The goods arrived; receive them into a location to add them to stock.",
    label: "Mark arrived",
  },
  close: {
    title: "Close this PO?",
    body: "Nothing more will be received on it. It can be reopened later.",
    label: "Close",
  },
  reopen: {
    title: "Reopen this PO?",
    body: "More goods can be received on it again.",
    label: "Reopen",
  },
  cancel: {
    title: "Cancel this PO?",
    body: "It stays in the list, marked cancelled. Only POs with nothing received can be cancelled.",
    label: "Cancel PO",
  },
};

const PoView = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const nav = useNavigate();
  const detail = useSignal<PoDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const notice = useSignal("");
  const form = useStore<PoForm>(blankPo());
  const editOpen = useSignal(false);
  const deleteOpen = useSignal(false);
  const rform = useStore<ReceiveForm>({ location_id: "", date: "", notes: "", rows: [] });
  const receiveOpen = useSignal(false);
  const tform = useStore<TtForm>(blankTt(id, "USD", null));
  const ttOpen = useSignal(false);
  const ttDelete = useSignal<TtInfo | null>(null);
  const ttDeleteOpen = useSignal(false);
  const costOpen = useSignal(false);
  const action = useSignal<PoAction>("order");
  const actionOpen = useSignal(false);
  // Through a computed signal: a prop computed inline from `action` is
  // treated as constant by the optimizer, so the title would go stale.
  const actionTitle = useComputed$(() => ACTION_TEXT[action.value].title);
  const actionDate = useSignal("");
  const reason = useSignal("");
  const actionError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ORDER_TABLES,
    $(async () => {
      try {
        const d = unwrap(await commands.getPo(id));
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
        <PageHeader title="Purchase order not found" />
        <Link class="text-sm text-brand-700 hover:underline" href="/orders/">
          Back to purchase orders
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const p = det.po;
  const user = session.user;
  const costs = sees(user, "costs");
  const isOpen = p.status === "ordered" || p.status === "shipped" || p.status === "arrived";
  const receiving = p.status !== "draft" && p.status !== "cancelled";
  const canEdit = can(user, "orders", "edit");
  const canReceive = isOpen && can(user, "orders", "create") && det.lines.some((l) => l.remaining > 0);
  const voidCount = det.receipts.filter((r) => r.voided).length;

  const ask = $((a: PoAction) => {
    action.value = a;
    reason.value = "";
    actionDate.value = todayManila();
    actionError.value = "";
    actionOpen.value = true;
  });

  const rows: [string, string][] = [
    ["Supplier", p.supplier_name ? `${p.supplier_name}${p.supplier_code ? ` (${p.supplier_code})` : ""}` : "Deleted supplier"],
    ["PO date", formatDay(p.date)],
    ["Supplier's PI", p.supplier_ref ?? ""],
    ["Expected arrival", p.expected_date ? formatDay(p.expected_date) : ""],
    ["Currency", p.currency],
  ];
  if (costs && p.currency !== "PHP") rows.push(["Exchange rate", p.fx_rate ? `₱${p.fx_rate} per ${p.currency}` : "not entered"]);
  if (p.box_count > 0 || det.lines.some((l) => l.serial_code)) {
    rows.push(["Serial year", p.serial_year ?? todayManila().slice(0, 4)]);
    rows.push([
      "Boxes",
      p.box_count ? String(p.box_count) : p.status === "draft" ? "serials are given when marked ordered" : "none",
    ]);
  }
  rows.push(["Notes", p.notes ?? ""]);
  rows.push(["Entered", `${formatDateTime(p.created_at)} by ${p.created_by_name ?? "—"}`]);
  if (p.ordered_at) rows.push(["Ordered", `${formatDateTime(p.ordered_at)} by ${p.ordered_by_name ?? "—"}`]);
  if (p.shipped_date) rows.push(["Shipped", formatDay(p.shipped_date)]);
  if (p.arrived_date) rows.push(["Arrived", formatDay(p.arrived_date)]);
  if (p.status === "closed") rows.push(["Closed", `${formatDateTime(p.closed_at)} by ${p.closed_by_name ?? "—"}`]);
  if (p.status === "cancelled") {
    rows.push(["Cancelled", `${formatDateTime(p.closed_at)} by ${p.closed_by_name ?? "—"}`]);
    rows.push(["Reason", p.cancel_reason ?? ""]);
  }
  const balance = p.total_minor != null && p.paid_minor != null ? p.total_minor - p.paid_minor : null;

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/orders/">
          ← Purchase orders
        </Link>
      </p>
      <PageHeader title={`PO ${p.no ?? ""}`} subtitle={p.supplier_name ?? undefined}>
        {p.status === "draft" && can(user, "orders", "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (deleteOpen.value = true)}>
            Delete
          </button>
        )}
        {(p.status === "draft" || isOpen) && p.receipt_count === 0 && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("cancel")}>
            Cancel PO
          </button>
        )}
        {p.status === "ordered" && p.receipt_count === 0 && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("back_to_draft")}>
            Back to draft
          </button>
        )}
        {isOpen && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("close")}>
            Close
          </button>
        )}
        {p.status === "closed" && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("reopen")}>
            Reopen
          </button>
        )}
        {p.status !== "cancelled" && canEdit && (
          <button
            q:slot="actions"
            class={btnSecondary}
            onClick$={() => {
              Object.assign(form, poToForm(det));
              editOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
        {p.status === "ordered" && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("ship")}>
            Mark shipped
          </button>
        )}
        {(p.status === "ordered" || p.status === "shipped") && canEdit && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => ask("arrive")}>
            Mark arrived
          </button>
        )}
        {p.status === "draft" && can(user, "orders", "approve") && (
          <button q:slot="actions" class={btnPrimary} onClick$={() => ask("order")}>
            Mark ordered
          </button>
        )}
        {canReceive && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(rform, receiveForm(det));
              receiveOpen.value = true;
            }}
          >
            Receive goods
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      {notice.value && <Alert tone="brand">{notice.value}</Alert>}
      <div class="mb-4 flex flex-wrap items-center gap-3">
        <PoStatusBadge status={p.status} />
        {receiving && <span class="text-sm">{RECEIVE_LABEL[p.progress]}</span>}
        <OfflineBadge show={!p.no_confirmed} />
        {p.supplier_id && (
          <Link class="text-sm text-brand-700 hover:underline" href={`/orders/suppliers/?q=${encodeURIComponent(p.supplier_name ?? "")}`}>
            Supplier details
          </Link>
        )}
      </div>
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
                  {costs && <th class={[th, "text-right"]}>Unit price</th>}
                  {costs && <th class={[th, "text-right"]}>Amount</th>}
                  {receiving && <th class={[th, "text-right"]}>Received</th>}
                  {receiving && <th class={[th, "text-right"]}>Left</th>}
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
                      {l.boxes.length > 0 && (
                        <details class="mt-1">
                          <summary class={[hint, "cursor-pointer"]}>
                            {l.boxes.length} {l.boxes.length === 1 ? "box" : "boxes"} ·{" "}
                            {serialRange(l.boxes.map((b) => b.serial))}
                            {receiving && ` · ${l.boxes.filter((b) => b.received > 0).length} received`}
                          </summary>
                          <ul class="mt-1 grid max-w-md grid-cols-[auto_auto_1fr] gap-x-4 text-xs tabular-nums">
                            {l.boxes.map((b) => (
                              <li key={b.id} class="contents">
                                <span class="font-mono">{b.serial ?? "no serial"}</span>
                                <span class="text-right">{b.pcs} pcs</span>
                                <span class={b.received > 0 ? "text-brand-700" : "text-ink-muted"}>
                                  {b.received > 0
                                    ? b.received === b.pcs
                                      ? "received"
                                      : `received ${b.received}`
                                    : "not received"}
                                </span>
                              </li>
                            ))}
                          </ul>
                        </details>
                      )}
                    </td>
                    <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>
                      {formatQty(l.qty, l.unit)}
                      {l.serial_code && (
                        <span class={[hint, "block"]}>{boxEquiv(l.qty, l.pcs_per_box ?? l.item_pcs_per_box)}</span>
                      )}
                    </td>
                    {costs && (
                      <td class={[td, "text-right tabular-nums"]}>
                        {l.price_minor == null ? <Badge tone="warning">no price</Badge> : formatMinor(l.price_minor, p.currency)}
                      </td>
                    )}
                    {costs && <td class={[td, "text-right tabular-nums"]}>{formatMinor(l.amount_minor, p.currency)}</td>}
                    {receiving && <td class={[td, "text-right tabular-nums"]}>{l.received || "—"}</td>}
                    {receiving && (
                      <td class={[td, "text-right tabular-nums", l.remaining < 0 && "text-danger"]}>{l.remaining}</td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
            {costs && (
              <div class="flex flex-wrap items-start justify-between gap-4 border-t border-line px-3 py-3">
                <div>
                  {can(user, "items", "edit") && p.total_minor != null && p.total_minor > 0 && (
                    <button type="button" class={btnGhost} onClick$={() => (costOpen.value = true)}>
                      Update cost sheets from this PO
                    </button>
                  )}
                </div>
                <dl class="grid w-80 grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm">
                  <dt class="font-semibold">Total</dt>
                  <dd class="text-right font-semibold tabular-nums">{formatMinor(p.total_minor, p.currency)}</dd>
                  {p.currency !== "PHP" && (
                    <>
                      <dt class="text-ink-muted">In pesos{p.fx_rate ? ` at ${p.fx_rate}` : ""}</dt>
                      <dd class="text-right tabular-nums">{p.total_php_c == null ? "needs FX" : formatPeso(p.total_php_c)}</dd>
                    </>
                  )}
                  <dt class="text-ink-muted">Paid ({p.currency})</dt>
                  <dd class="text-right tabular-nums">{formatMinor(p.paid_minor, p.currency)}</dd>
                  <dt class="font-semibold">Balance</dt>
                  <dd class={["text-right font-semibold tabular-nums", balance != null && balance < 0 && "text-danger"]}>
                    {formatMinor(balance, p.currency)}
                  </dd>
                </dl>
              </div>
            )}
          </section>

          {costs && (
            <section class={[card, "overflow-x-auto"]}>
              <div class="flex items-center justify-between border-b border-line px-4 py-3">
                <h2 class="font-semibold">TT payments</h2>
                {p.status !== "cancelled" && can(user, "orders", "create") && (
                  <button
                    type="button"
                    class={btnGhost}
                    onClick$={() => {
                      Object.assign(tform, blankTt(id, p.currency, p.fx_rate));
                      ttOpen.value = true;
                    }}
                  >
                    + Add payment
                  </button>
                )}
              </div>
              {det.payments.length === 0 ? (
                <Empty>No payments yet.</Empty>
              ) : (
                <table class={table}>
                  <thead>
                    <tr>
                      <th class={th}>Date</th>
                      <th class={[th, "text-right"]}>Amount</th>
                      <th class={[th, "text-right"]}>Rate</th>
                      <th class={[th, "text-right"]}>In pesos</th>
                      <th class={th}>Reference</th>
                      <th class={th} />
                    </tr>
                  </thead>
                  <tbody>
                    {det.payments.map((t) => (
                      <tr key={t.id}>
                        <td class={[td, "whitespace-nowrap"]}>{formatDay(t.date)}</td>
                        <td class={[td, "text-right tabular-nums"]}>{formatMinor(t.amount_minor, t.currency)}</td>
                        <td class={[td, "text-right tabular-nums"]}>{t.fx_rate ?? "—"}</td>
                        <td class={[td, "text-right tabular-nums"]}>{t.php_c == null ? "—" : formatPeso(t.php_c)}</td>
                        <td class={td}>
                          {t.reference ?? "—"}
                          {t.bank_account_name && <span class={[hint, "block"]}>from {t.bank_account_name}</span>}
                          {t.notes && <span class={[hint, "block"]}>{t.notes}</span>}
                        </td>
                        <td class={[td, "whitespace-nowrap text-right"]}>
                          {canEdit && (
                            <button
                              type="button"
                              class={btnGhost}
                              onClick$={() => {
                                Object.assign(tform, ttToForm(t));
                                ttOpen.value = true;
                              }}
                            >
                              Edit
                            </button>
                          )}
                          {can(user, "orders", "delete") && (
                            <button
                              type="button"
                              class={btnGhost}
                              onClick$={() => {
                                ttDelete.value = t;
                                ttDeleteOpen.value = true;
                              }}
                            >
                              Delete
                            </button>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
          )}

          {receiving && (
            <section class={[card, "overflow-x-auto"]}>
              <h2 class="border-b border-line px-4 py-3 font-semibold">
                Receiving reports
                {voidCount > 0 && <span class={[hint, "ml-2 font-normal"]}>{voidCount} void</span>}
              </h2>
              {det.receipts.length === 0 ? (
                <Empty>Nothing received yet.</Empty>
              ) : (
                <table class={table}>
                  <thead>
                    <tr>
                      <th class={th}>Number</th>
                      <th class={th}>Date</th>
                      <th class={th}>Into</th>
                      <th class={th}>Items</th>
                      <th class={th}>Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {det.receipts.map((r) => (
                      <tr key={r.id} class={r.voided ? "text-ink-muted" : ""}>
                        <td class={[td, "whitespace-nowrap"]}>
                          <Link class="font-medium text-brand-700 hover:underline" href={`/orders/receiving/?id=${r.id}`}>
                            {r.no ?? "—"}
                          </Link>
                          <OfflineBadge show={!r.no_confirmed} />
                        </td>
                        <td class={[td, "whitespace-nowrap"]}>{formatDay(r.date)}</td>
                        <td class={td}>{r.location_name ?? "—"}</td>
                        <td class={td}>{r.summary}</td>
                        <td class={td}>{r.voided ? <Badge tone="danger">Void</Badge> : <Badge tone="brand">Posted</Badge>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
          )}
        </div>
      </div>

      <PoFormModal open={editOpen} form={form} costs={costs} />
      <ReceiveModal
        open={receiveOpen}
        detail={det}
        form={rform}
        saved$={$((rr: string) => nav(`/orders/receiving/?id=${rr}`))}
      />
      <TtModal open={ttOpen} form={tform} />
      <Confirm
        open={ttDeleteOpen}
        title="Delete this payment?"
        onConfirm$={$(async () => {
          if (ttDelete.value) unwrap(await commands.deletePoPayment(ttDelete.value.id));
        })}
      >
        <p>
          {ttDelete.value ? `${formatMinor(ttDelete.value.amount_minor, ttDelete.value.currency)} on ${formatDay(ttDelete.value.date)}` : ""}{" "}
          is removed on every PC.
        </p>
      </Confirm>
      <Confirm
        open={costOpen}
        title="Update cost sheets?"
        confirmLabel="Update"
        onConfirm$={$(async () => {
          const codes = unwrap(await commands.poToCostSheets(id));
          notice.value = `Cost sheets updated: ${codes.join(", ")}.`;
        })}
      >
        <p>
          Each item's supplier price, currency and exchange rate become this PO's. Charge lines (freight, duties…)
          stay as they are, so landed costs and prices from margins change.
        </p>
      </Confirm>
      <Confirm
        open={deleteOpen}
        title={`Delete draft ${p.no ?? ""}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deletePo(id));
          await nav("/orders/");
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
              unwrap(
                await commands.setPoStatus(id, {
                  action: action.value,
                  date: actionDate.value,
                  reason: reason.value,
                }),
              );
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
          {(action.value === "ship" || action.value === "arrive") && (
            <Field label={action.value === "ship" ? "Shipped on" : "Arrived on"}>
              <input
                type="date"
                class={input}
                max={todayManila()}
                value={actionDate.value}
                onChange$={(_, el) => (actionDate.value = el.value)}
              />
            </Field>
          )}
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
  const loc = useLocation();
  const id = loc.url.searchParams.get("id");
  return id ? <PoView key={id} id={id} /> : <PoList />;
});

export const head: DocumentHead = { title: "Purchase orders · Sanco" };
