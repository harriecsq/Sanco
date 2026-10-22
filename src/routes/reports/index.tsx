import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="reports"
    title="Reports"
    milestone="M10"
    items={[
      "Sales, receivables aging, collections and checks",
      "Stock, expiring stock, PO spend, BD conversion, item margin",
      "CSV export",
    ]}
  />
));

export const head: DocumentHead = { title: "Reports · Sanco" };
