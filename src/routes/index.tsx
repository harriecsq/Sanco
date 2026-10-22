import { $, component$, useSignal, useStore, type Signal } from "@builder.io/qwik";
import { Link, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type Dashboard,
  type Kpi,
  type PeerInfo,
  type TodoInfo,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatPeso, todayManila } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, Empty, Field, Modal, NoAccess, PageHeader } from "~/components/ui/ui";
import {
  btnDanger,
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  input,
} from "~/components/ui/styles";

// Tables whose changes can move a KPI or an issue.
const DASHBOARD_TABLES = [
  "bd_requests",
  "bookings",
  "booking_lines",
  "delivery_receipts",
  "dr_lines",
  "payments",
  "payment_allocations",
  "stock_docs",
  "stock_doc_lines",
  "items",
  "clients",
  "purchase_orders",
  "po_lines",
  "po_receipts",
  "po_receipt_lines",
  "conflicts",
  "doc_numbers",
];

const TONE_TEXT: Record<Kpi["tone"], string> = {
  normal: "text-ink",
  warn: "text-warning",
  bad: "text-danger",
};

/** Where a to-do's document opens. */
const REF_HREF: Record<string, string> = {
  delivery_receipts: "/accounting/",
  payments: "/accounting/payments/",
  bookings: "/bookings/",
  bd_requests: "/bd/",
  purchase_orders: "/orders/",
  po_receipts: "/orders/receiving/",
  stock_docs: "/inventory/movements/",
  clients: "/clients/",
  items: "/items/",
};

interface TodoForm {
  id: string;
  title: string;
  notes: string;
  due: string;
  assignee: string;
}

const blankTodo = (): TodoForm => ({ id: "", title: "", notes: "", due: "", assignee: "" });

export default component$(() => {
  const session = useSession();
  const dash = useSignal<Dashboard | null>(null);
  const blocked = useSignal<PeerInfo[]>([]);
  const todos = useSignal<TodoInfo[]>([]);
  const users = useSignal<{ id: string; name: string }[]>([]);
  const showDone = useSignal(false);
  const error = useSignal("");
  const todoOpen = useSignal(false);
  const form = useStore<TodoForm>(blankTodo());

  const loadDash = $(async () => {
    if (!can(session.user, "dashboard")) return;
    try {
      dash.value = unwrap(await commands.dashboard());
      const peers = await commands.syncStatus();
      blocked.value = peers.status === "ok" ? peers.data.filter((p) => p.state === "blocked") : [];
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad(DASHBOARD_TABLES, loadDash);

  const loadTodos = $(async () => {
    try {
      todos.value = unwrap(await commands.listTodos(showDone.value));
      if (users.value.length === 0) users.value = unwrap(await commands.userOptions());
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad(["todos", "users"], loadTodos);

  const editTodo = $((t: TodoInfo | null) => {
    Object.assign(
      form,
      t
        ? { id: t.id, title: t.title, notes: t.notes ?? "", due: t.due ?? "", assignee: t.assignee_id ?? "" }
        : blankTodo(),
    );
    todoOpen.value = true;
  });

  if (!can(session.user, "dashboard")) return <NoAccess />;
  const s = session.status;
  const hour = Number(
    new Intl.DateTimeFormat("en-PH", {
      timeZone: "Asia/Manila",
      hour: "numeric",
      hourCycle: "h23",
    }).format(new Date()),
  );
  const greeting = hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
  const today = todayManila();
  const d = dash.value;
  const me = session.user?.id;

  return (
    <>
      <PageHeader
        title={`${greeting}, ${session.user?.display_name ?? ""}`}
        subtitle={`${s?.workspace ?? ""} · ${s?.pc_name ?? ""}`}
      />
      {error.value && <Alert>{error.value}</Alert>}

      {(blocked.value.length > 0 || (d && d.issues.length > 0)) && (
        <section class="mb-5 space-y-2" aria-label="Needs attention">
          <h2 class="text-xs font-semibold uppercase tracking-wide text-ink-muted">Needs attention</h2>
          {blocked.value.map((p) => (
            <div key={p.addr} class="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900">
              <strong>{p.name ?? p.addr}</strong> can't sync: {p.error ?? "refused"}
            </div>
          ))}
          {d?.issues.map((i) => (
            <Link
              key={i.key}
              href={i.href}
              class="flex items-start gap-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900 hover:border-amber-300"
            >
              <span class="min-w-6 rounded bg-white px-1.5 text-center font-semibold">{i.count}</span>
              <span>
                <strong>{i.title}</strong>
                <span class="block text-amber-800">{i.detail}</span>
              </span>
            </Link>
          ))}
        </section>
      )}

      <div class="grid gap-5 lg:grid-cols-[minmax(0,1fr)_22rem]">
        <section aria-label="Key figures">
          {d && d.kpis.length === 0 && (
            <div class={[card, "p-4 text-sm text-ink-muted"]}>Your role doesn't show any module figures.</div>
          )}
          <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {d?.kpis.map((k) => (
              <Link key={k.key} href={k.href} class={[card, "block p-4 hover:border-brand-300"]}>
                <p class="text-xs font-semibold uppercase tracking-wide text-ink-muted">{k.label}</p>
                <p class={["mt-1 text-2xl font-semibold tabular-nums", TONE_TEXT[k.tone]]}>
                  {k.money ? formatPeso(k.value) : k.value}
                </p>
                <p class="text-xs text-ink-muted">{k.hint}</p>
              </Link>
            ))}
          </div>
        </section>

        <section class={[card, "self-start"]} aria-label="To-dos">
          <div class="flex items-center justify-between border-b border-line px-4 py-2.5">
            <h2 class="font-semibold">To-dos</h2>
            <div class="flex items-center gap-1">
              <label class="flex items-center gap-1 text-xs text-ink-muted">
                <input
                  type="checkbox"
                  checked={showDone.value}
                  onChange$={(_, el) => {
                    showDone.value = el.checked;
                    loadTodos();
                  }}
                />
                Done
              </label>
              <button type="button" class={btnGhost} onClick$={() => editTodo(null)}>
                + Add
              </button>
            </div>
          </div>
          {todos.value.length === 0 ? (
            <Empty>Nothing to do.</Empty>
          ) : (
            <ul class="divide-y divide-line">
              {todos.value.map((t) => {
                const late = !t.done && !!t.due && t.due < today;
                const refBase = t.ref_tbl ? REF_HREF[t.ref_tbl] : undefined;
                return (
                  <li key={t.id} class="flex items-start gap-2.5 px-4 py-2.5 text-sm">
                    <input
                      type="checkbox"
                      class="mt-0.5"
                      aria-label={`Done: ${t.title}`}
                      checked={t.done}
                      onChange$={async (_, el) => {
                        const res = await commands.setTodoDone(t.id, el.checked);
                        if (res.status === "error") error.value = errorText(res.error);
                      }}
                    />
                    <div class="min-w-0 flex-1">
                      <button
                        type="button"
                        class={["text-left hover:underline", t.done && "text-ink-muted line-through"]}
                        onClick$={() => editTodo(t)}
                      >
                        {t.title}
                      </button>
                      <p class="text-xs text-ink-muted">
                        {t.due && (
                          <span class={late ? "font-medium text-danger" : ""}>
                            {late ? "Overdue · " : "Due "}
                            {formatDay(t.due)}
                            {" · "}
                          </span>
                        )}
                        {t.assignee_id
                          ? t.assignee_id === me
                            ? "Mine"
                            : `For ${t.assignee_name ?? "?"}`
                          : "Shared"}
                        {t.created_by && t.created_by !== me && ` · from ${t.created_by_name ?? "?"}`}
                        {refBase && t.ref_id && (
                          <>
                            {" · "}
                            <Link class="text-brand-700 hover:underline" href={`${refBase}?id=${t.ref_id}`}>
                              {t.ref_no ?? "Open"}
                            </Link>
                          </>
                        )}
                      </p>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>

      <Modal open={todoOpen} title={form.id ? "Edit to-do" : "New to-do"}>
        <TodoEditor form={form} open={todoOpen} users={users} me={me ?? ""} />
      </Modal>
    </>
  );
});

const TodoEditor = component$<{
  form: TodoForm;
  open: Signal<boolean>;
  users: Signal<{ id: string; name: string }[]>;
  me: string;
}>(({ form, open, users, me }) => {
  const error = useSignal("");
  const busy = useSignal(false);
  return (
    <form
      class="space-y-3"
      preventdefault:submit
      onSubmit$={async () => {
        busy.value = true;
        error.value = "";
        try {
          unwrap(
            await commands.saveTodo({
              id: form.id || null,
              title: form.title,
              notes: form.notes,
              due: form.due || null,
              assignee_id: form.assignee || null,
            }),
          );
          busy.value = false;
          open.value = false;
        } catch (e) {
          error.value = errorText(e);
          busy.value = false;
        }
      }}
    >
      {error.value && <Alert>{error.value}</Alert>}
      <Field label="To-do">
        <input class={input} required maxLength={200} value={form.title} onInput$={(_, el) => (form.title = el.value)} />
      </Field>
      <div class="grid grid-cols-2 gap-3">
        <Field label="Due">
          <input type="date" class={input} value={form.due} onChange$={(_, el) => (form.due = el.value)} />
        </Field>
        <Field label="For">
          <select class={input} onChange$={(_, el) => (form.assignee = el.value)}>
            <option value="" selected={!form.assignee}>
              Everyone (shared)
            </option>
            {users.value.map((u) => (
              <option key={u.id} value={u.id} selected={u.id === form.assignee}>
                {u.id === me ? `${u.name} (me)` : u.name}
              </option>
            ))}
          </select>
        </Field>
      </div>
      <Field label="Notes">
        <textarea class={input} rows={3} value={form.notes} onInput$={(_, el) => (form.notes = el.value)} />
      </Field>
      <div class="flex items-center justify-between gap-2">
        <div>
          {form.id && (
            <button
              type="button"
              class={btnDanger}
              disabled={busy.value}
              onClick$={async () => {
                busy.value = true;
                const res = await commands.deleteTodo(form.id);
                busy.value = false;
                if (res.status === "error") error.value = errorText(res.error);
                else open.value = false;
              }}
            >
              Delete
            </button>
          )}
        </div>
        <div class="flex gap-2">
          <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
            Cancel
          </button>
          <button type="submit" class={btnPrimary} disabled={busy.value}>
            Save
          </button>
        </div>
      </div>
    </form>
  );
});

export const head: DocumentHead = {
  title: "Dashboard · Sanco",
};
