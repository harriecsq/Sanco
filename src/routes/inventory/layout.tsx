import { component$, Slot } from "@builder.io/qwik";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  if (!can(session.user, "inventory")) return <NoAccess />;
  return (
    <>
      <TabNav
        label="Inventory"
        tabs={[
          { href: "/inventory/", label: "Stock" },
          { href: "/inventory/movements/", label: "Movements" },
          { href: "/inventory/expiry/", label: "Expiring" },
          { href: "/inventory/low/", label: "Low stock" },
          { href: "/inventory/locations/", label: "Locations" },
        ]}
      />
      <Slot />
    </>
  );
});
