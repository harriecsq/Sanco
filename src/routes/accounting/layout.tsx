import { component$, Slot } from "@builder.io/qwik";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  // Booking users open DRs from their bookings.
  if (!can(session.user, "accounting") && !can(session.user, "bookings")) return <NoAccess />;
  const money = can(session.user, "accounting");
  return (
    <>
      <TabNav
        label="Accounting"
        tabs={[
          { href: "/accounting/", label: "Delivery receipts" },
          ...(money
            ? [
                { href: "/accounting/receivables/", label: "Receivables" },
                { href: "/accounting/payments/", label: "Payments" },
                { href: "/accounting/checks/", label: "Checks" },
              ]
            : []),
        ]}
      />
      <Slot />
    </>
  );
});
