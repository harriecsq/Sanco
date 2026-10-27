import {
  $,
  component$,
  useComputed$,
  useSignal,
  useStore,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import {
  Link,
  useLocation,
  useNavigate,
  type DocumentHead,
} from "@builder.io/qwik-city";
import {
  commands,
  type BookingInfo,
  type ClientBalance,
  type ClientInfo,
  type DrInfo,
  type PaymentInfo,
  type PayMethod,
  type RateInfo,
  type ReceiptKind,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatDay,
  formatPct,
  formatPeso,
  formatPesoOr,
  moneyText,
  parseCount,
  parseMoney,
  parsePct,
  pctText,
} from "~/lib/format";
import { can, sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  blankRate,
  RateModal,
  RatesTable,
  rateForm,
  type RateForm,
} from "~/components/rates/rates";
import {
  BOOKING_STATUS,
  DR_STATUS,
  DrNo,
  DrStatusBadge,
  PROGRESS_LABEL,
  RECEIPT,
  receiptHref,
  StatusBadge,
} from "~/components/sales/sales";
import {
  METHOD_LABEL,
  PaymentBadge,
  PaymentFormModal,
  blankPayment,
  checkText,
} from "~/components/accounting/accounting";
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
import { Pager, usePage } from "~/components/ui/pager";
import { TabNav } from "~/components/ui/tabs";
import {
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  table,
  td,
  th,
} from "~/components/ui/styles";

interface Form {
  id: string;
  name: string;
  contact_person: string;
  phone: string;
  email: string;
  tin: string;
  address: string;
  terms: string;
  tier: string;
  credit: string;
  notes: string;
  active: boolean;
}

const blank = (): Form => ({
  id: "",
  name: "",
  contact_person: "",
  phone: "",
  email: "",
  tin: "",
  address: "",
  terms: "30",
  tier: "",
  credit: "",
  notes: "",
  active: true,
});

const toForm = (c: ClientInfo): Form => ({
  id: c.id,
  name: c.name,
  contact_person: c.contact_person ?? "",
  phone: c.phone ?? "",
  email: c.email ?? "",
  tin: c.tin ?? "",
  address: c.address ?? "",
  terms: c.terms_days == null ? "" : String(c.terms_days),
  tier: pctText(c.margin_tier_bp),
  credit: moneyText(c.credit_limit_c),
  notes: c.notes ?? "",
  active: c.active,
});

const ClientFormModal = component$<{
  open: Signal<boolean>;
  form: Form;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const session = useSession();
  const error = useSignal("");
  const busy = useSignal(false);
  const margins = sees(session.user, "margins");

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const id = unwrap(
        await commands.saveClient({
          id: form.id || null,
          name: form.name,
          contact_person: form.contact_person,
          phone: form.phone,
          email: form.email,
          tin: form.tin,
          address: form.address,
          terms_days: parseCount(form.terms, "number of days"),
          margin_tier_bp: parsePct(form.tier, "margin"),
          credit_limit_c: parseMoney(form.credit, "credit limit"),
          notes: form.notes,
          active: form.active,
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

  return (
    <Modal open={open} title={form.id ? "Edit client" : "New client"} wide>
      <form class="space-y-3" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        {!form.id && (
          <p class={hint}>The client code is given automatically when you save.</p>
        )}
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Name" class="sm:col-span-2">
            <input
              class={input}
              required
              value={form.name}
              onInput$={(_, el) => (form.name = el.value)}
            />
          </Field>
          <Field label="Contact person">
            <input
              class={input}
              value={form.contact_person}
              onInput$={(_, el) => (form.contact_person = el.value)}
            />
          </Field>
          <Field label="Phone">
            <input
              class={input}
              value={form.phone}
              onInput$={(_, el) => (form.phone = el.value)}
            />
          </Field>
          <Field label="Email">
            <input
              class={input}
              type="email"
              value={form.email}
              onInput$={(_, el) => (form.email = el.value)}
            />
          </Field>
          <Field label="TIN">
            <input
              class={input}
              value={form.tin}
              onInput$={(_, el) => (form.tin = el.value)}
            />
          </Field>
          <Field label="Address" class="sm:col-span-2">
            <textarea
              class={input}
              rows={2}
              value={form.address}
              onInput$={(_, el) => (form.address = el.value)}
            />
          </Field>
          <Field label="Payment terms (days)">
            <input
              class={input}
              inputMode="numeric"
              value={form.terms}
              onInput$={(_, el) => (form.terms = el.value)}
            />
          </Field>
          <Field label="Credit limit (₱)" hint="Leave blank for no limit.">
            <input
              class={input}
              inputMode="decimal"
              value={form.credit}
              onInput$={(_, el) => (form.credit = el.value)}
            />
          </Field>
          {margins && (
            <Field
              label="Margin tier (%)"
              hint="Prices items at landed cost ÷ (1 − tier) unless the client has a rate. Blank uses each item's default margin."
            >
              <input
                class={input}
                inputMode="decimal"
                value={form.tier}
                onInput$={(_, el) => (form.tier = el.value)}
              />
            </Field>
          )}
          <Field label="Notes" class="sm:col-span-2">
            <textarea
              class={input}
              rows={2}
              value={form.notes}
              onInput$={(_, el) => (form.notes = el.value)}
            />
          </Field>
        </div>
        {form.id && (
          <label class="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              class="accent-brand-500"
              checked={form.active}
              onChange$={(_, el) => (form.active = el.checked)}
            />
            Active (shown in pickers for new bookings)
          </label>
        )}
        <div class="flex justify-end gap-2 pt-2">
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
      </form>
    </Modal>
  );
});

const ClientList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const clients = useSignal<ClientInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const open = useSignal(false);

  useLiveLoad(
    ["clients", "doc_numbers", "client_item_rates"],
    $(async () => {
      try {
        clients.value = unwrap(await commands.listClients());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "status"]);
    const q = f.q.toLowerCase();
    return clients.value.filter(
      (c) =>
        (!q ||
          [c.code, c.name, c.contact_person, c.tin, c.phone].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.status ||
          (f.status === "duplicate" ? c.duplicate_name : (f.status === "active") === c.active)),
    );
  });

  const pg = usePage(shown);
  const margins = sees(session.user, "margins");

  return (
    <>
      <PageHeader title="Clients" subtitle="Retailers and other buyers.">
        {can(session.user, "clients", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blank());
              open.value = true;
            }}
          >
            New client
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, contact, TIN"
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "active", label: "Active" },
              { value: "inactive", label: "Inactive" },
              { value: "duplicate", label: "Duplicate names" },
            ],
          },
        ]}
      />
      <section class={card}>
        {shown.value.length === 0 ? (
          <Empty>
            {clients.value.length ? "No clients match." : "No clients yet."}
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Code</th>
                <th class={th}>Name</th>
                <th class={th}>Contact</th>
                <th class={[th, "text-right"]}>Terms</th>
                {margins && <th class={[th, "text-right"]}>Tier</th>}
                <th class={[th, "text-right"]}>Credit limit</th>
                <th class={[th, "text-right"]}>Rates</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((c) => (
                <tr key={c.id} class={c.active ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    {c.code ?? "—"}
                    {!c.code_confirmed && (
                      <span class={["ml-1", hint]} title="Given offline: may change after sync">
                        *
                      </span>
                    )}
                  </td>
                  <td class={td}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/clients/?id=${c.id}`}
                    >
                      {c.name}
                    </Link>
                    {c.duplicate_name && (
                      <span class="ml-2">
                        <Badge tone="warning">duplicate name</Badge>
                      </span>
                    )}
                  </td>
                  <td class={td}>
                    {[c.contact_person, c.phone].filter(Boolean).join(" · ") || "—"}
                  </td>
                  <td class={[td, "text-right"]}>
                    {c.terms_days == null ? "—" : `${c.terms_days} d`}
                  </td>
                  {margins && (
                    <td class={[td, "text-right"]}>{formatPct(c.margin_tier_bp)}</td>
                  )}
                  <td class={[td, "text-right tabular-nums"]}>
                    {formatPesoOr(c.credit_limit_c)}
                  </td>
                  <td class={[td, "text-right"]}>{c.rate_count || "—"}</td>
                  <td class={td}>
                    <Badge tone={c.active ? "brand" : "muted"}>
                      {c.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
      <ClientFormModal
        open={open}
        form={form}
        saved$={$((id: string) => nav(`/clients/?id=${id}`))}
      />
    </>
  );
});

/**
 * A client's balance: owed, overdue, credit and limit. With `unpaidHref`
 * (the client's page), the unpaid counts link to its DR and invoice tabs.
 */
const BalanceList = component$<{ b: ClientBalance; unpaidHref?: string }>(({ b, unpaidHref }) => {
  const over = b.credit_limit_c != null && b.balance_c > b.credit_limit_c;
  const owed = [
    { tab: "drs", n: b.open_drs, label: b.open_drs === 1 ? "DR" : "DRs" },
    { tab: "invoices", n: b.open_invoices, label: b.open_invoices === 1 ? "invoice" : "invoices" },
  ].filter((o, i) => o.n > 0 || (i === 0 && b.open_invoices === 0));
  return (
    <dl class="grid grid-cols-[1fr_auto] gap-x-4 gap-y-1.5 text-sm">
      <dt class="text-ink-muted">
        Owed on{" "}
        {owed.map((o, i) => (
          <span key={o.tab}>
            {i > 0 && " and "}
            {unpaidHref && o.n > 0 ? (
              <Link class="text-brand-700 hover:underline" href={`${unpaidHref}&tab=${o.tab}&status=unpaid`}>
                {o.n} {o.label}
              </Link>
            ) : (
              `${o.n} ${o.label}`
            )}
          </span>
        ))}
      </dt>
      <dd class="text-right tabular-nums">{formatPeso(b.outstanding_c)}</dd>
      <dt class="text-ink-muted">Of that, overdue</dt>
      <dd class={["text-right tabular-nums", b.overdue_c > 0 && "text-danger"]}>{formatPeso(b.overdue_c)}</dd>
      <dt class="text-ink-muted">Credit (paid, not applied)</dt>
      <dd class="text-right tabular-nums">{formatPeso(b.credit_c)}</dd>
      <dt class="font-semibold">Balance</dt>
      <dd class="text-right font-semibold tabular-nums">{formatPeso(b.balance_c)}</dd>
      <dt class="text-ink-muted">Credit limit</dt>
      <dd class="text-right tabular-nums">
        {b.credit_limit_c == null ? "No limit" : formatPeso(b.credit_limit_c)}
        {over && (
          <span class="ml-1">
            <Badge tone="danger">over</Badge>
          </span>
        )}
      </dd>
      {b.uncleared_c > 0 && (
        <>
          <dt class="text-ink-muted">Paid by uncleared checks</dt>
          <dd class="text-right tabular-nums">{formatPeso(b.uncleared_c)}</dd>
        </>
      )}
    </dl>
  );
});

type DetailTab = "bookings" | "drs" | "invoices" | "payments" | "rates";

/** Rows per page in a tab. */
const PAGE = 30;

const dayOf = (d: DrInfo) => d.delivered_date ?? d.date;

const BookingsTab = component$<{ rows: BookingInfo[] }>((props) => {
  const loc = useLocation();
  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status"]);
    const q = f.q.toLowerCase();
    return props.rows.filter(
      (b) =>
        (!q ||
          [b.no, b.client_ref, b.summary, b.notes].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || b.date >= f.from) &&
        (!f.to || b.date <= f.to) &&
        (!f.status ||
          (f.status === "to_deliver"
            ? b.status === "confirmed" && b.progress !== "full"
            : b.status === f.status)),
    );
  });
  const pg = usePage(shown, PAGE);
  const total = shown.value.reduce((s, b) => s + (b.status === "cancelled" ? 0 : b.totals.gross_c), 0);
  return (
    <>
      <FilterBar
        search="Search number, PO, items"
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
        ]}
      >
        <span class={hint}>
          {shown.value.length} booking(s) · {formatPeso(total)}
        </span>
      </FilterBar>
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{props.rows.length ? "No bookings match." : "No bookings yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Number</th>
                <th class={th}>Date</th>
                <th class={th}>Items</th>
                <th class={[th, "text-right"]}>Gross</th>
                <th class={th}>Delivery</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((b) => (
                <tr key={b.id} class={b.status === "cancelled" ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/bookings/?id=${b.id}`}>
                      {b.no ?? "—"}
                    </Link>
                    {b.client_ref && <span class={[hint, "block"]}>{b.client_ref}</span>}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(b.date)}</td>
                  <td class={td}>{b.summary}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(b.totals.gross_c)}</td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {b.status === "draft" || b.status === "cancelled" ? "—" : PROGRESS_LABEL[b.progress]}
                  </td>
                  <td class={td}>
                    <StatusBadge s={BOOKING_STATUS[b.status]} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
    </>
  );
});

/** The client's DRs or invoices. */
const DrsTab = component$<{ rows: DrInfo[]; kind: ReceiptKind }>((props) => {
  const k = RECEIPT[props.kind];
  const loc = useLocation();
  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status"]);
    const q = f.q.toLowerCase();
    return props.rows.filter(
      (d) =>
        (!q ||
          [d.no, d.booking_no, d.summary, d.received_by, d.notes].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.from || dayOf(d) >= f.from) &&
        (!f.to || dayOf(d) <= f.to) &&
        (!f.status ||
          (f.status === "unpaid"
            ? d.status === "delivered" && d.balance_c > 0
            : f.status === "void"
              ? d.voided
              : f.status === "no_number"
                ? !d.no && d.status !== "cancelled"
                : d.status === f.status && !d.voided)),
    );
  });
  const pg = usePage(shown, PAGE);
  const unpaid = shown.value.reduce((s, d) => s + (d.status === "delivered" ? d.balance_c : 0), 0);
  return (
    <>
      <FilterBar
        search={`Search ${k.name} no., booking, items`}
        dates
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "unpaid", label: "Unpaid" },
              ...(["requested", "approved", "delivered", "cancelled"] as const).map((s) => ({
                value: s,
                label: DR_STATUS[s].label,
              })),
              { value: "void", label: "Void" },
              { value: "no_number", label: `No ${k.name} no. yet` },
            ],
          },
        ]}
      >
        <span class={hint}>
          {shown.value.length} {shown.value.length === 1 ? k.name : k.plural} · {formatPeso(unpaid)} unpaid
        </span>
      </FilterBar>
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{props.rows.length ? `No ${k.plural} match.` : `No ${k.plural} yet.`}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>{k.title} no.</th>
                <th class={th}>Date</th>
                <th class={th}>Booking</th>
                <th class={th}>Items</th>
                <th class={[th, "text-right"]}>Gross</th>
                <th class={[th, "text-right"]}>Balance</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((d) => (
                <tr key={d.id} class={d.status === "cancelled" ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={receiptHref(d.kind, d.id)}>
                      <DrNo d={d} />
                    </Link>
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(dayOf(d))}</td>
                  <td class={[td, "whitespace-nowrap"]}>{d.booking_no ?? "—"}</td>
                  <td class={td}>{d.summary}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.totals.gross_c)}</td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {d.status === "delivered" ? (d.balance_c === 0 ? "Paid" : formatPeso(d.balance_c)) : "—"}
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
    </>
  );
});

const PaymentsTab = component$<{ rows: PaymentInfo[] }>((props) => {
  const loc = useLocation();
  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "method", "status"]);
    const q = f.q.toLowerCase();
    return props.rows.filter(
      (p) =>
        (!q ||
          [p.no, p.reference, p.or_no, p.notes, p.check?.check_no, p.check?.bank, p.bank_account_name, ...p.receipts].some(
            (v) => v?.toLowerCase().includes(q),
          )) &&
        (!f.from || p.date >= f.from) &&
        (!f.to || p.date <= f.to) &&
        (!f.method || p.method === f.method) &&
        (!f.status ||
          (f.status === "void"
            ? p.status === "void"
            : f.status === "unapplied"
              ? p.counts && p.unapplied_c !== 0
              : f.status === "not_counting"
                ? p.status === "posted" && !p.counts
                : p.counts)),
    );
  });
  const pg = usePage(shown, PAGE);
  const total = shown.value.filter((p) => p.counts).reduce((s, p) => s + p.amount_c, 0);
  return (
    <>
      <FilterBar
        search="Search no., OR, check, DR, invoice"
        dates
        selects={[
          {
            key: "method",
            label: "Methods",
            options: (Object.keys(METHOD_LABEL) as PayMethod[]).map((m) => ({ value: m, label: METHOD_LABEL[m] })),
          },
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "counting", label: "Counting as paid" },
              { value: "unapplied", label: "Not fully applied" },
              { value: "not_counting", label: "Bounced / replaced check" },
              { value: "void", label: "Void" },
            ],
          },
        ]}
      >
        <span class={hint}>
          {shown.value.length} payment(s) · {formatPeso(total)} counting
        </span>
      </FilterBar>
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{props.rows.length ? "No payments match." : "No payments yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>No.</th>
                <th class={th}>Received</th>
                <th class={th}>Method</th>
                <th class={[th, "text-right"]}>Amount</th>
                <th class={th}>Applied to</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((p) => (
                <tr key={p.id} class={p.counts ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/accounting/payments/?id=${p.id}`}>
                      {p.no ?? "—"}
                    </Link>
                    {p.or_no && <span class={[hint, "block"]}>{p.or_no}</span>}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(p.date)}</td>
                  <td class={td}>
                    {METHOD_LABEL[p.method]}
                    {p.check && <span class={[hint, "block"]}>{checkText(p)}</span>}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(p.amount_c)}</td>
                  <td class={td}>{p.receipts.length ? p.receipts.join(", ") : "—"}</td>
                  <td class={td}>
                    <PaymentBadge p={p} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
    </>
  );
});

const ClientDetail = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const client = useSignal<ClientInfo | null>(null);
  const missing = useSignal(false);
  const rates = useSignal<RateInfo[]>([]);
  const bookings = useSignal<BookingInfo[]>([]);
  const drs = useSignal<DrInfo[]>([]);
  const drOnly = useComputed$(() => drs.value.filter((d) => d.kind === "dr"));
  const invoices = useComputed$(() => drs.value.filter((d) => d.kind === "invoice"));
  const balance = useSignal<ClientBalance | null>(null);
  const payments = useSignal<PaymentInfo[]>([]);
  const payOpen = useSignal(false);
  const pay = useStore(blankPayment(id));
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const editOpen = useSignal(false);
  const deleteOpen = useSignal(false);
  const rate = useStore<RateForm>(blankRate(id));
  const rateOpen = useSignal(false);

  useLiveLoad(
    [
      "clients",
      "doc_numbers",
      "client_item_rates",
      "items",
      "item_costs",
      "item_cost_charges",
      "settings",
      "bookings",
      "booking_lines",
      "delivery_receipts",
      "dr_lines",
      "payments",
      "payment_allocations",
    ],
    $(async () => {
      try {
        const c = unwrap(await commands.getClient(id));
        client.value = c;
        missing.value = !c;
        rates.value = unwrap(await commands.listRates({ client_id: id, item_id: null }));
        const user = session.user;
        if (can(user, "bookings")) {
          bookings.value = unwrap(await commands.listBookings()).filter((b) => b.client_id === id);
        }
        if (can(user, "bookings") || can(user, "accounting")) {
          drs.value = unwrap(await commands.listDrs()).filter((d) => d.client_id === id);
        }
        balance.value = unwrap(await commands.clientBalance(id));
        if (can(user, "accounting")) {
          payments.value = unwrap(await commands.listPayments()).filter((p) => p.client_id === id);
        }
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  // Tabs the user may see, in order; the first is the default.
  const tabs = useComputed$(() => {
    const user = session.user;
    const all: { key: DetailTab; label: string; count: number; show: boolean }[] = [
      { key: "bookings", label: "Bookings", count: bookings.value.length, show: can(user, "bookings") },
      {
        key: "drs",
        label: "Delivery receipts",
        count: drOnly.value.length,
        show: can(user, "bookings") || can(user, "accounting"),
      },
      {
        key: "invoices",
        label: "Invoices",
        count: invoices.value.length,
        show: can(user, "bookings") || can(user, "accounting"),
      },
      { key: "payments", label: "Payments and checks", count: payments.value.length, show: can(user, "accounting") },
      { key: "rates", label: "Rates", count: rates.value.length, show: true },
    ];
    const visible = all.filter((t) => t.show);
    const asked = loc.url.searchParams.get("tab");
    const current = visible.find((t) => t.key === asked)?.key ?? visible[0].key;
    return {
      current,
      nav: visible.map((t) => ({
        href: `/clients/?id=${id}&tab=${t.key}`,
        label: t.label,
        count: t.count,
        active: t.key === current,
      })),
    };
  });

  const c = client.value;
  if (missing.value) {
    return (
      <>
        <PageHeader title="Client not found" />
        <p class="text-sm">
          It may have been deleted on another PC.{" "}
          <Link class="text-brand-700 hover:underline" href="/clients/">
            Back to clients
          </Link>
        </p>
      </>
    );
  }
  if (!c) return error.value ? <Alert>{error.value}</Alert> : null;

  const margins = sees(session.user, "margins");
  const canEditRates = can(session.user, "items", "edit");
  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/clients/">
          ← Clients
        </Link>
      </p>
      <PageHeader
        title={c.name}
        subtitle={`${c.code ?? "No code"}${c.code_confirmed ? "" : " (given offline, may change)"}`}
      >
        {can(session.user, "clients", "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (deleteOpen.value = true)}>
            Delete
          </button>
        )}
        {can(session.user, "clients", "edit") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, toForm(c));
              editOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      {!c.active && (
        <div class="mb-4">
          <Alert tone="warning">This client is inactive.</Alert>
        </div>
      )}
      <div class="grid gap-4 lg:grid-cols-[22rem_minmax(0,1fr)]">
        <div class="space-y-4 self-start lg:sticky lg:top-6">
          <section class={[card, "p-5"]}>
            <h2 class="mb-3 font-semibold">Details</h2>
            <InfoList
              rows={[
                ["Contact person", c.contact_person ?? ""],
                ["Phone", c.phone ?? ""],
                ["Email", c.email ?? ""],
                ["TIN", c.tin ?? ""],
                ["Address", c.address ?? ""],
                ["Payment terms", c.terms_days == null ? "" : `${c.terms_days} days`],
                ["Credit limit", c.credit_limit_c == null ? "No limit" : formatPesoOr(c.credit_limit_c)],
                ...(margins
                  ? ([["Margin tier", c.margin_tier_bp == null ? "Item defaults" : formatPct(c.margin_tier_bp)]] as [string, string][])
                  : []),
                ["Notes", c.notes ?? ""],
              ]}
            />
          </section>
          {balance.value && (
            <section class={[card, "p-5"]}>
              <div class="mb-3 flex items-center justify-between">
                <h2 class="font-semibold">Balance</h2>
                {can(session.user, "accounting", "create") && (
                  <button
                    class={btnSecondary}
                    onClick$={() => {
                      Object.assign(pay, blankPayment(id));
                      payOpen.value = true;
                    }}
                  >
                    Record payment
                  </button>
                )}
              </div>
              <BalanceList
                b={balance.value}
                unpaidHref={
                  can(session.user, "bookings") || can(session.user, "accounting")
                    ? `/clients/?id=${id}`
                    : undefined
                }
              />
            </section>
          )}
        </div>
        <div class="min-w-0">
          <TabNav label="Client records" tabs={tabs.value.nav} />
          {tabs.value.current === "bookings" && <BookingsTab rows={bookings.value} />}
          {tabs.value.current === "drs" && <DrsTab kind="dr" rows={drOnly.value} />}
          {tabs.value.current === "invoices" && (
            <DrsTab kind="invoice" rows={invoices.value} />
          )}
          {tabs.value.current === "payments" && <PaymentsTab rows={payments.value} />}
          {tabs.value.current === "rates" && (
            <section class={card}>
              <div class="flex items-center justify-between border-b border-line px-4 py-3">
                <p class={hint}>
                  Manual prices for this client. Other items use{" "}
                  {c.margin_tier_bp != null || !margins ? "the client's margin tier" : "each item's default margin"}.
                </p>
                {canEditRates && (
                  <button
                    class={btnSecondary}
                    onClick$={() => {
                      Object.assign(rate, blankRate(id));
                      rateOpen.value = true;
                    }}
                  >
                    Add rate
                  </button>
                )}
              </div>
              <RatesTable
                rates={rates.value}
                size={PAGE}
                showItem
                edit$={
                  canEditRates
                    ? $((r: RateInfo) => {
                        Object.assign(rate, rateForm(r));
                        rateOpen.value = true;
                      })
                    : undefined
                }
              />
            </section>
          )}
        </div>
      </div>
      <ClientFormModal open={editOpen} form={form} />
      <RateModal open={rateOpen} form={rate} lock="client" />
      <PaymentFormModal
        open={payOpen}
        form={pay}
        saved$={$(async (pid: string) => {
          await nav(`/accounting/payments/?id=${pid}`);
        })}
      />
      <Confirm
        open={deleteOpen}
        title={`Delete ${c.name}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteClient(id));
          await nav("/clients/");
        })}
      >
        <p>
          This deletes the client and its {rates.value.length} rate(s) on every PC. It
          can't be undone. To keep the history, mark the client inactive instead.
        </p>
      </Confirm>
    </>
  );
});

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  if (!can(session.user, "clients")) return <NoAccess />;
  const id = loc.url.searchParams.get("id");
  return id ? <ClientDetail key={id} id={id} /> : <ClientList />;
});

export const head: DocumentHead = { title: "Clients · Sanco" };

