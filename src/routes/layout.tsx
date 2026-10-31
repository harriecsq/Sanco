import {
  $,
  component$,
  Slot,
  useContextProvider,
  useStore,
  useVisibleTask$,
} from "@builder.io/qwik";
import { listen } from "@tauri-apps/api/event";
import { commands } from "~/lib/bindings";
import { SessionCtx, type SessionStore } from "~/lib/session";
import {
  CreateAdmin,
  Login,
  Removed,
  Setup,
  Splash,
} from "~/components/shell/gate";
import { Sidebar } from "~/components/shell/sidebar";
import { UpdateBanner } from "~/components/shell/update-banner";
import { installDatePicker } from "~/lib/date-picker";
import { installSelectSearch } from "~/lib/select-search";

// The whole app sits behind this gate: setup → first admin → sign-in →
// app shell. Status and session come from Rust and refresh on its events.

export default component$(() => {
  const store = useStore<SessionStore>({
    status: null,
    user: null,
    syncTick: 0,
  });
  useContextProvider(SessionCtx, store);

  const refresh = $(async () => {
    const status = await commands.workspaceStatus();
    if (status.status === "ok") store.status = status.data;
    const user = await commands.sessionInfo();
    store.user = user.status === "ok" ? user.data : null;
  });

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    ({ cleanup }) => {
      installDatePicker();
      installSelectSearch();
      refresh();
      // Startup opens the database in the background; poll until it's done.
      const poll = setInterval(() => {
        if (store.status?.starting !== false) refresh();
        else clearInterval(poll);
      }, 300);
      const offs = [
        listen("session-changed", () => refresh()),
        listen("sync-status", () => {
          store.syncTick++;
          refresh();
        }),
      ];
      cleanup(() => {
        clearInterval(poll);
        offs.forEach((p) => p.then((f) => f()));
      });
    },
    { strategy: "document-ready" },
  );

  const s = store.status;
  if (!s || s.starting) return <Splash />;
  if (s.removed) return <Removed onDone$={refresh} />;
  if (!s.ready) return <Setup onDone$={refresh} />;
  if (s.needs_admin) return <CreateAdmin onDone$={refresh} />;
  if (!store.user) return <Login onDone$={refresh} />;

  return (
    <div class="flex h-screen print:block print:h-auto">
      <Sidebar />
      <main class="min-w-0 flex-1 overflow-y-auto bg-surface-muted print:overflow-visible print:bg-white">
        <div class="print:hidden">
          <UpdateBanner />
        </div>
        <div class="mx-auto max-w-6xl px-6 py-6 print:max-w-none print:p-0">
          <Slot />
        </div>
      </main>
    </div>
  );
});
