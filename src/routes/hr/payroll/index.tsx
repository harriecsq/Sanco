import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import { Link, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type DebtKind, type PayrollDebt, type PayrollRow, type PayrollWeek } from "~/lib/bindings";
import { useLiveLoad, useLiveLoadUrl } from "~/lib/events";
import { formatDateTime, formatPeso, moneyText, parseMoney } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { amountCell, DEBT_KINDS, kindLabel, useWeek, WeekNav, weekRange } from "~/components/hr/hr";
import { PayrollSheet, PayslipSheets } from "~/components/hr/print";
import { Alert, Badge, Confirm, Empty, Modal, PageHeader } from "~/components/ui/ui";
import { btnGhost, btnPrimary, btnSecondary, card, hint, input, table, td as td0, th as th0 } from "~/components/ui/styles";

// Eleven money columns: tighter cells than other tables.
const th = `${th0.replace("px-3", "px-2")} whitespace-nowrap`;
const td = td0.replace("px-3", "px-2");

const TABLES = [
  "attendance",
  "employees",
  "employee_debts",
  "debt_overrides",
  "payrolls",
  "payroll_lines",
  "payroll_debt_lines",
];

const STATUS = {
  draft: { label: "Draft", tone: "warning" },
  posted: { label: "Posted", tone: "brand" },
  voided: { label: "Voided, not posted again", tone: "danger" },
} as const;

/** What's printed: nothing, the payroll, every payslip, or one employee's. */
type PrintMode = "" | "payroll" | "slips" | `slip:${string}`;

interface ChargeEdit {
  name: string;
  kind: DebtKind;
  debts: (PayrollDebt & { text: string })[];
}

export default component$(() => {
  const session = useSession();
  const week = useWeek();
  const data = useSignal<PayrollWeek | null>(null);
  const company = useSignal("Sanco Medical Supply");
  const error = useSignal("");
  const postOpen = useSignal(false);
  const voidOpen = useSignal(false);
  const printing = useSignal<PrintMode>("");
  const chargeOpen = useSignal(false);
  const chargeError = useSignal("");
  const charge = useStore<ChargeEdit>({ name: "", kind: "vale", debts: [] });

  useLiveLoadUrl(
    TABLES,
    $(async () => {
      if (!can(session.user, "hr")) return;
      try {
        data.value = unwrap(await commands.payrollWeek(week.value));
        error.value = "";
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );
  useLiveLoad(
    ["settings"],
    $(async () => {
      const res = await commands.companyInfo();
      if (res.status === "ok" && res.data.name.trim()) company.value = res.data.name.trim();
    }),
  );

  const print = $((mode: PrintMode) => {
    printing.value = mode;
    // Let the print layout render first.
    setTimeout(() => window.print(), 100);
  });

  const openCharge = $((r: PayrollRow, kind: DebtKind) => {
    charge.name = r.name;
    charge.kind = kind;
    charge.debts = r.debts
      .filter((d) => d.kind === kind)
      .map((d) => ({ ...d, text: moneyText(d.amount_c) }));
    chargeError.value = "";
    chargeOpen.value = true;
  });

  const saveCharge = $(async (i: number, reset: boolean) => {
    const d = charge.debts[i];
    chargeError.value = "";
    try {
      const amount = reset ? null : (parseMoney(d.text, "deduction") ?? 0);
      unwrap(await commands.setDebtCharge(d.debt_id, week.value, amount));
      if (reset) d.text = moneyText(d.scheduled_c);
      chargeOpen.value = charge.debts.length > 1;
    } catch (e) {
      chargeError.value = errorText(e);
    }
  });

  const p = data.value;
  const draft = !!p && p.status !== "posted";
  const canEdit = can(session.user, "hr", "edit") && draft;
  const canApprove = can(session.user, "hr", "approve");
  const worked = p?.rows.filter((r) => r.worked) ?? [];
  const sum = (f: (r: PayrollRow) => number) => worked.reduce((s, r) => s + f(r), 0);
  const slipRows =
    printing.value.startsWith("slip:")
      ? worked.filter((r) => `slip:${r.employee_id}` === printing.value)
      : worked;

  return (
    <>
      <div class="print:hidden">
        <PageHeader title="Payroll" subtitle="Paid on Saturday for Monday to Saturday.">
          <div q:slot="actions">
            <WeekNav week={week.value} />
          </div>
        </PageHeader>
        {error.value && <Alert>{error.value}</Alert>}

        {p && (
          <>
            <div class="mb-4 flex flex-wrap items-stretch gap-3">
              <div class={[card, "px-4 py-3"]}>
                <div class="text-xs text-ink-muted">Total to pay this week</div>
                <div class="text-2xl font-semibold tabular-nums">{formatPeso(p.total_net_c)}</div>
                <div class={hint}>
                  {worked.length} employee{worked.length === 1 ? "" : "s"} · gross {formatPeso(p.total_gross_c)}
                </div>
              </div>
              <div class={[card, "flex flex-1 flex-col justify-between gap-2 px-4 py-3"]}>
                <div class="flex flex-wrap items-center gap-2">
                  <span class="font-semibold">{weekRange(p.week)}</span>
                  <Badge tone={STATUS[p.status].tone}>{STATUS[p.status].label}</Badge>
                  {p.status === "posted" && (
                    <span class={hint}>
                      {p.posted_by ? `by ${p.posted_by} ` : ""}
                      {formatDateTime(p.posted_at)}
                    </span>
                  )}
                  {p.future && <span class={hint}>This week hasn't come yet.</span>}
                </div>
                <div class="flex flex-wrap gap-2">
                  {p.status === "posted" ? (
                    <>
                      <button type="button" class={btnPrimary} onClick$={() => print("payroll")}>
                        Print payroll
                      </button>
                      <button type="button" class={btnSecondary} onClick$={() => print("slips")}>
                        Print all payslips
                      </button>
                      {canApprove && (
                        <button type="button" class={[btnGhost, "text-danger"]} onClick$={() => (voidOpen.value = true)}>
                          Void
                        </button>
                      )}
                    </>
                  ) : (
                    canApprove && (
                      <button
                        type="button"
                        class={btnPrimary}
                        disabled={!p.can_post}
                        onClick$={() => (postOpen.value = true)}
                      >
                        Post payroll
                      </button>
                    )
                  )}
                  {draft && <span class={[hint, "self-center"]}>Printing is available once the payroll is posted.</span>}
                </div>
              </div>
            </div>

            <div class="mb-3 space-y-2">
              {draft && p.unmarked > 0 && (
                <Alert tone="warning">
                  {p.unmarked} attendance box{p.unmarked === 1 ? " is" : "es are"} not marked this week.{" "}
                  <Link class="font-medium underline" href={`/hr/?week=${p.week}`}>
                    Open attendance
                  </Link>
                </Alert>
              )}
              {draft && p.over.length > 0 && (
                <Alert>
                  Deductions exceed pay for {p.over.join(", ")}. Click a loan amount to reduce it; the rest moves
                  to the following weeks. The payroll can't be posted until then.
                </Alert>
              )}
            </div>
          </>
        )}

        <section class={[card, "overflow-x-auto"]}>
          {!p ? (
            <Empty>Loading…</Empty>
          ) : p.rows.length === 0 ? (
            <Empty>
              {p.status === "posted" ? "Nobody was paid this week." : "No employees yet."}
            </Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>Employee</th>
                  <th class={[th, "text-right"]}>Rate</th>
                  <th class={[th, "text-right"]}># of D</th>
                  <th class={[th, "text-right"]}>Total</th>
                  <th class={[th, "text-right"]}>SSS/PI/PH</th>
                  {DEBT_KINDS.map((k) => (
                    <th key={k.kind} class={[th, "text-right"]}>
                      {k.label}
                    </th>
                  ))}
                  <th class={[th, "text-right"]}>Amount</th>
                  {p.status === "posted" && <th class={th}></th>}
                </tr>
              </thead>
              <tbody>
                {p.rows.map((r) =>
                  !r.worked ? (
                    <tr key={r.employee_id} class="text-ink-muted">
                      <td class={td}>{r.name}</td>
                      <td class={[td, "text-right tabular-nums"]}>{amountCell(r.daily_rate_c)}</td>
                      <td class={td} colSpan={p.status === "posted" ? 10 : 9}>
                        No work this week
                        {r.absent + r.holidays > 0 &&
                          ` (${[r.absent && `${r.absent} absent`, r.holidays && `${r.holidays} unpaid holiday`]
                            .filter(Boolean)
                            .join(", ")})`}
                      </td>
                    </tr>
                  ) : (
                    <tr key={r.employee_id} class={r.net_c < 0 ? "bg-red-50" : ""}>
                      <td class={[td, "whitespace-nowrap"]}>
                        <Link class="hover:text-brand-700" href={`/hr/employees/?id=${r.employee_id}`}>
                          {r.name}
                        </Link>
                        {r.net_c < 0 && (
                          <span class="block text-xs text-danger">
                            Deductions exceed pay by {formatPeso(-r.net_c)}. Reduce a loan.
                          </span>
                        )}
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>{amountCell(r.daily_rate_c)}</td>
                      <td class={[td, "text-right tabular-nums"]}>
                        {r.days}
                        {r.unmarked > 0 && draft && (
                          <span class={[hint, "block whitespace-nowrap"]} title="Boxes not marked">
                            {r.unmarked} blank
                          </span>
                        )}
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>{amountCell(r.gross_c)}</td>
                      <td
                        class={[td, "text-right tabular-nums"]}
                        title={`SSS ${formatPeso(r.sss_c)} · PhilHealth ${formatPeso(r.philhealth_c)} · Pag-IBIG ${formatPeso(r.pagibig_c)}`}
                      >
                        {amountCell(r.contrib_c)}
                      </td>
                      {DEBT_KINDS.map((k) => {
                        const debts = r.debts.filter((d) => d.kind === k.kind);
                        const changed = debts.some((d) => d.changed);
                        return (
                          <td key={k.kind} class={[td, "text-right tabular-nums"]}>
                            {canEdit && debts.length > 0 ? (
                              <button
                                type="button"
                                class={[
                                  "rounded px-1 underline decoration-dotted underline-offset-2 hover:bg-brand-50",
                                  changed && "font-semibold text-brand-700",
                                ]}
                                title={changed ? "Changed by hand. Click to change." : "Click to change this week's deduction"}
                                onClick$={() => openCharge(r, k.kind)}
                              >
                                {amountCell(r[k.field]) || "0.00"}
                              </button>
                            ) : (
                              amountCell(r[k.field])
                            )}
                          </td>
                        );
                      })}
                      <td class={[td, "text-right font-semibold tabular-nums", r.net_c < 0 && "text-danger"]}>
                        {amountCell(r.net_c) || "0.00"}
                      </td>
                      {p.status === "posted" && (
                        <td class={[td, "text-right"]}>
                          <button
                            type="button"
                            class={btnGhost}
                            title={`Print ${r.name}'s payslip`}
                            onClick$={() => print(`slip:${r.employee_id}`)}
                          >
                            Payslip
                          </button>
                        </td>
                      )}
                    </tr>
                  ),
                )}
              </tbody>
              <tfoot>
                <tr class="font-semibold">
                  <td class={td} colSpan={3}>
                    Total
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>{amountCell(sum((r) => r.gross_c))}</td>
                  <td class={[td, "text-right tabular-nums"]}>{amountCell(sum((r) => r.contrib_c))}</td>
                  {DEBT_KINDS.map((k) => (
                    <td key={k.kind} class={[td, "text-right tabular-nums"]}>
                      {amountCell(sum((r) => r[k.field]))}
                    </td>
                  ))}
                  <td class={[td, "text-right tabular-nums whitespace-nowrap"]}>{formatPeso(p.total_net_c)}</td>
                  {p.status === "posted" && <td class={td}></td>}
                </tr>
              </tfoot>
            </table>
          )}
        </section>
      </div>

      {p && p.status === "posted" && printing.value && (
        <div class="hidden print:block">
          {printing.value === "payroll" ? (
            <PayrollSheet p={p} />
          ) : (
            <PayslipSheets company={company.value} week={p.week} rows={slipRows} />
          )}
        </div>
      )}

      <Modal open={chargeOpen} title={`${charge.name}: ${kindLabel(charge.kind)} this week`}>
        <div class="space-y-3">
          {chargeError.value && <Alert>{chargeError.value}</Alert>}
          <p class={hint}>
            Lower this week's deduction (₱0.05 steps). What's left is spread over the following weeks.
          </p>
          {charge.debts.map((d, i) => (
            <form
              key={d.debt_id}
              class="space-y-2 rounded-md border border-line p-3"
              preventdefault:submit
              onSubmit$={() => saveCharge(i, false)}
            >
              <div class="flex flex-wrap justify-between gap-2 text-sm">
                <span>{d.notes || kindLabel(d.kind)}</span>
                <span class={hint}>
                  Balance {formatPeso(d.balance_c)} · schedule {formatPeso(d.scheduled_c)}
                </span>
              </div>
              <div class="flex gap-2">
                <input
                  class={input}
                  inputMode="decimal"
                  aria-label="Deduction this week"
                  value={d.text}
                  onInput$={(_, el) => (charge.debts[i].text = el.value)}
                />
                <button class={btnPrimary}>Save</button>
                {d.changed && (
                  <button type="button" class={btnSecondary} onClick$={() => saveCharge(i, true)}>
                    Use schedule
                  </button>
                )}
              </div>
            </form>
          ))}
          <div class="flex justify-end">
            <button type="button" class={btnSecondary} onClick$={() => (chargeOpen.value = false)}>
              Close
            </button>
          </div>
        </div>
      </Modal>

      <Confirm
        open={postOpen}
        title={`Post the payroll for ${p ? weekRange(p.week) : ""}?`}
        confirmLabel="Post payroll"
        onConfirm$={$(async () => {
          unwrap(await commands.postPayroll(week.value));
        })}
      >
        <p>
          Pays {worked.length} employee{worked.length === 1 ? "" : "s"} a total of{" "}
          <strong>{formatPeso(p?.total_net_c ?? 0)}</strong> and deducts the loans from their balances. The week's
          attendance is then locked; void the payroll to change anything.
        </p>
        {!!p?.unmarked && (
          <p class="mt-2 text-amber-800">
            {p.unmarked} attendance box{p.unmarked === 1 ? " is" : "es are"} still not marked and count as not worked.
          </p>
        )}
      </Confirm>

      <Confirm
        open={voidOpen}
        title={`Void the payroll for ${p ? weekRange(p.week) : ""}?`}
        confirmLabel="Void payroll"
        onConfirm$={$(async () => {
          unwrap(await commands.voidPayroll(week.value));
        })}
      >
        <p>
          The deductions go back on the loans and the week's attendance can be changed again. Post it again once
          it's right.
        </p>
      </Confirm>
    </>
  );
});

export const head: DocumentHead = { title: "Payroll · Sanco" };
