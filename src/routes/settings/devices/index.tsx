import {
  $,
  component$,
  useComputed$,
  useSignal,
  useVisibleTask$,
} from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type DeviceInfo, type PeerInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatAgo } from "~/lib/format";
import { isAdmin } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, Badge, Empty, Modal, PageHeader } from "~/components/ui/ui";
import {
  btnDanger,
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

interface Row {
  key: string;
  name: string;
  addr: string | null;
  version: string | null;
  site: string | null;
  thisPc: boolean;
  lastSeen: number | null;
  peer: PeerInfo | null;
}

const stateTone = {
  online: "brand",
  offline: "muted",
  blocked: "danger",
  unknown: "muted",
} as const;

export default component$(() => {
  const session = useSession();
  const devices = useSignal<DeviceInfo[]>([]);
  const peers = useSignal<PeerInfo[]>([]);
  const error = useSignal("");
  const code = useSignal("");
  const copied = useSignal(false);
  const syncing = useSignal(false);
  const removing = useSignal<Row | null>(null);
  const removeOpen = useSignal(false);
  const admin = isAdmin(session.user);

  const loadPeers = $(async () => {
    const res = await commands.syncStatus();
    if (res.status === "ok") peers.value = res.data;
  });
  const load = $(async () => {
    try {
      devices.value = unwrap(await commands.listDevices());
    } catch (e) {
      error.value = errorText(e);
    }
    await loadPeers();
  });
  useLiveLoad(["devices"], load);
  // Peer status changes arrive as sync-status, which bumps syncTick.
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    ({ track }) => {
      track(() => session.syncTick);
      loadPeers();
    },
    { strategy: "document-ready" },
  );

  const rows = useComputed$<Row[]>(() => {
    const bySite = new Map(
      peers.value.filter((p) => p.site).map((p) => [p.site!, p]),
    );
    const out: Row[] = devices.value.map((d) => {
      const peer = bySite.get(d.site) ?? null;
      bySite.delete(d.site);
      return {
        key: d.site,
        name: d.name ?? "Unnamed PC",
        addr: d.host && d.port ? `${d.host}:${d.port}` : null,
        version: d.app_version,
        site: d.site,
        thisPc: d.this_pc,
        lastSeen: d.last_seen,
        peer,
      };
    });
    // Peers we know only from a poke (their device row hasn't arrived).
    for (const p of peers.value) {
      if (p.site && !bySite.has(p.site)) continue;
      out.push({
        key: p.addr,
        name: p.name ?? "Unknown PC",
        addr: p.addr,
        version: p.app_version,
        site: p.site,
        thisPc: false,
        lastSeen: null,
        peer: p,
      });
    }
    return out.sort((a, b) =>
      a.thisPc ? -1 : b.thisPc ? 1 : a.name.localeCompare(b.name),
    );
  });

  const s = session.status;
  return (
    <>
      <PageHeader
        title="Devices & sync"
        subtitle="Every PC keeps a full copy and syncs with the others over Tailscale, every 30 seconds and right after each change."
      >
        <div q:slot="actions" class="flex gap-2">
          <button
            class={btnSecondary}
            disabled={syncing.value}
            onClick$={async () => {
              syncing.value = true;
              const res = await commands.syncNow();
              if (res.status === "ok") peers.value = res.data;
              syncing.value = false;
            }}
          >
            {syncing.value ? "Syncing…" : "Sync now"}
          </button>
          {admin && (
            <button
              class={btnPrimary}
              onClick$={async () => {
                error.value = "";
                copied.value = false;
                try {
                  code.value = unwrap(await commands.inviteCode());
                } catch (e) {
                  error.value = errorText(e);
                }
              }}
            >
              Show join code
            </button>
          )}
        </div>
      </PageHeader>

      <div class="space-y-4">
        {error.value && <Alert>{error.value}</Alert>}
        {s && !s.addr && (
          <Alert tone="warning">
            This PC's Tailscale address is unknown, so other PCs can't reach it.
            Check that Tailscale is running and signed in. It still syncs by
            reaching out to PCs it already knows.
          </Alert>
        )}

        {code.value && (
          <section class={[card, "space-y-2 p-4"]}>
            <div class="flex items-center justify-between">
              <h2 class="font-semibold">Join code</h2>
              <button class={btnGhost} onClick$={() => (code.value = "")}>
                Hide
              </button>
            </div>
            <textarea
              readOnly
              class={[input, "h-20 bg-surface-muted font-mono text-xs"]}
              value={code.value}
            />
            <div class="flex items-center gap-3">
              <button
                class={btnSecondary}
                onClick$={async () => {
                  await navigator.clipboard.writeText(code.value);
                  copied.value = true;
                }}
              >
                {copied.value ? "Copied" : "Copy"}
              </button>
              <p class="text-xs text-warning">
                The code is the workspace key: whoever has it can download all
                data, including password hashes. Give it only to PCs that should
                join, and don't send it over chat or email.
              </p>
            </div>
          </section>
        )}

        <section class={card}>
          {rows.value.length === 0 ? (
            <Empty>No devices yet.</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>PC</th>
                  <th class={th}>Address</th>
                  <th class={th}>Status</th>
                  <th class={th}>Last sync</th>
                  <th class={th}>Version</th>
                  {admin && <th class={th} />}
                </tr>
              </thead>
              <tbody>
                {rows.value.map((r) => (
                  <tr key={r.key}>
                    <td class={td}>
                      <p class="font-medium">{r.name}</p>
                      {r.site && (
                        <p class="font-mono text-xs text-ink-muted">
                          {r.site.slice(0, 8)}
                        </p>
                      )}
                    </td>
                    <td class={[td, "font-mono text-xs"]}>{r.addr ?? "—"}</td>
                    <td class={td}>
                      {r.thisPc ? (
                        <Badge tone="brand">This PC</Badge>
                      ) : r.peer ? (
                        <>
                          <Badge
                            tone={
                              stateTone[
                                r.peer.state as keyof typeof stateTone
                              ] ?? "muted"
                            }
                          >
                            {r.peer.state}
                          </Badge>
                          {r.peer.error && (
                            <p class="mt-1 max-w-xs text-xs text-ink-muted">
                              {errorText(r.peer.error)}
                            </p>
                          )}
                        </>
                      ) : (
                        <Badge>no address</Badge>
                      )}
                    </td>
                    <td class={[td, "text-xs"]}>
                      {r.thisPc ? (
                        "—"
                      ) : (
                        <>
                          {formatAgo(r.peer?.last_pull_ms)}
                          {!!r.peer?.last_conflicts && (
                            <span class="block text-warning">
                              {r.peer.last_conflicts} conflict(s)
                            </span>
                          )}
                        </>
                      )}
                      {r.lastSeen && (
                        <span class={["block", hint]}>
                          seen {formatAgo(r.lastSeen)}
                        </span>
                      )}
                    </td>
                    <td class={[td, "text-xs"]}>{r.version ?? "—"}</td>
                    {admin && (
                      <td class={[td, "text-right"]}>
                        {!r.thisPc && r.site && r.lastSeen !== null && (
                          <button
                            class={btnGhost}
                            onClick$={() => {
                              removing.value = r;
                              removeOpen.value = true;
                            }}
                          >
                            Remove
                          </button>
                        )}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
        <p class={hint}>
          A PC that is switched off shows as offline; it catches up when it's
          back. Until then, new document numbers show "may change". Remove PCs
          that are gone for good (sold, reinstalled).
        </p>
      </div>

      <Modal open={removeOpen} title="Remove PC">
        <div class="space-y-3 text-sm">
          <p>
            Remove <b>{removing.value?.name}</b> from the workspace? It stops
            syncing as soon as it hears about the removal. To use it again, it
            has to join with a new join code.
          </p>
          <p class={hint}>
            Removing doesn't change the workspace key. If the PC was lost or
            stolen, its copy of the data stays readable.
          </p>
          <div class="flex justify-end gap-2">
            <button
              class={btnSecondary}
              onClick$={() => (removeOpen.value = false)}
            >
              Cancel
            </button>
            <button
              class={btnDanger}
              onClick$={async () => {
                const site = removing.value?.site;
                removeOpen.value = false;
                if (!site) return;
                try {
                  unwrap(await commands.removeDevice(site));
                } catch (e) {
                  error.value = errorText(e);
                }
              }}
            >
              Remove
            </button>
          </div>
        </div>
      </Modal>
    </>
  );
});

export const head: DocumentHead = { title: "Devices & sync · Sanco" };
