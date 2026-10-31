import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type DebtInfo, type DebtStatus, type Employee } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatPeso } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { blankDebt, DebtModal, debtToForm, type DebtForm } from "~/components/hr/debt-form";
import { DEBT_KINDS, DEBT_STATUS, kindLabel } from "~/components/hr/hr";
import { Alert, Badge, Empty, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { btnGhost, btnPrimary, card, hint, table, td, th } from "~/components/ui/styles";

const TABLES = ["employee_debts", "employees", "payrolls", "payroll_lines", "payroll_debt_lines", "debt_overrides"];

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const debts = useSignal<DebtInfo[]>([]);
  const employees = useSignal<Employee[]>([]);
  const error = useSignal("");
  const formOpen = useSignal(false);
  const form = useStore<DebtForm>(blankDebt());

  useLiveLoad(
    TABLES,
    $(async () => {
      if (!can(session.user, "hr")) return;
      try {
        const [d, e] = await Promise.all([commands.listDebts(null), commands.listEmployees()]);
        debts.value = unwrap(d);
        employees.value = unwrap(e);
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "employee", "kind", "status"]);
    const q = f.q.toLowerCase();
    return debts.value.filter(
      (d) =>
        (!q || [d.employee_name, kindLabel(d.kind), d.notes].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.employee || d.employee_id === f.employee) &&
        (!f.kind || d.kind === f.kind) &&
        // Paid debts only when asked for.
        (f.status === "all" || (f.status ? d.status === f.status : d.status !== "paid")),
    );
  });
  const pg = usePage(shown);

  const canCreate = can(session.user, "hr", "create");
  const canEdit = can(session.user, "hr", "edit");
  const list = shown.value;
  const totalBalance = list.reduce((s, d) => s + d.balance_c, 0);
  const totalWeek = list.reduce((s, d) => s + d.this_week_c, 0);

  return (
    <>
      <PageHeader title="Debts" subtitle="Loans and vale, charged on the Saturday payroll.">
        {canCreate && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blankDebt());
              formOpen.value = true;
            }}
          >
            Add debt
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}

      <FilterBar
        search="Search employee, type, notes"
        selects={[
          {
            key: "employee",
            label: "Employees",
            options: employees.value.map((e) => ({ value: e.id, label: e.name })),
          },
          { key: "kind", label: "Types", options: DEBT_KINDS.map((k) => ({ value: k.kind, label: k.label })) },
          {
            key: "status",
            label: "Status",
            allLabel: "Not paid yet",
            options: [
              ...(Object.keys(DEBT_STATUS) as DebtStatus[]).map((s) => ({ value: s, label: DEBT_STATUS[s].label })),
              { value: "all", label: "All, paid included" },
            ],
          },
        ]}
      />

      <section class={[card, "overflow-x-auto"]}>
        {list.length === 0 ? (
          <Empty>{debts.value.length ? "Nothing matches." : "No debts yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Employee</th>
                <th class={th}>Type</th>
                <th class={[th, "text-right"]}>Amount</th>
                <th class={[th, "text-right"]}>Paid</th>
                <th class={[th, "text-right"]}>Balance</th>
                <th class={[th, "text-right"]}>Weekly</th>
                <th class={[th, "text-right"]}>This week</th>
                <th class={th}>Last week</th>
                <th class={th}>Remit date</th>
                <th class={th}>Status</th>
                <th class={th}></th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((d) => (
                <tr key={d.id} class={d.status === "paid" ? "text-ink-muted" : ""}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="hover:text-brand-700" href={`/hr/employees/?id=${d.employee_id}`}>
                      {d.employee_name}
                    </Link>
                    {d.notes && <span class={[hint, "block"]}>{d.notes}</span>}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{kindLabel(d.kind)}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.amount_c)}</td>
                  <td class={[td, "text-right tabular-nums"]}>{formatPeso(d.paid_c)}</td>
                  <td class={[td, "text-right font-semibold tabular-nums"]}>{formatPeso(d.balance_c)}</td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {formatPeso(d.weekly_c)}
                    <span class={[hint, "block"]}>
                      {d.weeks} wk{d.weeks === 1 ? "" : "s"}
                      {d.last_c !== d.weekly_c && d.weeks > 1 && `, last ${formatPeso(d.last_c)}`}
                    </span>
                  </td>
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
                          Object.assign(form, debtToForm(d));
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
            <tfoot>
              <tr class="font-semibold">
                <td class={td} colSpan={4}>
                  Total shown
                </td>
                <td class={[td, "text-right tabular-nums"]}>{formatPeso(totalBalance)}</td>
                <td class={td}></td>
                <td class={[td, "text-right tabular-nums"]}>{formatPeso(totalWeek)}</td>
                <td class={td} colSpan={4}></td>
              </tr>
            </tfoot>
          </table>
        )}
        <Pager of={pg} />
      </section>
      <p class={[hint, "mt-2"]}>
        This week = what this Saturday's payroll takes if the employee works. A week they don't work is skipped and
        spread over the weeks left.
      </p>

      <DebtModal open={formOpen} form={form} employees={employees.value} canDelete={can(session.user, "hr", "delete")} />
    </>
  );
});

export const head: DocumentHead = { title: "Debts · Sanco" };
