import { component$, Slot } from "@builder.io/qwik";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess } from "~/components/ui/ui";
import { TabNav } from "~/components/ui/tabs";

export default component$(() => {
  const session = useSession();
  if (!can(session.user, "hr")) return <NoAccess />;
  return (
    <>
      <div class="print:hidden">
        <TabNav
          label="HR"
          tabs={[
            { href: "/hr/", label: "Attendance" },
            { href: "/hr/payroll/", label: "Payroll" },
            { href: "/hr/debts/", label: "Debts" },
            { href: "/hr/employees/", label: "Employees" },
          ]}
        />
      </div>
      <Slot />
    </>
  );
});
