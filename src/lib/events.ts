import { type QRL, useVisibleTask$ } from "@builder.io/qwik";
import { listen } from "@tauri-apps/api/event";
import { useSession } from "./session";

/**
 * Calls `fn` in the browser once someone is signed in, then again whenever
 * one of `tables` changes (`db-changed`, local or synced) or the user
 * changes. `null` means any table.
 *
 * Screens are prerendered before anyone signs in, so loaders must read
 * permissions from the session when they run, not capture them at render.
 */
export function useLiveLoad(tables: string[] | null, fn: QRL<() => unknown>) {
  const session = useSession();
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(({ track, cleanup }) => {
    track(() => session.user?.id);
    if (!session.user) return;
    fn();
    const off = listen<string[]>("db-changed", (e) => {
      const changed = e.payload ?? [];
      if (
        !tables ||
        changed.length === 0 ||
        changed.some((t) => tables.includes(t))
      ) {
        fn();
      }
    });
    cleanup(() => {
      off.then((f) => f());
    });
  }, { strategy: "document-ready" });
}
