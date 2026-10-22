import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="orders"
    title="Purchase orders"
    milestone="M9"
    items={[
      "Purchase orders and TT payments",
      "Receiving into stock with expiry dates",
    ]}
  />
));

export const head: DocumentHead = { title: "Order · Sanco" };
