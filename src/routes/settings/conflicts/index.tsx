import {
  $,
  component$,
  useComputed$,
  useSignal,
  useTask$,
} from "@builder.io/qwik";
import { isBrowser } from "@builder.io/qwik/build";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type ConflictInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDateTime } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { Pager, usePage } from "~/components/ui/pager";
import { Alert, Badge, Empty, PageHeader } from "~/components/ui/ui";
import { btnPrimary, btnSecondary, card, hint } from "~/components/ui/styles";

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const list = useSignal<ConflictInfo[]>([]);
  const error = useSignal("");
  const busy = useSignal("");
  const status = useComputed$(() => loc.url.searchParams.get("status") ?? "");

  const load = $(async () => {
    try {
      list.value = unwrap(await commands.listConflicts(status.value !== ""));
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad(["conflicts"], load);
  useTask$(({ track }) => {
    track(() => status.value);
    if (isBrowser) load();
  });

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "status", "table"]);
    const q = f.q.toLowerCase();
    return list.value.filter(
      (c) =>
        (f.status !== "resolved" || c.resolved) &&
        (!f.table || c.tbl === f.table) &&
        (!q ||
          c.row_id.includes(q) ||
          c.fields.some(
            (d) =>
              d.lost.toLowerCase().includes(q) ||
              d.current.toLowerCase().includes(q),
          )),
    );
  });
  // Each conflict is a card with its own field table, so fewer per page.
  const pg = usePage(shown, 20);
  const tables = useComputed$(() =>
    [...new Set(list.value.map((c) => c.tbl))].sort(),
  );

  const resolve = $(async (id: string, restore: boolean) => {
    busy.value = id;
    error.value = "";
    try {
      unwrap(await commands.resolveConflict(id, restore));
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = "";
    }
  });

  const editable = can(session.user, "settings", "edit");

  return (
    <>
      <PageHeader
        title="Conflicts"
        subtitle="When two PCs edit the same record while out of touch, the later edit wins and the other is kept here. Review each one."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search values or record id"
        selects={[
          {
            key: "status",
            label: "Status",
            allLabel: "Open",
            options: [
              { value: "all", label: "Open and resolved" },
              { value: "resolved", label: "Resolved" },
            ],
          },
          {
            key: "table",
            label: "Tables",
            options: tables.value.map((t) => ({ value: t, label: t })),
          },
        ]}
      />
      {shown.value.length === 0 ? (
        <section class={card}>
          <Empty>
            {status.value
              ? "No conflicts match."
              : "No open conflicts. All PCs agree."}
          </Empty>
        </section>
      ) : (
        <section class="space-y-3">
          {pg.value.rows.map((c) => (
            <section key={c.id} class={[card, "p-4"]}>
              <div class="flex flex-wrap items-start justify-between gap-2">
                <div>
                  <p class="font-medium">
                    {c.tbl}{" "}
                    <span class="font-mono text-xs text-ink-muted">
                      {c.row_id.slice(0, 8)}
                    </span>
                    {c.resolved && (
                      <Badge tone="brand">
                        resolved{c.resolved_by ? ` by ${c.resolved_by}` : ""}
                      </Badge>
                    )}
                    {c.row_deleted && (
                      <Badge tone="danger">record deleted</Badge>
                    )}
                  </p>
                  <p class={hint}>
                    Lost edit by {c.lost_by ?? "system"} ·{" "}
                    {formatDateTime(c.lost_at)}
                  </p>
                </div>
                {!c.resolved && editable && (
                  <div class="flex gap-2">
                    <button
                      class={btnSecondary}
                      disabled={!!busy.value}
                      onClick$={() => resolve(c.id, false)}
                    >
                      Keep current
                    </button>
                    <button
                      class={btnPrimary}
                      disabled={!!busy.value || c.row_deleted}
                      title={
                        c.row_deleted
                          ? "The record was deleted, so the lost values can't be put back"
                          : undefined
                      }
                      onClick$={() => resolve(c.id, true)}
                    >
                      Use lost values
                    </button>
                  </div>
                )}
              </div>
              {c.fields.length > 0 ? (
                <table class="mt-3 w-full text-sm">
                  <thead>
                    <tr class="text-left text-xs text-ink-muted">
                      <th class="py-1 font-medium">Field</th>
                      <th class="py-1 font-medium">Lost edit</th>
                      <th class="py-1 font-medium">Current</th>
                    </tr>
                  </thead>
                  <tbody>
                    {c.fields.map((d) => (
                      <tr key={d.column} class="border-t border-line align-top">
                        <td class="py-1.5 pr-3 font-mono text-xs">
                          {d.column}
                        </td>
                        <td class="py-1.5 pr-3 text-amber-800">
                          {d.lost || "—"}
                        </td>
                        <td class="py-1.5">{d.current || "—"}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : (
                <p class={["mt-2", hint]}>
                  The current record already has the same values.
                </p>
              )}
            </section>
          ))}
          <div class={[card, "[&>div]:border-t-0"]}>
            <Pager of={pg} />
          </div>
        </section>
      )}
    </>
  );
});

export const head: DocumentHead = { title: "Conflicts · Sanco" };
