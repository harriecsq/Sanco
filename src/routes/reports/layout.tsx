import { component$, Slot } from "@builder.io/qwik";
import { can, sees } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  const user = session.user;
  if (!can(user, "reports")) return <NoAccess />;
  return (
    <>
      <TabNav
        label="Reports"
        tabs={[
          { href: "/reports/", label: "Sales" },
          { href: "/reports/receivables/", label: "Receivables aging" },
          { href: "/reports/collections/", label: "Collections & checks" },
          { href: "/reports/stock/", label: "Stock" },
          ...(sees(user, "costs") ? [{ href: "/reports/spend/", label: "PO spend" }] : []),
          { href: "/reports/bd/", label: "BD conversion" },
          ...(sees(user, "costs") && sees(user, "margins")
            ? [{ href: "/reports/margin/", label: "Item margin" }]
            : []),
        ]}
      />
      <Slot />
    </>
  );
});
