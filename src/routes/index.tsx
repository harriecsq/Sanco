import { $, component$, useSignal, useVisibleTask$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type DevNote,
  type PeerInfo,
  type WorkspaceStatus,
} from "~/lib/bindings";

// M3 sync test page: set up a workspace, see peers, and add notes that
// should appear on every instance. Replaced by the app shell in M4.

const manila = new Intl.DateTimeFormat("en-PH", {
  timeZone: "Asia/Manila",
  dateStyle: "medium",
  timeStyle: "medium",
});

const stateClass: Record<string, string> = {
  online: "bg-brand-50 text-brand-800",
  offline: "bg-surface-muted text-ink-muted",
  blocked: "bg-red-50 text-danger",
  unknown: "bg-surface-muted text-ink-muted",
};

export default component$(() => {
  const status = useSignal<WorkspaceStatus | null>(null);
  const peers = useSignal<PeerInfo[]>([]);
  const notes = useSignal<DevNote[]>([]);
  const invite = useSignal("");
  const error = useSignal("");
  const busy = useSignal(false);
  const wsName = useSignal("Sanco Medical Supply");
  const joinCode = useSignal("");
  const note = useSignal("");

  const loadStatus = $(async () => {
    const res = await commands.workspaceStatus();
    if (res.status === "ok") status.value = res.data;
  });
  const loadPeers = $(async () => {
    const res = await commands.syncStatus();
    peers.value = res.status === "ok" ? res.data : [];
  });
  const loadNotes = $(async () => {
    const res = await commands.devNotes();
    notes.value = res.status === "ok" ? res.data : [];
  });
  const loadAll = $(async () => {
    await loadStatus();
    if (status.value?.ready) {
      await Promise.all([loadPeers(), loadNotes()]);
    }
  });

  const run = $(
    async (
      action: () => Promise<
        { status: "ok"; data: unknown } | { status: "error"; error: string }
      >,
    ) => {
      busy.value = true;
      error.value = "";
      try {
        const res = await action();
        if (res.status === "error") error.value = res.error;
      } finally {
        busy.value = false;
        await loadAll();
      }
    },
  );

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(({ cleanup }) => {
    loadAll();
    const off = [
      listen("db-changed", () => loadNotes()),
      listen("sync-status", () => {
        loadStatus();
        loadPeers();
      }),
    ];
    cleanup(() => off.forEach((p) => p.then((f) => f())));
  });

  const s = status.value;

  return (
    <main class="mx-auto max-w-3xl space-y-6 p-8">
      <header class="flex items-center justify-between">
        <div class="inline-flex items-center gap-3 rounded-full bg-brand-500 px-5 py-2 text-white">
          <span class="grid size-8 place-items-center bg-white text-lg font-semibold text-brand-500">
            S
          </span>
          <span class="leading-tight">
            <span class="block text-xl">Sanco</span>
            <span class="block text-xs font-semibold">Medical Supply</span>
          </span>
        </div>
        <span class="text-sm text-ink-muted">M3 sync test</span>
      </header>

      {error.value && (
        <p class="rounded-md bg-red-50 px-3 py-2 text-danger">{error.value}</p>
      )}
      {s?.error && !error.value && (
        <p class="rounded-md bg-red-50 px-3 py-2 text-danger">{s.error}</p>
      )}

      {s && !s.ready && (
        <section class="grid gap-6 md:grid-cols-2">
          <form
            class="space-y-3 rounded-lg border border-line p-5"
            preventdefault:submit
            onSubmit$={() => run(() => commands.createWorkspace(wsName.value))}
          >
            <h2 class="font-semibold">New workspace</h2>
            <p class="text-sm text-ink-muted">
              On the first PC only. Other PCs join it.
            </p>
            <input
              class="w-full rounded-md border border-line px-3 py-2"
              bind:value={wsName}
            />
            <button
              class="rounded-md bg-brand-500 px-4 py-2 font-medium text-white hover:bg-brand-600 disabled:opacity-50"
              disabled={busy.value}
            >
              Create
            </button>
          </form>
          <form
            class="space-y-3 rounded-lg border border-line p-5"
            preventdefault:submit
            onSubmit$={() => run(() => commands.joinWorkspace(joinCode.value))}
          >
            <h2 class="font-semibold">Join a workspace</h2>
            <p class="text-sm text-ink-muted">
              Paste the join code from a PC that is already set up.
            </p>
            <textarea
              class="h-24 w-full rounded-md border border-line px-3 py-2 font-mono text-xs"
              bind:value={joinCode}
            />
            <button
              class="rounded-md bg-brand-500 px-4 py-2 font-medium text-white hover:bg-brand-600 disabled:opacity-50"
              disabled={busy.value || !joinCode.value.trim()}
            >
              {busy.value ? "Joining…" : "Join"}
            </button>
          </form>
          <p class="text-xs text-ink-muted md:col-span-2">
            Data folder: {s.data_dir} · port {s.port}
          </p>
        </section>
      )}

      {s?.ready && (
        <>
          <section class="space-y-1 rounded-lg border border-line p-5 text-sm">
            <h2 class="text-base font-semibold">{s.workspace}</h2>
            <p>
              This PC: <b>{s.pc_name}</b> · {s.addr ?? "address unknown"}
            </p>
            <p class="font-mono text-xs text-ink-muted">site {s.site_id}</p>
            <p class="text-xs text-ink-muted">Data folder: {s.data_dir}</p>
            <div class="flex gap-2 pt-2">
              <button
                class="rounded-md border border-line px-3 py-1.5 hover:bg-brand-50"
                onClick$={async () => {
                  const res = await commands.inviteCode();
                  if (res.status === "ok") invite.value = res.data;
                  else error.value = res.error;
                }}
              >
                Show join code
              </button>
              <button
                class="rounded-md border border-line px-3 py-1.5 hover:bg-brand-50 disabled:opacity-50"
                disabled={busy.value}
                onClick$={() => run(() => commands.syncNow())}
              >
                Sync now
              </button>
            </div>
            {invite.value && (
              <div class="space-y-1 pt-2">
                <textarea
                  readOnly
                  class="h-20 w-full rounded-md border border-line bg-surface-muted px-3 py-2 font-mono text-xs"
                  value={invite.value}
                />
                <p class="text-xs text-warning">
                  The code contains the workspace key. Share it only with PCs
                  that should get all the data.
                </p>
              </div>
            )}
          </section>

          <section class="rounded-lg border border-line p-5">
            <h2 class="mb-3 font-semibold">Peers</h2>
            {peers.value.length === 0 ? (
              <p class="text-sm text-ink-muted">No other PCs yet.</p>
            ) : (
              <table class="w-full text-left text-sm">
                <thead class="text-ink-muted">
                  <tr>
                    <th class="py-1">PC</th>
                    <th>Address</th>
                    <th>Status</th>
                    <th>Last pull</th>
                  </tr>
                </thead>
                <tbody>
                  {peers.value.map((p) => (
                    <tr key={p.addr} class="border-t border-line align-top">
                      <td class="py-1.5">{p.name ?? "?"}</td>
                      <td class="font-mono text-xs">{p.addr}</td>
                      <td>
                        <span
                          class={[
                            "rounded px-2 py-0.5 text-xs",
                            stateClass[p.state] ?? stateClass.unknown,
                          ]}
                        >
                          {p.state}
                        </span>
                        {p.error && (
                          <p class="pt-1 text-xs text-ink-muted">{p.error}</p>
                        )}
                      </td>
                      <td class="text-xs">
                        {p.last_pull_ms ? manila.format(p.last_pull_ms) : "—"}
                        {p.last_conflicts > 0 && (
                          <span class="text-warning">
                            {" "}
                            · {p.last_conflicts} conflict(s)
                          </span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          <section class="rounded-lg border border-line p-5">
            <h2 class="mb-3 font-semibold">Test notes</h2>
            <form
              class="mb-3 flex gap-2"
              preventdefault:submit
              onSubmit$={async () => {
                const title = note.value.trim();
                if (!title) return;
                note.value = "";
                await run(() => commands.devAddNote(title));
              }}
            >
              <input
                class="flex-1 rounded-md border border-line px-3 py-2"
                placeholder="Type a note; it should appear on every PC"
                bind:value={note}
              />
              <button class="rounded-md bg-brand-500 px-4 py-2 font-medium text-white hover:bg-brand-600">
                Add
              </button>
            </form>
            <ul class="divide-y divide-line text-sm">
              {notes.value.map((n) => (
                <li key={n.id} class="flex justify-between py-1.5">
                  <span>{n.title}</span>
                  <span class="text-xs text-ink-muted">
                    {manila.format(n.created_at)} ·{" "}
                    {n.site === s.site_id ? "this PC" : n.site.slice(0, 8)}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        </>
      )}
    </main>
  );
});

export const head: DocumentHead = {
  title: "Sanco",
};
