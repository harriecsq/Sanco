import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type BankAccountInfo, type BankMove, type BankMoveKind } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatMinor, formatPeso, formatPesoOr } from "~/lib/format";
import { can, sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { CHECK_STATUS } from "~/components/accounting/accounting";
import { bankAccountLabel } from "~/components/accounting/bank-account-select";
import { CURRENCIES } from "~/components/items/cost-sheet";
import { Alert, Badge, Confirm, Empty, Field, Modal, NoAccess, PageHeader } from "~/components/ui/ui";
import { isPageKey, Pager, usePage } from "~/components/ui/pager";
import {
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  table,
  td,
  th,
} from "~/components/ui/styles";

const TABLES = [
  "bank_accounts",
  "payments",
  "supplier_payments",
  "purchase_orders",
  "clients",
  "suppliers",
  "doc_numbers",
];

const KIND_LABEL: Record<BankMoveKind, string> = {
  check_deposit: "Check deposit",
  transfer_in: "Transfer in",
  tt_out: "TT to supplier",
};

/** A move with the account it belongs to. */
type Row = BankMove & { account_id: string };

interface AccountForm {
  id: string;
  name: string;
  bank: string;
  account_no: string;
  currency: string;
  notes: string;
  active: boolean;
}

const blankForm = (): AccountForm => ({
  id: "",
  name: "",
  bank: "",
  account_no: "",
  currency: "PHP",
  notes: "",
  active: true,
});

const toForm = (a: BankAccountInfo): AccountForm => ({
  id: a.id,
  name: a.name,
  bank: a.bank ?? "",
  account_no: a.account_no ?? "",
  currency: a.currency,
  notes: a.notes ?? "",
  active: a.active,
});

/** Money in counts once it has landed: cleared checks and transfers. */
const landedIn = (m: BankMove) =>
  m.kind === "transfer_in" || (m.kind === "check_deposit" && m.check_status === "cleared");

const Figure = component$<{ label: string; value: string; note?: string; tone?: "danger" | "warning" }>(
  ({ label, value, note, tone }) => (
    <div>
      <div class="text-xs text-ink-muted">{label}</div>
      <div
        class={[
          "font-semibold tabular-nums",
          tone === "danger" && "text-danger",
          tone === "warning" && "text-amber-800",
        ]}
      >
        {value}
      </div>
      {note && <div class="text-xs text-ink-muted">{note}</div>}
    </div>
  ),
);

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const accounts = useSignal<BankAccountInfo[]>([]);
  const rows = useSignal<Row[]>([]);
  const error = useSignal("");
  const formOpen = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);
  const form = useStore<AccountForm>(blankForm());
  const deleteOpen = useSignal(false);

  useLiveLoad(
    TABLES,
    $(async () => {
      if (!can(session.user, "accounting")) return;
      try {
        const list = unwrap(await commands.listBankAccounts());
        const moves = await Promise.all(
          list.map(async (a) =>
            unwrap(await commands.bankAccountMoves(a.id)).map((m) => ({ ...m, account_id: a.id })),
          ),
        );
        accounts.value = list;
        rows.value = moves.flat().sort((a, b) => b.date.localeCompare(a.date) || b.id.localeCompare(a.id));
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "account", "kind", "status"]);
    const q = f.q.toLowerCase();
    return rows.value.filter(
      (m) =>
        (!q || [m.no, m.party_name, m.detail].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || m.date >= f.from) &&
        (!f.to || m.date <= f.to) &&
        (!f.account || m.account_id === f.account) &&
        (!f.kind || m.kind === f.kind) &&
        (!f.status || m.check_status === f.status),
    );
  });
  const pg = usePage(shown);
  const selected = useComputed$(() => loc.url.searchParams.get("account") ?? "");

  const pick = $((id: string) => {
    const next = new URLSearchParams(loc.url.search);
    if (id && next.get("account") !== id) next.set("account", id);
    else next.delete("account");
    // Another account's moves start at their first page.
    for (const k of [...next.keys()]) if (isPageKey(k)) next.delete(k);
    const qs = next.toString();
    nav(`${loc.url.pathname}${qs ? `?${qs}` : ""}`, { replaceState: true, scroll: false });
  });

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      unwrap(
        await commands.saveBankAccount({
          id: form.id || null,
          name: form.name,
          bank: form.bank,
          account_no: form.account_no,
          currency: form.currency,
          notes: form.notes,
          active: form.active,
        }),
      );
      formOpen.value = false;
    } catch (e) {
      formError.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  if (!can(session.user, "accounting")) return <NoAccess />;
  const canCreate = can(session.user, "accounting", "create");
  const canEdit = can(session.user, "accounting", "edit");
  const canDelete = can(session.user, "accounting", "delete");
  const costs = sees(session.user, "costs");
  const nameOf = (id: string) => accounts.value.find((a) => a.id === id)?.name ?? "—";

  const list = shown.value;
  const totalIn = list.filter(landedIn).reduce((s, m) => s + (m.amount_c ?? 0), 0);
  const pending = list.filter((m) => m.check_status === "deposited");
  const totalPending = pending.reduce((s, m) => s + (m.amount_c ?? 0), 0);
  const outs = list.filter((m) => m.kind === "tt_out");
  const totalOut = outs.reduce((s, m) => s + (m.amount_c ?? 0), 0);
  const outMissingFx = outs.some((m) => m.amount_c == null);

  return (
    <>
      <PageHeader
        title="Bank accounts"
        subtitle="The company's accounts, with the checks deposited, transfers received and TTs paid from each."
      >
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
            Add account
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}

      {accounts.value.length === 0 ? (
        <section class={[card, "mb-4"]}>
          <Empty>
            No bank accounts yet.
            {canCreate && " Add the accounts you deposit checks to and pay suppliers from."}
          </Empty>
        </section>
      ) : (
        <div class="mb-4 grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {accounts.value.map((a) => (
            <div
              key={a.id}
              class={[
                card,
                "flex flex-col gap-3 px-4 py-3",
                selected.value === a.id && "border-brand-500 ring-2 ring-brand-100",
                !a.active && "opacity-70",
              ]}
            >
              <div class="flex items-start justify-between gap-2">
                <button
                  type="button"
                  class="min-w-0 text-left"
                  title={selected.value === a.id ? "Show every account" : "Show only this account"}
                  onClick$={() => pick(a.id)}
                >
                  <div class="flex flex-wrap items-center gap-2">
                    <span class="font-semibold text-ink hover:text-brand-700">{a.name}</span>
                    {!a.active && <Badge>Inactive</Badge>}
                    {a.currency !== "PHP" && <Badge tone="brand">{a.currency}</Badge>}
                  </div>
                  <div class={hint}>{[a.bank, a.account_no].filter(Boolean).join(" · ") || "—"}</div>
                </button>
                {canEdit && (
                  <button
                    type="button"
                    class={btnGhost}
                    onClick$={() => {
                      Object.assign(form, toForm(a));
                      formError.value = "";
                      formOpen.value = true;
                    }}
                  >
                    Edit
                  </button>
                )}
              </div>
              <div class="grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3">
                <Figure label="Cleared checks" value={formatPeso(a.checks_cleared_c)} />
                <Figure
                  label="Deposited, not cleared"
                  value={formatPeso(a.checks_pending_c)}
                  note={`${a.checks_pending} check(s)`}
                  tone={a.checks_pending > 0 ? "warning" : undefined}
                />
                <Figure label="Transfers in" value={formatPeso(a.transfers_in_c)} />
                {a.checks_bounced > 0 && (
                  <Figure label="Bounced" value={`${a.checks_bounced} check(s)`} tone="danger" />
                )}
                {a.tt_count != null && a.tt_count > 0 && (
                  <Figure
                    label="TTs paid"
                    value={a.tt_out_php_c == null ? "—" : formatPeso(a.tt_out_php_c)}
                    note={`${a.tt_count} payment(s)`}
                  />
                )}
              </div>
              <div class={hint}>
                {a.last_date ? `Last activity ${formatDay(a.last_date)}` : "No activity yet"}
              </div>
            </div>
          ))}
        </div>
      )}

      <FilterBar
        search="Search payment, PO, client, supplier, check no."
        dates
        selects={[
          {
            key: "account",
            label: "Accounts",
            options: accounts.value.map((a) => ({ value: a.id, label: bankAccountLabel(a) })),
          },
          {
            key: "kind",
            label: "Types",
            options: (Object.keys(KIND_LABEL) as BankMoveKind[])
              .filter((k) => costs || k !== "tt_out")
              .map((k) => ({ value: k, label: KIND_LABEL[k] })),
          },
          {
            key: "status",
            label: "Check statuses",
            options: (["deposited", "cleared", "bounced"] as const).map((s) => ({
              value: s,
              label: CHECK_STATUS[s].label,
            })),
          },
        ]}
      >
        <span class={hint}>Dates filter the deposit date for checks.</span>
      </FilterBar>

      <section class={[card, "overflow-x-auto"]}>
        {list.length === 0 ? (
          <Empty>
            {rows.value.length
              ? "Nothing matches."
              : "Nothing yet. Deposit a check, record a bank transfer or add a TT payment with an account."}
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Date</th>
                {!selected.value && <th class={th}>Account</th>}
                <th class={th}>Type</th>
                <th class={th}>Payment / PO</th>
                <th class={th}>From / to</th>
                <th class={th}>Details</th>
                <th class={th}>Status</th>
                <th class={[th, "text-right"]}>In</th>
                <th class={[th, "text-right"]}>Out</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((m) => {
                const out = m.kind === "tt_out";
                const s = m.check_status ? CHECK_STATUS[m.check_status] : null;
                return (
                  <tr key={`${m.kind}-${m.id}`} class={m.check_status === "bounced" ? "text-ink-muted" : ""}>
                    <td class={[td, "whitespace-nowrap"]}>{formatDay(m.date)}</td>
                    {!selected.value && <td class={td}>{nameOf(m.account_id)}</td>}
                    <td class={[td, "whitespace-nowrap"]}>{KIND_LABEL[m.kind]}</td>
                    <td class={[td, "whitespace-nowrap"]}>
                      <Link
                        class="text-brand-700 hover:underline"
                        href={out ? `/orders/?id=${m.po_id}` : `/accounting/payments/?id=${m.id}`}
                      >
                        {m.no ?? "—"}
                      </Link>
                    </td>
                    <td class={td}>{m.party_name ?? "—"}</td>
                    <td class={td}>{m.detail ?? "—"}</td>
                    <td class={[td, "whitespace-nowrap"]}>
                      {s ? <Badge tone={s.tone}>{s.label}</Badge> : <Badge tone="brand">{out ? "Paid" : "Received"}</Badge>}
                      {m.cleared_date && <span class={[hint, "block"]}>cleared {formatDay(m.cleared_date)}</span>}
                      {m.bounced_date && <span class={[hint, "block"]}>bounced {formatDay(m.bounced_date)}</span>}
                    </td>
                    <td class={[td, "text-right tabular-nums", m.check_status === "bounced" && "line-through"]}>
                      {out ? "" : formatPesoOr(m.amount_c)}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>
                      {out && (
                        <>
                          {formatPesoOr(m.amount_c)}
                          {m.currency !== "PHP" && (
                            <span class={[hint, "block"]}>{formatMinor(m.amount_minor, m.currency)}</span>
                          )}
                        </>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
            <tfoot>
              <tr>
                <td class={[td, "font-semibold"]} colSpan={selected.value ? 6 : 7}>
                  Total shown
                  <span class={[hint, "block font-normal"]}>
                    In counts cleared checks and transfers.
                    {pending.length > 0 && ` ${formatPeso(totalPending)} in ${pending.length} deposited check(s) not cleared yet.`}
                  </span>
                </td>
                <td class={[td, "text-right font-semibold tabular-nums"]}>{formatPeso(totalIn)}</td>
                <td class={[td, "text-right font-semibold tabular-nums"]}>
                  {outs.length ? formatPeso(totalOut) : ""}
                  {outMissingFx && <span class={[hint, "block font-normal"]}>some without a rate</span>}
                </td>
              </tr>
            </tfoot>
          </table>
        )}
        <Pager of={pg} />
      </section>

      <Modal open={formOpen} title={form.id ? "Edit bank account" : "Add bank account"}>
        <form class="space-y-3" preventdefault:submit onSubmit$={save}>
          {formError.value && <Alert>{formError.value}</Alert>}
          <Field label="Name" hint="What staff pick from, e.g. BDO Savings – Main.">
            <input class={input} required value={form.name} onInput$={(_, el) => (form.name = el.value)} />
          </Field>
          <div class="grid grid-cols-2 gap-3">
            <Field label="Bank">
              <input class={input} value={form.bank} onInput$={(_, el) => (form.bank = el.value)} />
            </Field>
            <Field label="Account no.">
              <input
                class={input}
                value={form.account_no}
                onInput$={(_, el) => (form.account_no = el.value)}
              />
            </Field>
            <Field label="Currency">
              <select class={input} onChange$={(_, el) => (form.currency = el.value)}>
                {CURRENCIES.map((c) => (
                  <option key={c} value={c} selected={c === form.currency}>
                    {c}
                  </option>
                ))}
              </select>
            </Field>
            <label class="flex items-center gap-2 self-end pb-2 text-sm">
              <input
                type="checkbox"
                checked={form.active}
                onChange$={(_, el) => (form.active = el.checked)}
              />
              Active
            </label>
          </div>
          <Field label="Notes">
            <textarea class={input} rows={2} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
          </Field>
          <p class={hint}>Inactive accounts stay on past payments but can't be picked for new ones.</p>
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

      <Confirm
        open={deleteOpen}
        title={`Delete ${form.name || "bank account"}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteBankAccount(form.id));
          if (selected.value === form.id) await pick(form.id);
        })}
      >
        <p>
          This deletes the account on every PC and can't be undone. An account that payments or TTs
          name can't be deleted; mark it inactive instead.
        </p>
      </Confirm>
    </>
  );
});

export const head: DocumentHead = { title: "Bank accounts · Sanco" };
