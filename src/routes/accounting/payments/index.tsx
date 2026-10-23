import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type CheckStep,
  type Choice,
  type PaymentDetail,
  type PaymentInfo,
  type PayMethod,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDateTime, formatDay, formatPeso } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  CheckStepModal,
  METHOD_LABEL,
  MONEY_TABLES,
  PaymentBadge,
  PaymentFormModal,
  STEP_BUTTON,
  blankPayment,
  checkText,
  paymentForm,
  stepsFor,
} from "~/components/accounting/accounting";
import { Alert, Badge, Empty, Field, InfoList, Modal, NoAccess, PageHeader } from "~/components/ui/ui";
import { btnDanger, btnPrimary, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

/** Payment no. with its "given offline" hint. */
const PayNo = component$<{ p: PaymentInfo }>(({ p }) => (
  <>
    {p.no ?? "—"}
    {p.no && !p.no_confirmed && (
      <span class="ml-2" title="Given while offline from other PCs; it may still change.">
        <Badge tone="warning">offline no.</Badge>
      </span>
    )}
  </>
));

const PaymentList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const payments = useSignal<PaymentInfo[]>([]);
  const clients = useSignal<Choice[]>([]);
  const error = useSignal("");
  const formOpen = useSignal(false);
  const form = useStore(blankPayment());

  useLiveLoad(
    MONEY_TABLES,
    $(async () => {
      try {
        payments.value = unwrap(await commands.listPayments());
        clients.value = unwrap(await commands.clientOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "method", "status", "client"]);
    const q = f.q.toLowerCase();
    return payments.value.filter(
      (p) =>
        (!q ||
          [p.no, p.client_name, p.reference, p.or_no, p.notes, p.check?.check_no, p.check?.bank, p.bank_account_name, ...p.dr_nos].some(
            (v) => v?.toLowerCase().includes(q),
          )) &&
        (!f.from || p.date >= f.from) &&
        (!f.to || p.date <= f.to) &&
        (!f.client || p.client_id === f.client) &&
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

  const total = shown.value.filter((p) => p.counts).reduce((s, p) => s + p.amount_c, 0);

  return (
    <>
      <PageHeader title="Payments" subtitle="Cash, checks and bank transfers received from clients, applied to their DRs.">
        {can(session.user, "accounting", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blankPayment(filtersOf(loc.url, ["client"]).client));
              formOpen.value = true;
            }}
          >
            Record payment
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search no., client, OR, check, DR"
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
          {
            key: "client",
            label: "Clients",
            options: clients.value.map((c) => ({ value: c.id, label: c.label })),
          },
        ]}
      >
        <span class={hint}>
          {shown.value.length} payment(s) · {formatPeso(total)} counting
        </span>
      </FilterBar>
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{payments.value.length ? "No payments match." : "No payments yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>No.</th>
                <th class={th}>Received</th>
                <th class={th}>Client</th>
                <th class={th}>Method</th>
                <th class={[th, "text-right"]}>Amount</th>
                <th class={th}>Applied to</th>
                <th class={[th, "text-right"]}>Credit</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((p) => (
                <tr key={p.id} class={p.counts ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/accounting/payments/?id=${p.id}`}>
                      <PayNo p={p} />
                    </Link>
                    {p.or_no && <span class={[hint, "block"]}>OR {p.or_no}</span>}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(p.date)}</td>
                  <td class={td}>{p.client_name ?? "—"}</td>
                  <td class={td}>
                    {METHOD_LABEL[p.method]}
                    {p.check && <span class={[hint, "block"]}>{checkText(p)}</span>}
                    {!p.check && p.reference && <span class={[hint, "block"]}>{p.reference}</span>}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(p.amount_c)}</td>
                  <td class={td}>{p.dr_nos.length ? p.dr_nos.map((n) => `DR ${n}`).join(", ") : "—"}</td>
                  <td class={[td, "text-right tabular-nums", p.unapplied_c < 0 && "text-danger"]}>
                    {p.counts && p.unapplied_c !== 0 ? formatPeso(p.unapplied_c) : "—"}
                  </td>
                  <td class={td}>
                    <PaymentBadge p={p} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <PaymentFormModal
        open={formOpen}
        form={form}
        saved$={$(async (id: string) => {
          await nav(`/accounting/payments/?id=${id}`);
        })}
      />
    </>
  );
});

const PaymentView = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const nav = useNavigate();
  const detail = useSignal<PaymentDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const formOpen = useSignal(false);
  const form = useStore(blankPayment());
  const stepOpen = useSignal(false);
  const step = useSignal<CheckStep>("deposit");
  const stepPayment = useSignal<PaymentInfo | null>(null);
  const voidOpen = useSignal(false);
  const reason = useSignal("");
  const voidError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    MONEY_TABLES,
    $(async () => {
      try {
        const d = unwrap(await commands.getPayment(id));
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
        <PageHeader title="Payment not found" />
        <Link class="text-sm text-brand-700 hover:underline" href="/accounting/payments/">
          Back to payments
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const p = det.payment;
  const user = session.user;
  const c = p.check;

  const rows: [string, string][] = [
    ["Client", p.client_name ? `${p.client_name}${p.client_code ? ` (${p.client_code})` : ""}` : "—"],
    ["Received on", formatDay(p.date)],
    ["Method", METHOD_LABEL[p.method]],
    ["Amount", formatPeso(p.amount_c)],
    ["OR / CR no.", p.or_no ?? ""],
  ];
  if (c) {
    rows.push(["Bank", c.bank ?? ""], ["Check no.", c.check_no ?? ""], ["Check date", formatDay(c.check_date)]);
    if (c.deposited_date) rows.push(["Deposited", `${formatDay(c.deposited_date)}${c.deposit_account ? ` to ${c.deposit_account}` : ""}`]);
    if (c.cleared_date) rows.push(["Cleared", formatDay(c.cleared_date)]);
    if (c.bounced_date) rows.push(["Bounced", `${formatDay(c.bounced_date)}: ${c.bounce_reason ?? ""}`]);
  } else {
    if (p.method === "bank_transfer") rows.push(["Received into", p.bank_account_name ?? ""]);
    rows.push(["Reference", p.reference ?? ""]);
  }
  rows.push(["Notes", p.notes ?? ""], ["Recorded", `${formatDateTime(p.created_at)} by ${p.created_by_name ?? "—"}`]);
  if (p.status === "void") {
    rows.push(["Voided", `${formatDateTime(p.voided_at)} by ${p.voided_by_name ?? "—"}`], ["Reason", p.void_reason ?? ""]);
  }

  const canEdit = can(user, "accounting", "edit");
  const steps = canEdit ? stepsFor(p) : [];

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/accounting/payments/">
          ← Payments
        </Link>
      </p>
      <PageHeader title={`Payment ${p.no ?? ""}`} subtitle={p.client_name ?? undefined}>
        {p.status === "posted" && can(user, "accounting", "delete") && (
          <button
            q:slot="actions"
            class={btnSecondary}
            onClick$={() => {
              reason.value = "";
              voidError.value = "";
              voidOpen.value = true;
            }}
          >
            Void
          </button>
        )}
        {steps.map((s) => (
          <button
            q:slot="actions"
            key={s}
            class={btnSecondary}
            onClick$={() => {
              stepPayment.value = detail.value?.payment ?? null;
              step.value = s;
              stepOpen.value = true;
            }}
          >
            {STEP_BUTTON[s]}
          </button>
        ))}
        {p.check && (p.check.status === "bounced" || p.check.status === "on_hand") && p.status === "posted" &&
          can(user, "accounting", "create") && (
            <button
              q:slot="actions"
              class={btnSecondary}
              onClick$={() => {
                Object.assign(form, blankPayment(p.client_id ?? "", p.id));
                formOpen.value = true;
              }}
            >
              Record replacement
            </button>
          )}
        {p.status === "posted" && canEdit && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              if (detail.value) Object.assign(form, paymentForm(detail.value));
              formOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 space-y-2">
        <div class="flex flex-wrap items-center gap-3 text-sm">
          <PaymentBadge p={p} />
          {p.replaces_id && (
            <span>
              Replaces check{" "}
              <Link class="text-brand-700 hover:underline" href={`/accounting/payments/?id=${p.replaces_id}`}>
                {p.replaces_no ?? "—"}
              </Link>
            </span>
          )}
        </div>
        {c?.status === "replaced" && (
          <Alert tone="warning">
            This check was replaced by payment{" "}
            <Link class="underline" href={`/accounting/payments/?id=${c.replaced_by_id}`}>
              {c.replaced_by_no ?? "—"}
            </Link>
            ; it no longer counts as paid.
          </Alert>
        )}
        {c?.status === "bounced" && (
          <Alert>
            This check bounced: the DRs below owe again until the client replaces it (Record replacement).
          </Alert>
        )}
        {p.counts && p.unapplied_c < 0 && (
          <Alert>
            The allocations are {formatPeso(-p.unapplied_c)} more than the amount (edits made on PCs that were
            offline). Edit the payment to fix them.
          </Alert>
        )}
      </div>
      <div class="grid gap-4 xl:grid-cols-[24rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <InfoList rows={rows} />
        </section>
        <section class={[card, "overflow-x-auto"]}>
          <h2 class="border-b border-line px-4 py-3 font-semibold">Applied to</h2>
          {det.allocations.length === 0 ? (
            <Empty>Not applied to any DR: the whole amount is the client's credit.</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>DR</th>
                  <th class={th}>Delivered</th>
                  <th class={[th, "text-right"]}>DR gross</th>
                  <th class={[th, "text-right"]}>Applied</th>
                  <th class={[th, "text-right"]}>DR still owes</th>
                </tr>
              </thead>
              <tbody>
                {det.allocations.map((a) => (
                  <tr key={a.dr_id} class={a.delivered ? "" : "text-ink-muted"}>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link class="font-medium text-brand-700 hover:underline" href={`/accounting/?id=${a.dr_id}`}>
                        {a.dr_no ? `DR ${a.dr_no}` : "DR (no number)"}
                      </Link>
                      {!a.delivered && <span class={[hint, "block"]}>voided: back as credit</span>}
                    </td>
                    <td class={[td, "whitespace-nowrap"]}>{formatDay(a.delivered_date)}</td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPeso(a.gross_c)}</td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPeso(a.amount_c)}</td>
                    <td class={[td, "text-right tabular-nums", a.dr_balance_c < 0 && "text-danger"]}>
                      {a.delivered ? formatPeso(a.dr_balance_c) : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <dl class="ml-auto grid w-72 grid-cols-[1fr_auto] gap-x-4 gap-y-1 border-t border-line px-3 py-3 text-sm">
            <dt class="text-ink-muted">Applied</dt>
            <dd class="text-right tabular-nums">{formatPeso(p.applied_c)}</dd>
            <dt class="text-ink-muted">Credit (not applied)</dt>
            <dd class="text-right tabular-nums">{formatPeso(p.unapplied_c)}</dd>
          </dl>
        </section>
      </div>

      <PaymentFormModal
        open={formOpen}
        form={form}
        saved$={$(async (newId: string) => {
          if (newId !== id) await nav(`/accounting/payments/?id=${newId}`);
        })}
      />
      <CheckStepModal open={stepOpen} payment={stepPayment} step={step} />
      <Modal open={voidOpen} title="Void payment">
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            voidError.value = "";
            try {
              unwrap(await commands.voidPayment(id, reason.value));
              voidOpen.value = false;
            } catch (e) {
              voidError.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          {voidError.value && <Alert>{voidError.value}</Alert>}
          <p class="text-sm">
            The payment stops counting and the DRs it paid owe again, on every PC. It stays in the list, marked
            void. This can't be undone.
          </p>
          <Field label="Reason">
            <input class={input} required value={reason.value} onInput$={(_, el) => (reason.value = el.value)} />
          </Field>
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (voidOpen.value = false)}>
              Back
            </button>
            <button class={btnDanger} disabled={busy.value}>
              Void payment
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
  if (!can(session.user, "accounting")) return <NoAccess />;
  const id = loc.url.searchParams.get("id");
  return id ? <PaymentView key={id} id={id} /> : <PaymentList />;
});

export const head: DocumentHead = { title: "Payments · Sanco" };
