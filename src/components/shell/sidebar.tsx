import { component$, useSignal, useVisibleTask$ } from "@builder.io/qwik";
import { Link, useLocation } from "@builder.io/qwik-city";
import { commands, type PeerInfo } from "~/lib/bindings";
import { NAV } from "~/lib/nav";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { Logo } from "./gate";

export const NavIcon = component$<{ d: string; class?: string }>((props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    class={["size-5 shrink-0", props.class]}
    aria-hidden="true"
  >
    <path d={props.d} />
  </svg>
));

function syncSummary(peers: PeerInfo[] | null) {
  if (!peers) return { tone: "muted", text: "Sync status unknown" };
  const total = peers.length;
  const online = peers.filter((p) => p.state === "online").length;
  const blocked = peers.filter((p) => p.state === "blocked").length;
  if (total === 0) return { tone: "muted", text: "No other PCs yet" };
  if (blocked > 0)
    return {
      tone: "warning",
      text: `${blocked} PC${blocked > 1 ? "s" : ""} can't sync`,
    };
  if (online === 0) return { tone: "muted", text: "Offline: no PCs reachable" };
  return {
    tone: "ok",
    text:
      online === total
        ? `Synced · ${total} other PC${total > 1 ? "s" : ""} online`
        : `${online} of ${total} other PCs online`,
  };
}

const dot: Record<string, string> = {
  ok: "bg-brand-500",
  warning: "bg-warning",
  muted: "bg-ink-muted/50",
};

export const Sidebar = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const peers = useSignal<PeerInfo[] | null>(null);

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      track(() => session.syncTick);
      const res = await commands.syncStatus();
      peers.value = res.status === "ok" ? res.data : null;
    },
    { strategy: "document-ready" },
  );

  const user = session.user;
  const path = loc.url.pathname;
  const isActive = (href: string) =>
    href === "/" ? path === "/" : path.startsWith(href);
  const sync = syncSummary(peers.value);
  const canSeeSync = can(user, "settings");

  return (
    <aside class="flex w-60 shrink-0 flex-col border-r border-line bg-white print:hidden">
      <div class="px-4 py-4">
        <Logo />
      </div>
      <nav class="flex-1 space-y-0.5 overflow-y-auto px-2" aria-label="Modules">
        {NAV.filter((n) => can(user, n.module)).map((n) => {
          const active = isActive(n.href);
          return (
            <Link
              key={n.href}
              href={n.href}
              aria-current={active ? "page" : undefined}
              class={[
                "flex items-center gap-3 rounded-md px-3 py-2 text-sm",
                active
                  ? "bg-brand-50 font-semibold text-brand-700"
                  : "text-ink hover:bg-surface-muted",
              ]}
            >
              <NavIcon
                d={n.icon}
                class={active ? "text-brand-500" : "text-ink-muted"}
              />
              {n.label}
            </Link>
          );
        })}
      </nav>
      <div class="space-y-3 border-t border-line px-4 py-3 text-sm">
        {canSeeSync ? (
          <Link
            href="/settings/devices/"
            class="flex items-center gap-2 text-xs text-ink-muted hover:text-brand-700"
          >
            <span class={["size-2 rounded-full", dot[sync.tone]]} />
            {sync.text}
          </Link>
        ) : (
          <p class="flex items-center gap-2 text-xs text-ink-muted">
            <span class={["size-2 rounded-full", dot[sync.tone]]} />
            {sync.text}
          </p>
        )}
        <div>
          <p class="truncate font-medium">{user?.display_name}</p>
          <p class="truncate text-xs text-ink-muted">
            {user?.roles.join(", ") || "No role"}
          </p>
          <div class="mt-1.5 flex gap-3 text-xs">
            <Link href="/account/" class="text-brand-700 hover:underline">
              My account
            </Link>
            <button
              type="button"
              class="text-brand-700 hover:underline"
              onClick$={() => commands.logout()}
            >
              Sign out
            </button>
          </div>
        </div>
      </div>
    </aside>
  );
});
