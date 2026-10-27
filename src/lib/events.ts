import { type QRL, useComputed$, useVisibleTask$ } from "@builder.io/qwik";
import { useLocation } from "@builder.io/qwik-city";
import { isPageKey } from "~/components/ui/pager";
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

/**
 * Like `useLiveLoad`, and also reloads when the query string changes (for
 * screens whose loader reads filters such as a date range). Turning a page
 * of a table doesn't reload.
 */
export function useLiveLoadUrl(tables: string[] | null, fn: QRL<() => unknown>) {
  const session = useSession();
  const loc = useLocation();
  // A computed only notifies when its value changes, so page keys drop out.
  const filters = useComputed$(() => {
    const p = new URLSearchParams(loc.url.search);
    for (const k of [...p.keys()]) if (isPageKey(k)) p.delete(k);
    return p.toString();
  });
  useLiveLoad(tables, fn);
  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(({ track }) => {
    const search = track(() => filters.value);
    if (!session.user || search === undefined) return;
    fn();
  }, { strategy: "document-ready" });
}
