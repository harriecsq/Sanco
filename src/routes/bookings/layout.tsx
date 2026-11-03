import { component$, Slot } from "@builder.io/qwik";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

// Booking is the DR and invoice screen: every client order starts here.
// Accounting users may open the same screens.
export default component$(() => {
  const session = useSession();
  if (!can(session.user, "bookings") && !can(session.user, "accounting")) return <NoAccess />;
  return (
    <>
      <TabNav
        label="Booking"
        tabs={[
          { href: "/bookings/", label: "Delivery receipts" },
          { href: "/bookings/invoices/", label: "Invoices" },
        ]}
      />
      <Slot />
    </>
  );
});
