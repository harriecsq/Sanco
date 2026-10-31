import { $, component$, useComputed$ } from "@builder.io/qwik";
import { useLocation, useNavigate } from "@builder.io/qwik-city";
import type { AttendanceStatus, DebtKind, DebtStatus } from "~/lib/bindings";
import { addDays, todayManila } from "~/lib/format";
import { btnSecondary } from "~/components/ui/styles";

// A payroll week runs Sunday to Saturday and is named by its Saturday (the
// payday). Attendance is Monday to Saturday.

/** Day of the week of `YYYY-MM-DD`, Sunday = 0 … Saturday = 6. */
export const dayOfWeek = (date: string) => {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d)).getUTCDay();
};

/** The Saturday on or after a date: its payroll week. */
export const weekOf = (date: string) => addDays(date, 6 - dayOfWeek(date));

export const isSaturday = (date: string) =>
  /^\d{4}-\d{2}-\d{2}$/.test(date) && dayOfWeek(date) === 6;

const long = new Intl.DateTimeFormat("en-US", { timeZone: "UTC", month: "long", day: "numeric" });
const short = new Intl.DateTimeFormat("en-US", { timeZone: "UTC", month: "short", day: "numeric" });
const weekdayFmt = new Intl.DateTimeFormat("en-US", { timeZone: "UTC", weekday: "short" });

const utc = (date: string) => {
  const [y, m, d] = date.split("-").map(Number);
  return Date.UTC(y, m - 1, d);
};

/** "October 26 2026 to October 31 2026", as on the printed payroll. */
export const periodLabel = (week: string) => {
  const mon = addDays(week, -5);
  return `${long.format(utc(mon))} ${mon.slice(0, 4)} to ${long.format(utc(week))} ${week.slice(0, 4)}`;
};

/** "Oct 26 – Oct 31, 2026". */
export const weekRange = (week: string) =>
  `${short.format(utc(addDays(week, -5)))} – ${short.format(utc(week))}, ${week.slice(0, 4)}`;

/** "Mon" and "Oct 26" for a column header. */
export const dayHeader = (date: string) => ({
  weekday: weekdayFmt.format(utc(date)),
  day: short.format(utc(date)),
});

/** Payroll columns, in order. */
export const DEBT_KINDS: { kind: DebtKind; label: string; field: DebtField }[] = [
  { kind: "pi_loan", label: "PI Loan", field: "pi_loan_c" },
  { kind: "pi_stl", label: "PI STL", field: "pi_stl_c" },
  { kind: "sss_el", label: "SSS EL", field: "sss_el_c" },
  { kind: "sss_sl", label: "SSS SL", field: "sss_sl_c" },
  { kind: "vale", label: "Vale", field: "vale_c" },
];
export type DebtField = "pi_loan_c" | "pi_stl_c" | "sss_el_c" | "sss_sl_c" | "vale_c";

export const kindLabel = (k: DebtKind) => DEBT_KINDS.find((d) => d.kind === k)?.label ?? k;

export const DEBT_STATUS: Record<DebtStatus, { label: string; tone: "brand" | "muted" | "danger" | "warning" }> = {
  active: { label: "Active", tone: "brand" },
  paid: { label: "Paid", tone: "muted" },
  overdue_schedule: { label: "Overdue schedule", tone: "warning" },
  past_remit: { label: "Past remit date", tone: "danger" },
};

/** Attendance box colors. White = not marked. */
export const MARK: Record<"none" | AttendanceStatus, { label: string; box: string }> = {
  none: { label: "Not marked", box: "bg-white border-line" },
  present: { label: "Present", box: "bg-emerald-500 border-emerald-600" },
  absent: { label: "Absent", box: "bg-red-500 border-red-600" },
  holiday: { label: "Holiday", box: "bg-yellow-300 border-yellow-500" },
};

/** Click order: white → green → red → yellow → white. */
export const nextMark = (s: AttendanceStatus | null): AttendanceStatus | null =>
  s === null ? "present" : s === "present" ? "absent" : s === "absent" ? "holiday" : null;

export const Legend = component$(() => (
  <div class="flex flex-wrap items-center gap-4 text-sm text-ink-muted" aria-label="Legend">
    {(["present", "absent", "holiday", "none"] as const).map((k) => (
      <span key={k} class="flex items-center gap-1.5">
        <span class={["inline-block size-4 rounded border", MARK[k].box]} />
        {MARK[k].label}
      </span>
    ))}
    <span class="text-xs">Click a box to change it. Holidays are unpaid but aren't counted as absences.</span>
  </div>
));

/** The week in `?week=`, or this week. */
export function useWeek() {
  const loc = useLocation();
  return useComputed$(() => {
    const w = loc.url.searchParams.get("week") ?? "";
    return isSaturday(w) ? w : weekOf(todayManila());
  });
}

/** Previous / this / next week, kept in `?week=`. */
export const WeekNav = component$<{ week: string }>(({ week }) => {
  const loc = useLocation();
  const nav = useNavigate();
  const go = $((w: string) => {
    const next = new URLSearchParams(loc.url.search);
    next.set("week", w);
    nav(`${loc.url.pathname}?${next.toString()}`, { replaceState: true, scroll: false });
  });
  const thisWeek = weekOf(todayManila());
  return (
    <div class="flex flex-wrap items-center gap-2">
      <button type="button" class={btnSecondary} aria-label="Previous week" onClick$={() => go(addDays(week, -7))}>
        ‹
      </button>
      <span class="min-w-48 text-center text-sm font-semibold tabular-nums">{weekRange(week)}</span>
      <button type="button" class={btnSecondary} aria-label="Next week" onClick$={() => go(addDays(week, 7))}>
        ›
      </button>
      {week !== thisWeek && (
        <button type="button" class={btnSecondary} onClick$={() => go(thisWeek)}>
          This week
        </button>
      )}
      <input
        type="date"
        class="rounded-md border border-line px-2 py-1 text-sm"
        aria-label="Go to the week of"
        value={week}
        onChange$={(_, el) => el.value && go(weekOf(el.value))}
      />
    </div>
  );
});

/** Centavos for a table cell; blank for zero, like the paper payroll. */
export const amountCell = (c: number) =>
  c ? (c / 100).toLocaleString("en-PH", { minimumFractionDigits: 2, maximumFractionDigits: 2 }) : "";
