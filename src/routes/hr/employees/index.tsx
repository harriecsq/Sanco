import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type Employee,
  type EmployeeProfile,
  type PayrollRow,
} from "~/lib/bindings";
import { useLiveLoad, useLiveLoadUrl } from "~/lib/events";
import { formatDay, formatPeso, moneyText, parseMoney } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { blankDebt, DebtModal, debtToForm, type DebtForm } from "~/components/hr/debt-form";
import { DEBT_STATUS, kindLabel, weekRange } from "~/components/hr/hr";
import { PayslipSheets } from "~/components/hr/print";
import { Alert, Badge, Confirm, Empty, Field, InfoList, Modal, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { btnGhost, btnPrimary, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

const TABLES = [
  "employees",
  "employee_debts",
  "attendance",
  "payrolls",
  "payroll_lines",
  "payroll_debt_lines",
  "debt_overrides",
];

interface EmployeeForm {
  id: string;
  name: string;
  position: string;
  daily_rate: string;
  sss: string;
  philhealth: string;
  pagibig: string;
  start_date: string;
  active: boolean;
  notes: string;
}

const blankForm = (): EmployeeForm => ({
  id: "",
  name: "",
  position: "",
  daily_rate: "",
  sss: "",
  philhealth: "",
  pagibig: "",
  start_date: "",
  active: true,
  notes: "",
});

const toForm = (e: Employee): EmployeeForm => ({
  id: e.id,
  name: e.name,
  position: e.position ?? "",
  daily_rate: moneyText(e.daily_rate_c),
  sss: moneyText(e.sss_c),
  philhealth: moneyText(e.philhealth_c),
  pagibig: moneyText(e.pagibig_c),
  start_date: e.start_date ?? "",
  active: e.active,
  notes: e.notes ?? "",
});

const contrib = (e: Employee) => e.sss_c + e.philhealth_c + e.pagibig_c;

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const employees = useSignal<Employee[]>([]);
  const profile = useSignal<EmployeeProfile | null>(null);
  const error = useSignal("");
  const formOpen = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);
  const form = useStore<EmployeeForm>(blankForm());
  const deleteOpen = useSignal(false);
  const debtOpen = useSignal(false);
  const debtForm = useStore<DebtForm>(blankDebt());
  const showPaid = useSignal(false);
  const company = useSignal("Sanco Medical Supply");
  const slip = useSignal<{ week: string; row: PayrollRow } | null>(null);
  const selected = useComputed$(() => loc.url.searchParams.get("id") ?? "");

  useLiveLoadUrl(
    TABLES,
    $(async () => {
      if (!can(session.user, "hr")) return;
      try {
        employees.value = unwrap(await commands.listEmployees());
        const id = loc.url.searchParams.get("id");
        profile.value = id ? unwrap(await commands.employeeProfile(id)) : null;
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

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "status"]);
    const q = f.q.toLowerCase();
    return employees.value.filter(
      (e) =>
        (!q || [e.name, e.position, e.notes].some((v) => v?.toLowerCase().includes(q))) &&
        (f.status === "all" || (f.status === "inactive" ? !e.active : e.active)),
    );
  });
  const pg = usePage(shown);

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      const num = (t: string, what: string) => parseMoney(t, what) ?? 0;
      unwrap(
        await commands.saveEmployee({
          id: form.id || null,
          name: form.name,
          position: form.position,
          daily_rate_c: num(form.daily_rate, "daily rate"),
          sss_c: num(form.sss, "SSS"),
          philhealth_c: num(form.philhealth, "PhilHealth"),
          pagibig_c: num(form.pagibig, "Pag-IBIG"),
          start_date: form.start_date || null,
          active: form.active,
          notes: form.notes,
        }),
      );
      formOpen.value = false;
    } catch (e) {
      formError.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const printSlip = $(async (week: string, employeeId: string) => {
    try {
      const p = unwrap(await commands.payrollWeek(week));
      const row = p.rows.find((r) => r.employee_id === employeeId && r.worked);
      if (!row) return;
      slip.value = { week, row };
      setTimeout(() => window.print(), 100);
    } catch (e) {
      error.value = errorText(e);
    }
  });

  const canCreate = can(session.user, "hr", "create");
  const canEdit = can(session.user, "hr", "edit");
  const canDelete = can(session.user, "hr", "delete");
  const pr = profile.value;

  const formModal = (
    <Modal open={formOpen} title={form.id ? "Edit employee" : "Add employee"}>
      <form class="space-y-3" preventdefault:submit onSubmit$={save}>
        {formError.value && <Alert>{formError.value}</Alert>}
        <Field label="Name">
          <input class={input} required value={form.name} onInput$={(_, el) => (form.name = el.value)} />
        </Field>
        <div class="grid grid-cols-2 gap-3">
          <Field label="Position">
            <input class={input} value={form.position} onInput$={(_, el) => (form.position = el.value)} />
          </Field>
          <Field label="Daily rate">
            <input
              class={input}
              inputMode="decimal"
              required
              placeholder="0.00"
              value={form.daily_rate}
              onInput$={(_, el) => (form.daily_rate = el.value)}
            />
          </Field>
        </div>
        <fieldset>
          <legend class="mb-1 text-sm font-medium">Contributions per week</legend>
          <div class="grid grid-cols-3 gap-3">
            <Field label="SSS">
              <input class={input} inputMode="decimal" placeholder="0.00" value={form.sss} onInput$={(_, el) => (form.sss = el.value)} />
            </Field>
            <Field label="PhilHealth">
              <input
                class={input}
                inputMode="decimal"
                placeholder="0.00"
                value={form.philhealth}
                onInput$={(_, el) => (form.philhealth = el.value)}
              />
            </Field>
            <Field label="Pag-IBIG">
              <input
                class={input}
                inputMode="decimal"
                placeholder="0.00"
                value={form.pagibig}
                onInput$={(_, el) => (form.pagibig = el.value)}
              />
            </Field>
          </div>
          <p class={[hint, "mt-1"]}>Charged only in weeks the employee works at least one day.</p>
        </fieldset>
        <div class="grid grid-cols-2 gap-3">
          <Field label="Start date">
            <input type="date" class={input} value={form.start_date} onChange$={(_, el) => (form.start_date = el.value)} />
          </Field>
          <label class="flex items-center gap-2 self-end pb-2 text-sm">
            <input type="checkbox" checked={form.active} onChange$={(_, el) => (form.active = el.checked)} />
            Active
          </label>
        </div>
        <Field label="Notes">
          <textarea class={input} rows={2} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
        </Field>
        <p class={hint}>Inactive employees keep their history but leave the attendance sheet.</p>
        <div class="flex items-center justify-between gap-2">
          <span>
            {form.id && canDelete && (
              <button
                type="button"
                class={[btnGhost, "text-danger"]}
                onClick$={() => {
                  formOpen.value = false;
                  deleteOpen.value = true;
                }}
              >
                Delete
              </button>
            )}
          </span>
          <span class="flex gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (formOpen.value = false)}>
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Save
            </button>
          </span>
        </div>
      </form>
    </Modal>
  );

  const shared = (
    <>
      {formModal}
      <DebtModal open={debtOpen} form={debtForm} employees={employees.value} canDelete={canDelete} />
      <Confirm
        open={deleteOpen}
        title={`Delete ${form.name || "employee"}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteEmployee(form.id));
        })}
      >
        <p>
          Only an employee with no attendance, debts or payroll can be deleted. Otherwise, edit them and untick Active.
        </p>
      </Confirm>
    </>
  );

  // Profile -----------------------------------------------------------------
  if (selected.value) {
    if (!pr) {
      return (
        <>
          {error.value && <Alert>{error.value}</Alert>}
          <section class={card}>
            <Empty>{employees.value.length ? "That employee no longer exists." : "Loading…"}</Empty>
          </section>
        </>
      );
    }
    const e = pr.employee;
    const debts = pr.debts.filter((d) => showPaid.value || d.status !== "paid");
    const paidCount = pr.debts.filter((d) => d.status === "paid").length;
    return (
      <>
        <div class="print:hidden">
          <div class="mb-2">
            <Link href="/hr/employees/" class="text-sm text-brand-700 hover:underline">
              ‹ All employees
            </Link>
          </div>
          <PageHeader title={e.name} subtitle={[e.position, e.active ? "" : "Inactive"].filter(Boolean).join(" · ")}>
            {canEdit && (
              <button
                q:slot="actions"
                class={btnSecondary}
                onClick$={() => {
                  Object.assign(form, toForm(e));
                  formError.value = "";
                  formOpen.value = true;
                }}
              >
                Edit
              </button>
            )}
            {canCreate && (
              <button
                q:slot="actions"
                class={btnPrimary}
                onClick$={() => {
                  Object.assign(debtForm, blankDebt(e.id));
                  debtOpen.value = true;
                }}
              >
                Add debt
              </button>
            )}
          </PageHeader>
          {error.value && <Alert>{error.value}</Alert>}

          <div class="mb-4 grid gap-4 md:grid-cols-3">
            <section class={[card, "p-4 md:col-span-2"]}>
              <InfoList
                rows={[
                  ["Daily rate", formatPeso(e.daily_rate_c)],
                  ["SSS / week", formatPeso(e.sss_c)],
                  ["PhilHealth / week", formatPeso(e.philhealth_c)],
                  ["Pag-IBIG / week", formatPeso(e.pagibig_c)],
                  ["Start date", formatDay(e.start_date)],
                  ["Notes", e.notes ?? ""],
                ]}
              />
            </section>
            <section class={[card, "p-4"]}>
              <h2 class="mb-2 text-sm font-semibold">{pr.year} so far</h2>
              <div class="grid grid-cols-3 gap-2 text-center">
                <div>
                  <div class={["text-2xl font-semibold tabular-nums", pr.absent > 0 && "text-danger"]}>{pr.absent}</div>
                  <div class={hint}>Absences</div>
                </div>
                <div>
                  <div class="text-2xl font-semibold tabular-nums">{pr.present}</div>
                  <div class={hint}>Days present</div>
                </div>
                <div>
                  <div class="text-2xl font-semibold tabular-nums">{pr.holidays}</div>
                  <div class={hint}>Holidays</div>
                </div>
              </div>
              <div class="mt-3 border-t border-line pt-3 text-sm">
                Debt balance <span class="float-right font-semibold tabular-nums">{formatPeso(e.debt_balance_c)}</span>
              </div>
            </section>
          </div>

          <section class={[card, "mb-4 overflow-x-auto"]}>
            <div class="flex items-center justify-between border-b border-line px-4 py-2">
              <h2 class="text-sm font-semibold">Debts</h2>
              {paidCount > 0 && (
                <label class="flex items-center gap-1.5 text-sm text-ink-muted">
                  <input type="checkbox" checked={showPaid.value} onChange$={(_, el) => (showPaid.value = el.checked)} />
                  Show paid ({paidCount})
                </label>
              )}
            </div>
            {debts.length === 0 ? (
              <Empty>No current debts.</Empty>
            ) : (
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>Type</th>
                    <th class={[th, "text-right"]}>Amount</th>
                    <th class={[th, "text-right"]}>Balance</th>
                    <th class={[th, "text-right"]}>This week</th>
                    <th class={th}>Last week</th>
                    <th class={th}>Remit date</th>
                    <th class={th}>Status</th>
                    <th class={th}></th>
                  </tr>
                </thead>
                <tbody>
                  {debts.map((d) => (
                    <tr key={d.id} class={d.status === "paid" ? "text-ink-muted" : ""}>
                      <td class={td}>
                        {kindLabel(d.kind)}
                        {d.notes && <span class={[hint, "block"]}>{d.notes}</span>}
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>
                        {formatPeso(d.amount_c)}
                        <span class={[hint, "block"]}>
                          {formatPeso(d.weekly_c)} × {d.weeks} wk{d.weeks === 1 ? "" : "s"}
                        </span>
                      </td>
                      <td class={[td, "text-right font-semibold tabular-nums"]}>{formatPeso(d.balance_c)}</td>
                      <td class={[td, "text-right tabular-nums"]}>{d.this_week_c ? formatPeso(d.this_week_c) : "—"}</td>
                      <td class={[td, "whitespace-nowrap"]}>{formatDay(d.last_week)}</td>
                      <td class={[td, "whitespace-nowrap"]}>
                        {formatDay(d.remit_date)}
                        {d.remit_warning && d.status !== "paid" && (
                          <span class="block text-xs text-amber-800">Ends after this date</span>
                        )}
                      </td>
                      <td class={td}>
                        <Badge tone={DEBT_STATUS[d.status].tone}>{DEBT_STATUS[d.status].label}</Badge>
                      </td>
                      <td class={[td, "text-right"]}>
                        {canEdit && (
                          <button
                            type="button"
                            class={btnGhost}
                            onClick$={() => {
                              Object.assign(debtForm, debtToForm(d));
                              debtOpen.value = true;
                            }}
                          >
                            Edit
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          <section class={[card, "overflow-x-auto"]}>
            <h2 class="border-b border-line px-4 py-2 text-sm font-semibold">Payslips</h2>
            {pr.payslips.length === 0 ? (
              <Empty>No posted payrolls yet.</Empty>
            ) : (
              <table class={table}>
                <thead>
                  <tr>
                    <th class={th}>Week</th>
                    <th class={[th, "text-right"]}>Days</th>
                    <th class={[th, "text-right"]}>Total</th>
                    <th class={[th, "text-right"]}>Deductions</th>
                    <th class={[th, "text-right"]}>Net pay</th>
                    <th class={th}></th>
                  </tr>
                </thead>
                <tbody>
                  {pr.payslips.map((s) => (
                    <tr key={s.week}>
                      <td class={td}>
                        <Link class="hover:text-brand-700" href={`/hr/payroll/?week=${s.week}`}>
                          {weekRange(s.week)}
                        </Link>
                      </td>
                      <td class={[td, "text-right tabular-nums"]}>{s.days}</td>
                      <td class={[td, "text-right tabular-nums"]}>{formatPeso(s.gross_c)}</td>
                      <td class={[td, "text-right tabular-nums"]}>{formatPeso(s.deductions_c)}</td>
                      <td class={[td, "text-right font-semibold tabular-nums"]}>{formatPeso(s.net_c)}</td>
                      <td class={[td, "text-right"]}>
                        <button type="button" class={btnGhost} onClick$={() => printSlip(s.week, e.id)}>
                          Print payslip
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>
        </div>
        {slip.value && (
          <div class="hidden print:block">
            <PayslipSheets company={company.value} week={slip.value.week} rows={[slip.value.row]} />
          </div>
        )}
        {shared}
      </>
    );
  }

  // List --------------------------------------------------------------------
  const list = shown.value;
  return (
    <>
      <PageHeader title="Employees" subtitle="Daily rates and weekly SSS, PhilHealth and Pag-IBIG.">
        {canCreate && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blankForm());
              formError.value = "";
              formOpen.value = true;
            }}
          >
            Add employee
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search name, position"
        selects={[
          {
            key: "status",
            label: "Status",
            allLabel: "Active",
            options: [
              { value: "inactive", label: "Inactive" },
              { value: "all", label: "All" },
            ],
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {list.length === 0 ? (
          <Empty>{employees.value.length ? "Nothing matches." : "No employees yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Name</th>
                <th class={th}>Position</th>
                <th class={[th, "text-right"]}>Daily rate</th>
                <th class={[th, "text-right"]}>SSS/PI/PH per week</th>
                <th class={[th, "text-right"]}>Debt balance</th>
                <th class={th}></th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((e) => (
                <tr key={e.id} class={e.active ? "" : "text-ink-muted"}>
                  <td class={td}>
                    <Link class="font-medium hover:text-brand-700" href={`/hr/employees/?id=${e.id}`}>
                      {e.name}
                    </Link>
                    {!e.active && (
                      <span class="ml-2">
                        <Badge>Inactive</Badge>
                      </span>
                    )}
                  </td>
                  <td class={td}>{e.position ?? "—"}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(e.daily_rate_c)}</td>
                  <td
                    class={[td, "text-right tabular-nums"]}
                    title={`SSS ${formatPeso(e.sss_c)} · PhilHealth ${formatPeso(e.philhealth_c)} · Pag-IBIG ${formatPeso(e.pagibig_c)}`}
                  >
                    {formatPeso(contrib(e))}
                  </td>
                  <td class={[td, "text-right tabular-nums"]}>{e.debt_balance_c ? formatPeso(e.debt_balance_c) : "—"}</td>
                  <td class={[td, "text-right"]}>
                    {canEdit && (
                      <button
                        type="button"
                        class={btnGhost}
                        onClick$={() => {
                          Object.assign(form, toForm(e));
                          formError.value = "";
                          formOpen.value = true;
                        }}
                      >
                        Edit
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
      {shared}
    </>
  );
});

export const head: DocumentHead = { title: "Employees · Sanco" };
