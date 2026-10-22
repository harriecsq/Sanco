import { $, component$, useSignal, useVisibleTask$ } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { listen } from "@tauri-apps/api/event";
import { commands, type AuditEntry, type UserOption } from "~/lib/bindings";
import { formatDateTime } from "~/lib/format";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { Alert, Badge, Empty, PageHeader } from "~/components/ui/ui";
import { card, hint, table, td, th } from "~/components/ui/styles";

const LIMIT = 500;

const show = (v: unknown) =>
  v === null || v === undefined || v === "" ? "—" : String(v);

/** One line per changed column. */
function describe(e: AuditEntry): string[] {
  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(e.changes_json || "{}");
  } catch {
    return [e.changes_json];
  }
  return Object.entries(parsed).map(([col, v]) =>
    e.action === "update" && Array.isArray(v)
      ? `${col}: ${show(v[0])} → ${show(v[1])}`
      : `${col}: ${show(v)}`,
  );
}

const tone = {
  insert: "brand",
  update: "muted",
  delete: "danger",
  login: "muted",
  logout: "muted",
} as const;

export default component$(() => {
  const loc = useLocation();
  const session = useSession();
  const entries = useSignal<AuditEntry[]>([]);
  const users = useSignal<UserOption[]>([]);
  const tables = useSignal<string[]>([]);
  const error = useSignal("");
  const loading = useSignal(true);

  const load = $(async (url: URL) => {
    const f = filtersOf(url, ["q", "user", "table", "from", "to"]);
    loading.value = true;
    try {
      entries.value = unwrap(
        await commands.auditLog({
          q: f.q || null,
          user_id: f.user || null,
          tbl: f.table || null,
          from: f.from || null,
          to: f.to || null,
          limit: LIMIT,
        }),
      );
      error.value = "";
    } catch (e) {
      error.value = errorText(e);
    } finally {
      loading.value = false;
    }
  });

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      track(() => session.user?.id);
      if (!session.user) return;
      tables.value = await commands.auditTables();
      const res = await commands.userOptions();
      if (res.status === "ok") users.value = res.data;
    },
    { strategy: "document-ready" },
  );

  // Refetch when the filters change, and when anything is written.
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    ({ track, cleanup }) => {
      track(() => loc.url.search);
      track(() => session.user?.id);
      if (!session.user) return;
      load(loc.url);
      const off = listen("db-changed", () => load(loc.url));
      cleanup(() => {
        off.then((f) => f());
      });
    },
    { strategy: "document-ready" },
  );

  return (
    <>
      <PageHeader
        title="Audit log"
        subtitle="Every change, on any PC, with who made it. Sign-ins are logged too."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search values, actions or record id"
        dates
        selects={[
          {
            key: "user",
            label: "Users",
            options: users.value.map((u) => ({ value: u.id, label: u.name })),
          },
          {
            key: "table",
            label: "Tables",
            options: tables.value.map((t) => ({ value: t, label: t })),
          },
        ]}
      />
      <section class={card}>
        {entries.value.length === 0 ? (
          <Empty>{loading.value ? "Loading…" : "No entries match."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>When</th>
                <th class={th}>User</th>
                <th class={th}>Action</th>
                <th class={th}>Record</th>
                <th class={th}>Changes</th>
              </tr>
            </thead>
            <tbody>
              {entries.value.map((e) => {
                const lines = describe(e);
                return (
                  <tr key={e.id}>
                    <td class={[td, "whitespace-nowrap text-xs"]}>
                      {formatDateTime(e.at)}
                    </td>
                    <td class={td}>
                      {e.user_name ?? <span class={hint}>system</span>}
                    </td>
                    <td class={td}>
                      <Badge
                        tone={tone[e.action as keyof typeof tone] ?? "muted"}
                      >
                        {e.action}
                      </Badge>
                    </td>
                    <td class={td}>
                      {e.tbl}
                      <span class="block font-mono text-xs text-ink-muted">
                        {e.row_id.slice(0, 8)}
                      </span>
                    </td>
                    <td class={[td, "max-w-md text-xs"]}>
                      {lines.slice(0, 6).map((l, i) => (
                        <p key={i} class="truncate" title={l}>
                          {l}
                        </p>
                      ))}
                      {lines.length > 6 && (
                        <p class={hint}>+{lines.length - 6} more</p>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
      {entries.value.length >= LIMIT && (
        <p class={["mt-2", hint]}>
          Showing the newest {LIMIT}. Narrow the filters to see older entries.
        </p>
      )}
    </>
  );
});

export const head: DocumentHead = { title: "Audit log · Sanco" };
