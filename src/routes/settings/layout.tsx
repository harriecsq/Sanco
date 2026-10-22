import { component$, Slot } from "@builder.io/qwik";
import { Link, useLocation } from "@builder.io/qwik-city";
import { can, isAdmin } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";

const TABS = [
  { href: "/settings/", label: "This PC", admin: false },
  { href: "/settings/devices/", label: "Devices & sync", admin: false },
  { href: "/settings/users/", label: "Users", admin: true },
  { href: "/settings/roles/", label: "Roles", admin: true },
  { href: "/settings/conflicts/", label: "Conflicts", admin: false },
  { href: "/settings/backups/", label: "Backups", admin: false },
  { href: "/settings/audit/", label: "Audit log", admin: false },
];

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  if (!can(session.user, "settings")) return <NoAccess />;
  const admin = isAdmin(session.user);
  const path = loc.url.pathname;
  return (
    <>
      <nav
        class="mb-5 flex flex-wrap gap-1 border-b border-line"
        aria-label="Settings"
      >
        {TABS.filter((t) => admin || !t.admin).map((t) => {
          const active =
            t.href === "/settings/" ? path === t.href : path.startsWith(t.href);
          return (
            <Link
              key={t.href}
              href={t.href}
              aria-current={active ? "page" : undefined}
              class={[
                "-mb-px border-b-2 px-3 py-2 text-sm",
                active
                  ? "border-brand-500 font-semibold text-brand-700"
                  : "border-transparent text-ink-muted hover:text-ink",
              ]}
            >
              {t.label}
            </Link>
          );
        })}
      </nav>
      <Slot />
    </>
  );
});
