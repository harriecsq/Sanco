import {
  $,
  component$,
  useSignal,
  useVisibleTask$,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import {
  commands,
  type AgingBucket,
  type CheckStatus,
  type CheckStep,
  type Choice,
  type ClientBalance,
  type OpenDr,
  type PaymentDetail,
  type PaymentInfo,
  type PayMethod,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatPeso, moneyText, parseMoney, todayManila } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Badge, Field, Modal } from "~/components/ui/ui";
import { BankAccountSelect } from "~/components/accounting/bank-account-select";
import {
  btnDanger,
  btnGhost,
  btnPrimary,
  btnSecondary,
  hint,
  input,
  inputBase,
  table,
  td,
  th,
} from "~/components/ui/styles";

type Tone = "brand" | "muted" | "danger" | "warning";

/** Tables whose changes move balances. */
export const MONEY_TABLES = [
  "payments",
  "payment_allocations",
  "delivery_receipts",
  "dr_lines",
  "clients",
  "doc_numbers",
  "users",
  "bank_accounts",
];

export const METHOD_LABEL: Record<PayMethod, string> = {
  cash: "Cash",
  check: "Check",
  bank_transfer: "Bank transfer",
};

export const CHECK_STATUS: Record<CheckStatus, { label: string; tone: Tone }> = {
  on_hand: { label: "On hand", tone: "warning" },
  deposited: { label: "Deposited", tone: "brand" },
  cleared: { label: "Cleared", tone: "brand" },
  bounced: { label: "Bounced", tone: "danger" },
  replaced: { label: "Replaced", tone: "muted" },
};

export const BUCKETS: { key: AgingBucket; label: string }[] = [
  { key: "not_due", label: "Not due" },
  { key: "days1_to30", label: "1–30 days" },
  { key: "days31_to60", label: "31–60 days" },
  { key: "days61_to90", label: "61–90 days" },
  { key: "over90", label: "Over 90" },
];

export const BUCKET_LABEL = Object.fromEntries(BUCKETS.map((b) => [b.key, b.label])) as Record<
  AgingBucket,
  string
>;

/** A check's status, with a PDC badge while it is post-dated and on hand. */
export const CheckBadge = component$<{ p: PaymentInfo }>(({ p }) => {
  const c = p.check;
  if (!c) return null;
  const s = CHECK_STATUS[c.status];
  return (
    <span class="inline-flex flex-wrap gap-1">
      <Badge tone={s.tone}>{s.label}</Badge>
      {c.pdc && c.status === "on_hand" && <Badge tone="muted">PDC</Badge>}
    </span>
  );
});

/** Void, or a check that doesn't count, else nothing (or the check status). */
export const PaymentBadge = component$<{ p: PaymentInfo }>(({ p }) => {
  if (p.status === "void") return <Badge tone="danger">Void</Badge>;
  if (p.check) return <CheckBadge p={p} />;
  return <Badge tone="brand">Received</Badge>;
});

/** "BPI 0012345, dated Nov 15, 2026" */
export const checkText = (p: PaymentInfo) =>
  p.check
    ? `${p.check.bank ?? ""} ${p.check.check_no ?? ""}${p.check.check_date ? `, dated ${formatDay(p.check.check_date)}` : ""}`.trim()
    : "";

// Payment form ---------------------------------------------------------------

export interface PaymentForm {
  id: string;
  client_id: string;
  date: string;
  method: PayMethod;
  amount: string;
  reference: string;
  or_no: string;
  notes: string;
  bank: string;
  check_no: string;
  check_date: string;
  /** Bank transfers: received into. */
  bank_account_id: string;
  replaces_id: string;
  /** dr_id → amount text. */
  allocs: Record<string, string>;
}

export const blankPayment = (client_id = "", replaces_id = ""): PaymentForm => ({
  id: "",
  client_id,
  date: todayManila(),
  method: "cash",
  amount: "",
  reference: "",
  or_no: "",
  notes: "",
  bank: "",
  check_no: "",
  check_date: todayManila(),
  bank_account_id: "",
  replaces_id,
  allocs: {},
});

export const paymentForm = (d: PaymentDetail): PaymentForm => {
  const p = d.payment;
  return {
    id: p.id,
    client_id: p.client_id ?? "",
    date: p.date,
    method: p.method,
    amount: moneyText(p.amount_c),
    reference: p.reference ?? "",
    or_no: p.or_no ?? "",
    notes: p.notes ?? "",
    bank: p.check?.bank ?? "",
    check_no: p.check?.check_no ?? "",
    check_date: p.check?.check_date ?? "",
    bank_account_id: p.method === "bank_transfer" ? (p.bank_account_id ?? "") : "",
    replaces_id: p.replaces_id ?? "",
    allocs: Object.fromEntries(d.allocations.map((a) => [a.dr_id, moneyText(a.amount_c)])),
  };
};

/** Amount text, 0 when blank or invalid (errors show on save). */
function amountOf(text: string | undefined): number {
  try {
    return parseMoney(text ?? "") ?? 0;
  } catch {
    return 0;
  }
}

/** Spreads `amount` over the DRs, oldest first. */
function fillOldest(amount: number, drs: OpenDr[]): Record<string, string> {
  const out: Record<string, string> = {};
  let left = amount;
  for (const d of drs) {
    const take = Math.min(left, Math.max(d.open_c, 0));
    if (take > 0) out[d.dr_id] = moneyText(take);
    left -= take;
  }
  return out;
}

const drName = (d: { no: string | null }) => (d.no ? `DR ${d.no}` : "DR (no number)");

/** A replacement takes over the replaced check's amount and allocations, where still blank. */
async function prefillFrom(form: PaymentForm, replaces: string) {
  const r = await commands.getPayment(replaces);
  const d = r.status === "ok" ? r.data : null;
  if (!d) return;
  if (!form.amount) form.amount = moneyText(d.payment.amount_c);
  if (Object.keys(form.allocs).length === 0) {
    form.allocs = Object.fromEntries(
      d.allocations.filter((a) => a.delivered).map((a) => [a.dr_id, moneyText(a.amount_c)]),
    );
  }
}

/** Records or edits a payment and its allocations to the client's DRs. */
export const PaymentFormModal = component$<{
  open: Signal<boolean>;
  form: PaymentForm;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const clients = useSignal<Choice[]>([]);
  const checks = useSignal<PaymentInfo[]>([]);
  const drs = useSignal<OpenDr[]>([]);
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["clients", "doc_numbers", "payments"],
    $(async () => {
      try {
        clients.value = unwrap(await commands.clientOptions());
        // Checks a new payment may replace: on hand or bounced.
        checks.value = unwrap(await commands.listPayments()).filter(
          (p) =>
            p.status === "posted" &&
            p.check &&
            (p.check.status === "bounced" || p.check.status === "on_hand"),
        );
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  // The client's open DRs, again when the client or the replaced check changes.
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      if (!track(() => open.value)) return;
      const client = track(() => form.client_id);
      const replaces = track(() => form.replaces_id);
      error.value = "";
      if (!client) {
        drs.value = [];
        return;
      }
      try {
        drs.value = unwrap(await commands.paymentOpenDrs(client, form.id || null, replaces || null));
        if (replaces && !form.id) await prefillFrom(form, replaces);
      } catch (e) {
        error.value = errorText(e);
      }
    },
    { strategy: "document-ready" },
  );

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const amount = parseMoney(form.amount, "amount");
      if (!amount) throw new Error("enter the amount received");
      const allocations = Object.entries(form.allocs).map(([dr_id, text]) => {
        const d = drs.value.find((x) => x.dr_id === dr_id);
        return {
          dr_id,
          amount_c: parseMoney(text, `amount for ${d ? drName(d) : "a DR"}`) ?? 0,
        };
      });
      const id = unwrap(
        await commands.savePayment({
          id: form.id || null,
          client_id: form.client_id,
          date: form.date,
          method: form.method,
          amount_c: amount,
          reference: form.reference,
          or_no: form.or_no,
          notes: form.notes,
          check:
            form.method === "check"
              ? { bank: form.bank, check_no: form.check_no, check_date: form.check_date }
              : null,
          bank_account_id: form.method === "bank_transfer" ? form.bank_account_id || null : null,
          replaces_id: form.replaces_id || null,
          allocations,
        }),
      );
      open.value = false;
      busy.value = false;
      if (saved$) await saved$(id);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const amount = amountOf(form.amount);
  const allocated = Object.values(form.allocs).reduce((s, t) => s + amountOf(t), 0);
  const left = amount - allocated;
  const replaceable = checks.value.filter((c) => c.client_id === form.client_id && c.id !== form.id);
  const isPdc = form.method === "check" && form.check_date > form.date;

  return (
    <Modal open={open} title={form.id ? "Edit payment" : "Record payment"} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid gap-3 sm:grid-cols-3">
          <Field label="Client" class="sm:col-span-2">
            <select
              class={input}
              required
              disabled={!!form.id}
              onChange$={(_, el) => {
                form.client_id = el.value;
                form.replaces_id = "";
                form.allocs = {};
              }}
            >
              <option value="">Choose a client…</option>
              {clients.value
                .filter((c) => c.active || c.id === form.client_id)
                .map((c) => (
                  <option key={c.id} value={c.id} selected={c.id === form.client_id}>
                    {c.label}
                  </option>
                ))}
            </select>
          </Field>
          <Field label="Received on">
            <input
              type="date"
              class={input}
              required
              max={todayManila()}
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label="Method">
            <select
              class={input}
              disabled={!!form.id}
              onChange$={(_, el) => (form.method = el.value as PayMethod)}
            >
              {(Object.keys(METHOD_LABEL) as PayMethod[]).map((m) => (
                <option key={m} value={m} selected={m === form.method}>
                  {METHOD_LABEL[m]}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Amount received">
            <input
              class={[inputBase, "w-full text-right"]}
              inputMode="decimal"
              required
              placeholder="₱ 0.00"
              value={form.amount}
              onInput$={(_, el) => (form.amount = el.value)}
            />
          </Field>
          <Field label="OR / CR no." hint="From the receipt booklet, if issued.">
            <input class={input} value={form.or_no} onInput$={(_, el) => (form.or_no = el.value)} />
          </Field>
        </div>

        {form.method === "check" && (
          <div class="grid gap-3 rounded-md bg-surface-muted p-3 sm:grid-cols-3">
            <Field label="Bank">
              <input class={input} required value={form.bank} onInput$={(_, el) => (form.bank = el.value)} />
            </Field>
            <Field label="Check no.">
              <input
                class={input}
                required
                value={form.check_no}
                onInput$={(_, el) => (form.check_no = el.value)}
              />
            </Field>
            <Field label="Check date" hint={isPdc ? "Post-dated (PDC)." : undefined}>
              <input
                type="date"
                class={input}
                required
                value={form.check_date}
                onChange$={(_, el) => (form.check_date = el.value)}
              />
            </Field>
          </div>
        )}
        {form.method === "bank_transfer" && (
          <div class="grid gap-3 sm:grid-cols-2">
            <Field label="Received into">
              <BankAccountSelect
                value={form.bank_account_id}
                onPick$={(id) => {
                  form.bank_account_id = id;
                }}
              />
            </Field>
            <Field label="Transfer reference">
              <input class={input} value={form.reference} onInput$={(_, el) => (form.reference = el.value)} />
            </Field>
          </div>
        )}
        {form.method === "cash" && (
          <Field label="Reference">
            <input class={input} value={form.reference} onInput$={(_, el) => (form.reference = el.value)} />
          </Field>
        )}

        {!form.id && replaceable.length > 0 && (
          <Field
            label="Replaces check"
            hint="A bounced check, or a check on hand the client swapped. The old check stops counting."
          >
            <select class={input} onChange$={(_, el) => (form.replaces_id = el.value)}>
              <option value="">Doesn't replace a check</option>
              {replaceable.map((c) => (
                <option key={c.id} value={c.id} selected={c.id === form.replaces_id}>
                  {`${c.no ?? "—"} · ${checkText(c)} · ${formatPeso(c.amount_c)} · ${CHECK_STATUS[c.check!.status].label}`}
                </option>
              ))}
            </select>
          </Field>
        )}

        <div>
          <div class="mb-1 flex flex-wrap items-center justify-between gap-2">
            <span class="text-sm font-medium">Apply to DRs</span>
            <span class="flex gap-1">
              <button
                type="button"
                class={btnGhost}
                disabled={!amount || drs.value.length === 0}
                onClick$={() => (form.allocs = fillOldest(amount, drs.value))}
              >
                Fill oldest first
              </button>
              <button type="button" class={btnGhost} onClick$={() => (form.allocs = {})}>
                Clear
              </button>
            </span>
          </div>
          {!form.client_id ? (
            <p class={hint}>Choose the client to see their unpaid DRs.</p>
          ) : drs.value.length === 0 ? (
            <p class={hint}>This client has no unpaid delivered DRs; the payment stays as credit.</p>
          ) : (
            <div class="overflow-x-auto rounded-md border border-line">
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>DR</th>
                    <th class={th}>Delivered</th>
                    <th class={th}>Due</th>
                    <th class={[th, "text-right"]}>Gross</th>
                    <th class={[th, "text-right"]}>Open</th>
                    <th class={[th, "w-36 text-right"]}>Apply</th>
                  </tr>
                </thead>
                <tbody>
                  {drs.value.map((d) => (
                    <tr key={d.dr_id}>
                      <td class={[td, "whitespace-nowrap font-medium"]}>{drName(d)}</td>
                      <td class={[td, "whitespace-nowrap"]}>{formatDay(d.delivered_date)}</td>
                      <td class={[td, "whitespace-nowrap"]}>
                        {formatDay(d.due_date)}
                        {d.due_date && d.due_date < todayManila() && (
                          <span class="ml-1">
                            <Badge tone="danger">overdue</Badge>
                          </span>
                        )}
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.gross_c)}</td>
                      <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.open_c)}</td>
                      <td class={[td, "py-1"]}>
                        <input
                          class={[inputBase, "w-full text-right"]}
                          inputMode="decimal"
                          aria-label={`Apply to ${drName(d)}`}
                          placeholder="0.00"
                          value={form.allocs[d.dr_id] ?? ""}
                          onInput$={(_, el) => {
                            const next = { ...form.allocs };
                            if (el.value.trim()) next[d.dr_id] = el.value;
                            else delete next[d.dr_id];
                            form.allocs = next;
                          }}
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <dl class="mt-2 ml-auto grid w-72 grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm">
            <dt class="text-ink-muted">Applied</dt>
            <dd class="text-right tabular-nums">{formatPeso(allocated)}</dd>
            <dt class={left < 0 ? "font-semibold text-danger" : "text-ink-muted"}>
              {left < 0 ? "Over the amount" : "Left as credit"}
            </dt>
            <dd class={["text-right tabular-nums", left < 0 && "font-semibold text-danger"]}>
              {formatPeso(Math.abs(left))}
            </dd>
          </dl>
        </div>

        <Field label="Notes">
          <textarea class={input} rows={2} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
        </Field>
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button class={btnPrimary} disabled={busy.value}>
            {form.id ? "Save payment" : "Record payment"}
          </button>
        </div>
      </form>
    </Modal>
  );
});

// Check steps ----------------------------------------------------------------

export const STEP_TITLE: Record<CheckStep, string> = {
  deposit: "Deposit check",
  clear: "Check cleared",
  bounce: "Check bounced",
  undo: "Undo last step",
};

/** The steps a check can take now. */
export function stepsFor(p: PaymentInfo): CheckStep[] {
  if (p.status === "void" || !p.check) return [];
  switch (p.check.status) {
    case "on_hand":
      return ["deposit"];
    case "deposited":
      return ["clear", "bounce", "undo"];
    case "cleared":
      return ["bounce", "undo"];
    case "bounced":
      return ["deposit", "undo"];
    default:
      return [];
  }
}

export const STEP_BUTTON: Record<CheckStep, string> = {
  deposit: "Deposit",
  clear: "Cleared",
  bounce: "Bounced",
  undo: "Undo",
};

/** Deposits, clears or bounces a check, or takes it back one step. */
export const CheckStepModal = component$<{
  open: Signal<boolean>;
  payment: Signal<PaymentInfo | null>;
  step: Signal<CheckStep>;
}>(({ open, payment, step }) => {
  const date = useSignal(todayManila());
  const account = useSignal("");
  const reason = useSignal("");
  const error = useSignal("");
  const busy = useSignal(false);

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    ({ track }) => {
      if (!track(() => open.value)) return;
      const p = payment.value;
      const s = step.value;
      error.value = "";
      reason.value = "";
      account.value = p?.bank_account_id ?? "";
      const checkDate = p?.check?.check_date ?? "";
      const today = todayManila();
      date.value = s === "deposit" && checkDate > today ? checkDate : today;
    },
    { strategy: "document-ready" },
  );

  const submit = $(async () => {
    const p = payment.value;
    if (!p) return;
    busy.value = true;
    error.value = "";
    try {
      unwrap(
        await commands.checkStep(p.id, {
          step: step.value,
          date: date.value,
          bank_account_id: account.value || null,
          reason: reason.value,
        }),
      );
      open.value = false;
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const p = payment.value;
  const s = step.value;
  return (
    <Modal open={open} title={STEP_TITLE[s]}>
      <form class="space-y-3" preventdefault:submit onSubmit$={submit}>
        {error.value && <Alert>{error.value}</Alert>}
        {p && (
          <p class="text-sm">
            {checkText(p)} · <span class="font-medium">{formatPeso(p.amount_c)}</span> from{" "}
            {p.client_name ?? "—"}
          </p>
        )}
        {s === "bounce" && (
          <p class="text-sm">
            The check stops counting as paid: the DRs it paid owe again until the client replaces
            it.
          </p>
        )}
        {s === "undo" && (
          <p class="text-sm">
            {p?.check?.status === "deposited"
              ? "The check goes back to on hand."
              : p?.check?.status === "cleared"
                ? "The check goes back to deposited."
                : "The check goes back to before it bounced, and counts as paid again."}
          </p>
        )}
        {s !== "undo" && (
          <Field label={s === "deposit" ? "Deposited on" : s === "clear" ? "Cleared on" : "Bounced on"}>
            <input
              type="date"
              class={input}
              required
              max={todayManila()}
              value={date.value}
              onChange$={(_, el) => (date.value = el.value)}
            />
          </Field>
        )}
        {s === "deposit" && (
          <Field label="Deposited to">
            <BankAccountSelect
              value={account.value}
              onPick$={(id) => {
                account.value = id;
              }}
            />
          </Field>
        )}
        {s === "bounce" && (
          <Field label="Reason" hint="E.g. DAIF, account closed, signature differs.">
            <input class={input} required value={reason.value} onInput$={(_, el) => (reason.value = el.value)} />
          </Field>
        )}
        <div class="flex justify-end gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Back
          </button>
          <button class={s === "bounce" ? btnDanger : btnPrimary} disabled={busy.value}>
            {STEP_TITLE[s]}
          </button>
        </div>
      </form>
    </Modal>
  );
});

// Credit limit ---------------------------------------------------------------

/**
 * The client's balance against their credit limit, warning when it (plus
 * `extraC`, e.g. a new booking's total) goes over. Nothing without a limit.
 */
export const CreditWarning = component$<{ clientId: string; extraC?: number }>(
  (props) => {
    const bal = useSignal<ClientBalance | null>(null);
    // eslint-disable-next-line qwik/no-use-visible-task
    useVisibleTask$(
      async ({ track }) => {
        const id = track(() => props.clientId);
        bal.value = null;
        if (!id) return;
        const r = await commands.clientBalance(id);
        if (r.status === "ok") bal.value = r.data;
      },
      { strategy: "document-ready" },
    );
    const b = bal.value;
    if (!b) return null;
    const extraC = props.extraC ?? 0;
    const limit = b.credit_limit_c;
    const after = b.balance_c + extraC;
    const over = limit != null && after > limit;
    if (!over && b.overdue_c <= 0) return null;
    return (
      <Alert tone="warning">
        {b.client_name ?? "This client"}{" "}
        {b.balance_c < 0 ? `has ${formatPeso(-b.balance_c)} credit` : `owes ${formatPeso(b.balance_c)}`}
        {b.overdue_c > 0 && <> ({formatPeso(b.overdue_c)} overdue)</>}
        {limit != null && <>; credit limit {formatPeso(limit)}</>}.
        {over &&
          (extraC > 0
            ? ` This booking takes them ${formatPeso(after - limit)} over the limit.`
            : ` That is ${formatPeso(after - limit)} over the limit.`)}
      </Alert>
    );
  },
);
