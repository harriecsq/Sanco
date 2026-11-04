import { component$, Slot } from "@builder.io/qwik";
import { can, isAdmin } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

const TABS = [
  { href: "/settings/", label: "This PC", admin: false },
  { href: "/settings/company/", label: "Company", admin: false },
  { href: "/settings/pricing/", label: "Pricing", admin: false },
  { href: "/settings/numbers/", label: "Number formats", admin: false },
  { href: "/settings/devices/", label: "Devices & sync", admin: false },
  { href: "/settings/users/", label: "Users", admin: true },
  { href: "/settings/roles/", label: "Roles", admin: true },
  { href: "/settings/conflicts/", label: "Conflicts", admin: false },
  { href: "/settings/backups/", label: "Backups", admin: false },
  { href: "/settings/audit/", label: "Audit log", admin: false },
];

export default component$(() => {
  const session = useSession();
  if (!can(session.user, "settings")) return <NoAccess />;
  const admin = isAdmin(session.user);
  return (
    <>
      <TabNav
        label="Settings"
        tabs={TABS.filter((t) => admin || !t.admin)}
      />
      <Slot />
    </>
  );
});
