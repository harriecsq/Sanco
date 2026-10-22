import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="items"
    title="Items"
    milestone="M5"
    items={[
      "Item catalog",
      "Cost sheet and landed cost",
      "Default margins and per-client rates",
    ]}
  />
));

export const head: DocumentHead = { title: "Items · Sanco" };
