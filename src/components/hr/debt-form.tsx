import { $, component$, useSignal, useVisibleTask$, type Signal } from "@builder.io/qwik";
import { commands, type DebtInfo, type DebtKind, type DebtPlan, type Employee } from "~/lib/bindings";
import { formatDay, formatPeso, InputError, moneyText, parseCount, parseMoney, todayManila } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Confirm, Field, Modal } from "~/components/ui/ui";
import { btnGhost, btnPrimary, btnSecondary, hint, input } from "~/components/ui/styles";
import { DEBT_KINDS } from "./hr";

export interface DebtForm {
  id: string;
  employee_id: string;
  kind: DebtKind;
  amount: string;
  start_date: string;
  /** Enter the number of weeks, or type the weekly amount. */
  mode: "weeks" | "weekly";
  weeks: string;
  weekly: string;
  remit_date: string;
  notes: string;
  /** Already deducted (editing). */
  paid_c: number;
}

export const blankDebt = (employeeId = ""): DebtForm => ({
  id: "",
  employee_id: employeeId,
  kind: "vale",
  amount: "",
  start_date: todayManila(),
  mode: "weeks",
  weeks: "",
  weekly: "",
  remit_date: "",
  notes: "",
  paid_c: 0,
});

export const debtToForm = (d: DebtInfo): DebtForm => ({
  id: d.id,
  employee_id: d.employee_id,
  kind: d.kind,
  amount: moneyText(d.amount_c),
  start_date: d.start_date,
  mode: d.weekly_typed ? "weekly" : "weeks",
  weeks: String(d.weeks),
  weekly: d.weekly_typed ? moneyText(d.weekly_c) : "",
  remit_date: d.remit_date ?? "",
  notes: d.notes ?? "",
  paid_c: d.paid_c,
});

/** The form's schedule inputs, or null while incomplete. */
function planInput(f: DebtForm) {
  const amount = parseMoney(f.amount, "amount");
  if (!amount || !f.start_date) return null;
  const weeks = f.mode === "weeks" ? parseCount(f.weeks, "number of weeks") : null;
  const weekly = f.mode === "weekly" ? parseMoney(f.weekly, "weekly amount") : null;
  if (!weeks && !weekly) return null;
  return {
    amount_c: amount,
    start_date: f.start_date,
    weeks,
    weekly_c: weekly,
    remit_date: f.remit_date || null,
  };
}

/** Add or edit a debt, with its schedule shown as it's typed. */
export const DebtModal = component$<{
  open: Signal<boolean>;
  form: DebtForm;
  employees: Employee[];
  canDelete: boolean;
}>(({ open, form, employees, canDelete }) => {
  const plan = useSignal<DebtPlan | null>(null);
  const planError = useSignal("");
  const error = useSignal("");
  const busy = useSignal(false);
  const deleteOpen = useSignal(false);

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(({ track, cleanup }) => {
    track(() => [form.amount, form.start_date, form.mode, form.weeks, form.weekly, form.remit_date, open.value]);
    const t = setTimeout(async () => {
      try {
        const p = planInput(form);
        if (!p) {
          plan.value = null;
          planError.value = "";
          return;
        }
        plan.value = unwrap(await commands.debtPlan(p));
        planError.value = "";
      } catch (e) {
        plan.value = null;
        planError.value = errorText(e);
      }
    }, 200);
    cleanup(() => clearTimeout(t));
  });

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const p = planInput(form);
      if (!p) throw new InputError("Enter the amount, start date and number of weeks or weekly amount.");
      unwrap(
        await commands.saveDebt({
          id: form.id || null,
          employee_id: form.employee_id,
          kind: form.kind,
          amount_c: p.amount_c,
          start_date: p.start_date,
          weeks: p.weeks,
          weekly_c: p.weekly_c,
          remit_date: p.remit_date,
          notes: form.notes,
        }),
      );
      open.value = false;
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const p = plan.value;
  return (
    <>
      <Modal open={open} title={form.id ? "Edit debt" : "Add debt"}>
        <form class="space-y-3" preventdefault:submit onSubmit$={save}>
          {error.value && <Alert>{error.value}</Alert>}
          <div class="grid grid-cols-2 gap-3">
            <Field label="Employee" class="col-span-2">
              <select
                class={input}
                required
                disabled={!!form.id && form.paid_c > 0}
                onChange$={(_, el) => (form.employee_id = el.value)}
              >
                <option value="">Choose…</option>
                {employees
                  .filter((e) => e.active || e.id === form.employee_id)
                  .map((e) => (
                    <option key={e.id} value={e.id} selected={e.id === form.employee_id}>
                      {e.name}
                    </option>
                  ))}
              </select>
            </Field>
            <Field label="Type">
              <select class={input} onChange$={(_, el) => (form.kind = el.value as DebtKind)}>
                {DEBT_KINDS.map((k) => (
                  <option key={k.kind} value={k.kind} selected={k.kind === form.kind}>
                    {k.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Amount" hint={form.paid_c ? `${formatPeso(form.paid_c)} deducted so far.` : undefined}>
              <input
                class={input}
                inputMode="decimal"
                required
                value={form.amount}
                placeholder="0.00"
                onInput$={(_, el) => (form.amount = el.value)}
              />
            </Field>
            <Field label="Start date" hint="Charged from the Saturday on or after it.">
              <input
                type="date"
                class={input}
                required
                value={form.start_date}
                onChange$={(_, el) => (form.start_date = el.value)}
              />
            </Field>
            <Field label="Remit date" hint="Should be fully paid by then.">
              <input
                type="date"
                class={input}
                value={form.remit_date}
                onChange$={(_, el) => (form.remit_date = el.value)}
              />
            </Field>
          </div>

          <fieldset class="space-y-2">
            <div class="flex gap-4 text-sm">
              <label class="flex items-center gap-1.5">
                <input
                  type="radio"
                  name="debt-mode"
                  checked={form.mode === "weeks"}
                  onChange$={() => (form.mode = "weeks")}
                />
                Number of weeks
              </label>
              <label class="flex items-center gap-1.5">
                <input
                  type="radio"
                  name="debt-mode"
                  checked={form.mode === "weekly"}
                  onChange$={() => (form.mode = "weekly")}
                />
                Weekly amount
              </label>
            </div>
            {form.mode === "weeks" ? (
              <input
                class={input}
                inputMode="numeric"
                aria-label="Number of weeks"
                placeholder="e.g. 4"
                value={form.weeks}
                onInput$={(_, el) => (form.weeks = el.value)}
              />
            ) : (
              <input
                class={input}
                inputMode="decimal"
                aria-label="Weekly amount"
                placeholder="e.g. 478.50"
                value={form.weekly}
                onInput$={(_, el) => (form.weekly = el.value)}
              />
            )}
            <div class="rounded-md bg-brand-50 px-3 py-2 text-sm text-brand-900" aria-live="polite">
              {planError.value ? (
                <span class="text-danger">{planError.value}</span>
              ) : p ? (
                <>
                  <div class="font-semibold">
                    {formatPeso(p.weekly_c)} a week
                    {p.weeks > 1 && p.last_c !== p.weekly_c && `, last week ${formatPeso(p.last_c)}`}
                    {" · "}
                    {p.weeks} week{p.weeks === 1 ? "" : "s"}
                  </div>
                  <div class="text-xs">
                    {formatDay(p.first_week)} to {formatDay(p.last_week)} (Saturdays)
                  </div>
                </>
              ) : (
                <span class="text-ink-muted">Enter the amount and weeks to see the weekly charge.</span>
              )}
            </div>
            {p?.remit_warning && form.remit_date && (
              <Alert tone="warning">
                The last payment, {formatDay(p.last_week)}, is after the remit date {formatDay(form.remit_date)}.
                Use fewer weeks or a bigger weekly amount.
              </Alert>
            )}
          </fieldset>

          <Field label="Notes">
            <textarea class={input} rows={2} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
          </Field>
          <p class={hint}>Amounts are in steps of ₱0.05. A week the employee doesn't work is skipped, and what it missed is spread over the weeks left.</p>
          <div class="flex items-center justify-between gap-2">
            <span>
              {form.id && canDelete && form.paid_c === 0 && (
                <button
                  type="button"
                  class={[btnGhost, "text-danger"]}
                  onClick$={() => {
                    open.value = false;
                    deleteOpen.value = true;
                  }}
                >
                  Delete
                </button>
              )}
            </span>
            <span class="flex gap-2">
              <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
                Cancel
              </button>
              <button class={btnPrimary} disabled={busy.value}>
                Save
              </button>
            </span>
          </div>
        </form>
      </Modal>
      <Confirm
        open={deleteOpen}
        title="Delete this debt?"
        onConfirm$={$(async () => {
          unwrap(await commands.deleteDebt(form.id));
        })}
      >
        <p>Nothing has been deducted from it yet. This deletes it on every PC.</p>
      </Confirm>
    </>
  );
});
