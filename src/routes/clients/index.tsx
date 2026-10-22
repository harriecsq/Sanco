import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="clients"
    title="Clients"
    milestone="M5"
    items={[
      "Client list with terms, margin tier and credit limit",
      "Each client's rates, bookings, DRs, balance and checks",
    ]}
  />
));

export const head: DocumentHead = { title: "Clients · Sanco" };
