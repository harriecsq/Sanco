import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import { Link, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type AttendanceStatus, type AttendanceWeek } from "~/lib/bindings";
import { useLiveLoadUrl } from "~/lib/events";
import { formatDay } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { dayHeader, isPaidDay, Legend, MARK, nextMark, useWeek, WeekNav } from "~/components/hr/hr";
import { Alert, Empty, Field, Modal, PageHeader } from "~/components/ui/ui";
import { btnGhost, btnPrimary, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

const TABLES = ["attendance", "employees", "payrolls"];

interface NoteForm {
  employee_id: string;
  name: string;
  date: string;
  status: AttendanceStatus;
  note: string;
}

export default component$(() => {
  const session = useSession();
  const week = useWeek();
  const data = useSignal<AttendanceWeek | null>(null);
  const error = useSignal("");
  const noteOpen = useSignal(false);
  const note = useStore<NoteForm>({ employee_id: "", name: "", date: "", status: "absent", note: "" });
  const holidayOpen = useSignal(false);
  const holiday = useStore<{ date: string; name: string; paid: boolean }>({ date: "", name: "", paid: false });
  const busy = useSignal(false);

  const load = $(async () => {
    if (!can(session.user, "hr")) return;
    try {
      data.value = unwrap(await commands.attendanceWeek(week.value));
      error.value = "";
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoadUrl(TABLES, load);

  const set = $(async (employeeId: string, date: string, status: AttendanceStatus | null, text: string) => {
    const d = data.value;
    if (!d) return;
    // Shown at once; the reload after the write confirms it.
    data.value = {
      ...d,
      rows: d.rows.map((r) =>
        r.employee_id !== employeeId
          ? r
          : { ...r, cells: r.cells.map((c) => (c.date === date ? { ...c, status, note: text || null } : c)) },
      ),
    };
    try {
      unwrap(await commands.setAttendance(employeeId, date, status, text));
    } catch (e) {
      error.value = errorText(e);
      await load();
    }
  });

  const markDay = $(async (date: string, status: AttendanceStatus, text: string, onlyUnmarked: boolean) => {
    busy.value = true;
    try {
      unwrap(await commands.markAttendanceDay(date, status, text, onlyUnmarked));
      error.value = "";
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const d = data.value;
  const editable = !!d && !d.posted && can(session.user, "hr", "edit");

  return (
    <>
      <PageHeader title="Attendance" subtitle="Monday to Saturday. Pay counts green (present) and blue (paid holiday) days.">
        <div q:slot="actions">
          <WeekNav week={week.value} />
        </div>
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      {d?.posted && (
        <div class="mb-3">
          <Alert tone="brand">
            This week's payroll is posted, so attendance is locked.{" "}
            <Link class="font-medium underline" href={`/hr/payroll/?week=${d.week}`}>
              Void the payroll
            </Link>{" "}
            to change it.
          </Alert>
        </div>
      )}
      <div class="mb-3">
        <Legend />
      </div>

      <section class={[card, "overflow-x-auto"]}>
        {!d ? (
          <Empty>Loading…</Empty>
        ) : d.rows.length === 0 ? (
          <Empty>
            No employees yet.{" "}
            <Link class="text-brand-700 hover:underline" href="/hr/employees/">
              Add employees
            </Link>{" "}
            first.
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Employee</th>
                {d.days.map((date) => {
                  const h = dayHeader(date);
                  return (
                    <th key={date} class={[th, "text-center"]}>
                      <div>{h.weekday}</div>
                      <div class="font-normal normal-case">{h.day}</div>
                      {editable && (
                        <div class="mt-1 flex justify-center gap-1 normal-case">
                          <button
                            type="button"
                            class="rounded px-1 text-[11px] font-medium text-emerald-700 hover:bg-emerald-50"
                            title="Mark everyone not marked yet present"
                            disabled={busy.value}
                            onClick$={() => markDay(date, "present", "", true)}
                          >
                            All ✓
                          </button>
                          <button
                            type="button"
                            class="rounded px-1 text-[11px] font-medium text-yellow-700 hover:bg-yellow-50"
                            title="Mark this day a holiday for everyone"
                            disabled={busy.value}
                            onClick$={() => {
                              holiday.date = date;
                              holiday.name = "";
                              holiday.paid = false;
                              holidayOpen.value = true;
                            }}
                          >
                            Holiday
                          </button>
                        </div>
                      )}
                    </th>
                  );
                })}
                <th class={[th, "text-right"]}>Days</th>
                <th class={[th, "text-right"]}>Absent</th>
              </tr>
            </thead>
            <tbody>
              {d.rows.map((r) => {
                const present = r.cells.filter((c) => isPaidDay(c.status)).length;
                const absent = r.cells.filter((c) => c.status === "absent").length;
                return (
                  <tr key={r.employee_id}>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link class="hover:text-brand-700" href={`/hr/employees/?id=${r.employee_id}`}>
                        {r.name}
                      </Link>
                      {!r.active && <span class={[hint, "ml-1"]}>(inactive)</span>}
                    </td>
                    {r.cells.map((c) => {
                      const m = MARK[c.status ?? "none"];
                      const canNote =
                        editable && (c.status === "absent" || c.status === "holiday" || c.status === "paid_holiday");
                      return (
                        <td key={c.date} class={[td, "text-center"]}>
                          <div class="inline-flex flex-col items-center gap-0.5">
                            <button
                              type="button"
                              disabled={!editable}
                              class={[
                                "size-8 rounded border-2 transition-colors",
                                m.box,
                                editable ? "cursor-pointer hover:ring-2 hover:ring-brand-100" : "cursor-default",
                              ]}
                              title={`${m.label}${c.note ? `: ${c.note}` : ""} · ${formatDay(c.date)}`}
                              aria-label={`${r.name}, ${formatDay(c.date)}: ${m.label}`}
                              onClick$={() => set(r.employee_id, c.date, nextMark(c.status), c.note ?? "")}
                            >
                              {c.note ? <span class="text-xs font-bold text-white drop-shadow">•</span> : null}
                            </button>
                            {canNote && (
                              <button
                                type="button"
                                class="text-[11px] text-ink-muted hover:text-brand-700"
                                onClick$={() => {
                                  note.employee_id = r.employee_id;
                                  note.name = r.name;
                                  note.date = c.date;
                                  note.status = c.status as AttendanceStatus;
                                  note.note = c.note ?? "";
                                  noteOpen.value = true;
                                }}
                              >
                                {c.note ? "edit note" : c.status === "absent" ? "reason" : "name"}
                              </button>
                            )}
                          </div>
                        </td>
                      );
                    })}
                    <td class={[td, "text-right tabular-nums"]}>{present}</td>
                    <td class={[td, "text-right tabular-nums", absent > 0 && "text-danger"]}>{absent || ""}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>

      <Modal open={noteOpen} title={note.status === "absent" ? "Reason for absence" : "Holiday"}>
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            await set(note.employee_id, note.date, note.status, note.note.trim());
            noteOpen.value = false;
          }}
        >
          <p class={hint}>
            {note.name}, {formatDay(note.date)}. Optional.
          </p>
          <Field label={note.status === "absent" ? "Reason" : "Holiday name"}>
            <input
              class={input}
              value={note.note}
              placeholder={note.status === "absent" ? "e.g. sick" : "e.g. All Saints' Day"}
              onInput$={(_, el) => (note.note = el.value)}
            />
          </Field>
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (noteOpen.value = false)}>
              Cancel
            </button>
            <button class={btnPrimary}>Save</button>
          </div>
        </form>
      </Modal>

      <Modal open={holidayOpen} title={`Holiday on ${formatDay(holiday.date)}`}>
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            await markDay(holiday.date, holiday.paid ? "paid_holiday" : "holiday", holiday.name.trim(), false);
            holidayOpen.value = false;
          }}
        >
          <p class="text-sm">Marks this day for every active employee, replacing what's there.</p>
          <fieldset class="flex gap-4 text-sm">
            <label class="flex items-center gap-1.5">
              <input type="radio" name="holiday-paid" checked={!holiday.paid} onChange$={() => (holiday.paid = false)} />
              <span class={["inline-block size-4 rounded border", MARK.holiday.box]} />
              Unpaid
            </label>
            <label class="flex items-center gap-1.5">
              <input type="radio" name="holiday-paid" checked={holiday.paid} onChange$={() => (holiday.paid = true)} />
              <span class={["inline-block size-4 rounded border", MARK.paid_holiday.box]} />
              Paid (counts as a day worked)
            </label>
          </fieldset>
          <Field label="Holiday name" hint="Optional.">
            <input
              class={input}
              value={holiday.name}
              placeholder="e.g. Bonifacio Day"
              onInput$={(_, el) => (holiday.name = el.value)}
            />
          </Field>
          <div class="flex justify-end gap-2">
            <button type="button" class={btnGhost} onClick$={() => (holidayOpen.value = false)}>
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Mark holiday
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

export const head: DocumentHead = { title: "Attendance · Sanco" };
