import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="accounting"
    title="Accounting"
    milestone="M8"
    items={[
      "Delivery receipts: requested → approved → delivered",
      "Receivables with aging buckets",
      "Checks received, including PDCs",
      "Payments allocated to DRs",
    ]}
  />
));

export const head: DocumentHead = { title: "Accounting · Sanco" };
