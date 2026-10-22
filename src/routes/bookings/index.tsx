import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="bookings"
    title="Booking"
    milestone="M7"
    items={[
      "Confirmed client orders with rates from the client's pricing",
      "Draft → confirmed → closed/cancelled",
      "DR requests from bookings",
    ]}
  />
));

export const head: DocumentHead = { title: "Booking · Sanco" };
