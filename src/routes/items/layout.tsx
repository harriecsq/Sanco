import { component$, Slot } from "@builder.io/qwik";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  if (!can(session.user, "items")) return <NoAccess />;
  return (
    <>
      <TabNav
        label="Items"
        tabs={[
          { href: "/items/", label: "Catalog" },
          { href: "/items/rates/", label: "Client rates" },
        ]}
      />
      <Slot />
    </>
  );
});
