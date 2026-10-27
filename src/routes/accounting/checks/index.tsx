import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type CheckStep, type Choice, type PaymentInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { addDays, formatDay, formatPeso, todayManila } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  CHECK_STATUS,
  CheckBadge,
  CheckStepModal,
  MONEY_TABLES,
  STEP_BUTTON,
  stepsFor,
} from "~/components/accounting/accounting";
import { Alert, Empty, NoAccess, PageHeader } from "~/components/ui/ui";
import { Pager, usePage } from "~/components/ui/pager";
import { btnGhost, card, hint, table, td, th } from "~/components/ui/styles";

const Tile = component$<{ label: string; value: number; note: string; tone?: "danger" | "warning" }>(
  ({ label, value, note, tone }) => (
    <div class={[card, "px-4 py-3"]}>
      <div class="text-xs font-semibold uppercase tracking-wide text-ink-muted">{label}</div>
      <div class={["mt-1 text-lg font-semibold tabular-nums", tone === "danger" && "text-danger", tone === "warning" && "text-amber-800"]}>
        {formatPeso(value)}
      </div>
      <div class={hint}>{note}</div>
    </div>
  ),
);

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const checks = useSignal<PaymentInfo[]>([]);
  const clients = useSignal<Choice[]>([]);
  const error = useSignal("");
  const stepOpen = useSignal(false);
  const step = useSignal<CheckStep>("deposit");
  const target = useSignal<PaymentInfo | null>(null);

  useLiveLoad(
    MONEY_TABLES,
    $(async () => {
      try {
        checks.value = unwrap(await commands.listPayments()).filter((p) => p.check);
        clients.value = unwrap(await commands.clientOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const today = todayManila();
  const weekEnd = addDays(today, 7);
  const live = checks.value.filter((p) => p.status === "posted");
  const sum = (list: PaymentInfo[]) => list.reduce((s, p) => s + p.amount_c, 0);
  const toDeposit = live.filter((p) => p.check!.status === "on_hand" && (p.check!.check_date ?? "") <= weekEnd);
  const pdcs = live.filter((p) => p.check!.status === "on_hand" && p.check!.pdc);
  const deposited = live.filter((p) => p.check!.status === "deposited");
  const bounced = live.filter((p) => p.check!.status === "bounced");

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status", "client"]);
    const q = f.q.toLowerCase();
    const end = addDays(todayManila(), 7);
    return checks.value.filter((p) => {
      const c = p.check!;
      const day = c.check_date ?? "";
      return (
        (!q || [p.no, p.client_name, c.bank, c.check_no, p.or_no].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || day >= f.from) &&
        (!f.to || day <= f.to) &&
        (!f.client || p.client_id === f.client) &&
        (!f.status
          ? p.status === "posted"
          : f.status === "void"
            ? p.status === "void"
            : f.status === "pdc"
              ? p.status === "posted" && c.status === "on_hand" && c.pdc
              : f.status === "to_deposit"
                ? p.status === "posted" && c.status === "on_hand" && day <= end
                : p.status === "posted" && c.status === f.status)
      );
    });
  });
  const pg = usePage(shown);

  if (!can(session.user, "accounting")) return <NoAccess />;
  const canEdit = can(session.user, "accounting", "edit");

  return (
    <>
      <PageHeader
        title="Checks"
        subtitle="Checks received from clients. A check counts as paid until it bounces or is replaced."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <Tile label="To deposit (next 7 days)" value={sum(toDeposit)} note={`${toDeposit.length} check(s)`} tone="warning" />
        <Tile label="PDCs on hand" value={sum(pdcs)} note={`${pdcs.length} check(s)`} />
        <Tile label="Deposited, not cleared" value={sum(deposited)} note={`${deposited.length} check(s)`} />
        <Tile label="Bounced, not replaced" value={sum(bounced)} note={`${bounced.length} check(s)`} tone="danger" />
      </div>
      <FilterBar
        search="Search bank, check no., client"
        dates
        selects={[
          {
            key: "status",
            label: "Statuses",
            allLabel: "All but void",
            options: [
              { value: "to_deposit", label: "To deposit (next 7 days)" },
              { value: "pdc", label: "PDCs on hand" },
              ...(["on_hand", "deposited", "cleared", "bounced", "replaced"] as const).map((s) => ({
                value: s,
                label: CHECK_STATUS[s].label,
              })),
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
        <span class={hint}>Dates filter the check date.</span>
      </FilterBar>
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{checks.value.length ? "No checks match." : "No checks yet. Record one under Payments."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Check date</th>
                <th class={th}>Bank / check no.</th>
                <th class={th}>Client</th>
                <th class={[th, "text-right"]}>Amount</th>
                <th class={th}>Payment</th>
                <th class={th}>Status</th>
                {canEdit && <th class={th}></th>}
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((p) => {
                const c = p.check!;
                const due = c.status === "on_hand" && (c.check_date ?? "") <= today;
                return (
                  <tr key={p.id} class={p.counts ? "" : "text-ink-muted"}>
                    <td class={[td, "whitespace-nowrap"]}>
                      {formatDay(c.check_date)}
                      {due && <span class={[hint, "block text-amber-800"]}>can be deposited</span>}
                    </td>
                    <td class={td}>
                      <span class="font-medium">{c.bank ?? "—"}</span> {c.check_no}
                      {c.bounced_date && (
                        <span class={[hint, "block"]}>
                          Bounced {formatDay(c.bounced_date)}: {c.bounce_reason}
                        </span>
                      )}
                      {c.deposited_date && c.status !== "bounced" && (
                        <span class={[hint, "block"]}>
                          Deposited {formatDay(c.deposited_date)}
                          {c.deposit_account ? ` to ${c.deposit_account}` : ""}
                        </span>
                      )}
                    </td>
                    <td class={td}>{p.client_name ?? "—"}</td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPeso(p.amount_c)}</td>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link class="text-brand-700 hover:underline" href={`/accounting/payments/?id=${p.id}`}>
                        {p.no ?? "—"}
                      </Link>
                      <span class={[hint, "block"]}>received {formatDay(p.date)}</span>
                    </td>
                    <td class={td}>
                      {p.status === "void" ? "Void" : <CheckBadge p={p} />}
                    </td>
                    {canEdit && (
                      <td class={[td, "whitespace-nowrap text-right"]}>
                        {stepsFor(p)
                          .filter((s) => s !== "undo")
                          .map((s) => (
                            <button
                              key={s}
                              class={btnGhost}
                              onClick$={() => {
                                target.value = p;
                                step.value = s;
                                stepOpen.value = true;
                              }}
                            >
                              {STEP_BUTTON[s]}
                            </button>
                          ))}
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
      <CheckStepModal open={stepOpen} payment={target} step={step} />
    </>
  );
});

export const head: DocumentHead = { title: "Checks · Sanco" };
