import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="inventory"
    title="Inventory"
    milestone="M6"
    items={[
      "Stock on hand by item, location and expiry",
      "Receipts, adjustments and transfers",
      "Near-expiry and low-stock lists",
    ]}
  />
));

export const head: DocumentHead = { title: "Inventory · Sanco" };
