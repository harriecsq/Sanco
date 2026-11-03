import { component$, Slot } from "@builder.io/qwik";
import { useLocation } from "@builder.io/qwik-city";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  if (!can(session.user, "accounting")) return <NoAccess />;
  const path = loc.url.pathname;
  return (
    <>
      <TabNav
        label="Accounting"
        tabs={[
          {
            href: "/accounting/receivables/",
            label: "Receivables",
            active: path === "/accounting/" || path.startsWith("/accounting/receivables/"),
          },
          { href: "/accounting/payments/", label: "Payments" },
          { href: "/accounting/checks/", label: "Checks" },
          { href: "/accounting/bank-accounts/", label: "Bank accounts" },
        ]}
      />
      <Slot />
    </>
  );
});
