import { $, component$, useComputed$, useSignal } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type Backups } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatBytes, formatDateTime } from "~/lib/format";
import { can, isAdmin } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { Alert, Badge, Empty, Modal, PageHeader } from "~/components/ui/ui";
import {
  btnDanger,
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  table,
  td,
  th,
} from "~/components/ui/styles";

const KINDS: Record<string, string> = {
  daily: "Daily",
  manual: "Manual",
  "before-restore": "Before restore",
  "pre-migration": "Before update",
  other: "Other",
};

/** `YYYY-MM-DD` in Manila, to compare with the date filter. */
const manilaDay = (ms: number) =>
  new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Manila" }).format(ms);

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const data = useSignal<Backups | null>(null);
  const error = useSignal("");
  const notice = useSignal("");
  const busy = useSignal(false);
  const restoring = useSignal("");
  const confirmOpen = useSignal(false);

  const load = $(async () => {
    try {
      data.value = unwrap(await commands.listBackups());
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad([], load);

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "kind"]);
    return (data.value?.files ?? []).filter((b) => {
      const day = manilaDay(b.modified_ms);
      return (
        (!f.q || b.name.includes(f.q)) &&
        (!f.kind || b.kind === f.kind) &&
        (!f.from || day >= f.from) &&
        (!f.to || day <= f.to)
      );
    });
  });

  const editable = can(session.user, "settings", "edit");
  const admin = isAdmin(session.user);

  return (
    <>
      <PageHeader
        title="Backups"
        subtitle={`A backup is made every day (the last ${data.value?.keep_daily ?? 14} are kept). Every other PC also holds a full copy.`}
      >
        {editable && (
          <button
            q:slot="actions"
            class={btnPrimary}
            disabled={busy.value}
            onClick$={async () => {
              busy.value = true;
              error.value = "";
              try {
                const name = unwrap(await commands.backupNow());
                notice.value = `Saved ${name}.`;
                await load();
              } catch (e) {
                error.value = errorText(e);
              } finally {
                busy.value = false;
              }
            }}
          >
            {busy.value ? "Backing up…" : "Back up now"}
          </button>
        )}
      </PageHeader>
      <div class="space-y-3">
        {error.value && <Alert>{error.value}</Alert>}
        {notice.value && <Alert tone="brand">{notice.value}</Alert>}
        <FilterBar
          search="Search file name"
          dates
          selects={[
            {
              key: "kind",
              label: "Kinds",
              options: Object.entries(KINDS).map(([value, label]) => ({
                value,
                label,
              })),
            },
          ]}
        />
        <section class={card}>
          {shown.value.length === 0 ? (
            <Empty>No backups yet.</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>Made</th>
                  <th class={th}>Kind</th>
                  <th class={th}>File</th>
                  <th class={[th, "text-right"]}>Size</th>
                  {admin && <th class={th} />}
                </tr>
              </thead>
              <tbody>
                {shown.value.map((b) => (
                  <tr key={b.name}>
                    <td class={td}>{formatDateTime(b.modified_ms)}</td>
                    <td class={td}>
                      <Badge tone={b.kind === "manual" ? "brand" : "muted"}>
                        {KINDS[b.kind] ?? b.kind}
                      </Badge>
                    </td>
                    <td class={[td, "font-mono text-xs"]}>{b.name}</td>
                    <td class={[td, "text-right"]}>{formatBytes(b.size)}</td>
                    {admin && (
                      <td class={[td, "text-right"]}>
                        <button
                          class={btnGhost}
                          onClick$={() => {
                            restoring.value = b.name;
                            confirmOpen.value = true;
                          }}
                        >
                          Restore
                        </button>
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
        {data.value && <p class={hint}>Folder: {data.value.dir}</p>}
      </div>

      <Modal open={confirmOpen} title="Restore this backup?">
        <div class="space-y-3 text-sm">
          <p>
            This PC's data is replaced with <b>{restoring.value}</b>. It then
            syncs again as a new device and gets back everything the other PCs
            still have.
          </p>
          <p>
            <b>
              Changes made on this PC since the backup that never reached
              another PC are lost.
            </b>{" "}
            The current data is saved first as a "Before restore" backup.
          </p>
          <p class={hint}>
            Only needed when this PC's data is damaged. If another PC is fine,
            setting this PC up again with a join code is usually simpler.
          </p>
          <div class="flex justify-end gap-2">
            <button
              class={btnSecondary}
              onClick$={() => (confirmOpen.value = false)}
            >
              Cancel
            </button>
            <button
              class={btnDanger}
              disabled={busy.value}
              onClick$={async () => {
                busy.value = true;
                error.value = "";
                notice.value = "";
                try {
                  unwrap(await commands.restoreBackup(restoring.value));
                  notice.value = `Restored ${restoring.value}. Syncing with the other PCs…`;
                } catch (e) {
                  error.value = errorText(e);
                } finally {
                  busy.value = false;
                  confirmOpen.value = false;
                  await load();
                }
              }}
            >
              {busy.value ? "Restoring…" : "Restore"}
            </button>
          </div>
        </div>
      </Modal>
    </>
  );
});

export const head: DocumentHead = { title: "Backups · Sanco" };
