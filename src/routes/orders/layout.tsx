import { component$, Slot } from "@builder.io/qwik";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  if (!can(session.user, "orders")) return <NoAccess />;
  return (
    <>
      <TabNav
        label="Order"
        tabs={[
          { href: "/orders/", label: "Purchase orders" },
          { href: "/orders/suppliers/", label: "Suppliers" },
        ]}
      />
      <Slot />
    </>
  );
});
