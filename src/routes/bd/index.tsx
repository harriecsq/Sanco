import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ModulePlaceholder } from "~/components/shell/placeholder";

export default component$(() => (
  <ModulePlaceholder
    module="bd"
    title="Business Development"
    milestone="M7"
    items={[
      "Client and prospect requests and inquiries",
      "Quoted rates and follow-up dates",
      "Open → quoted → won/lost; won requests become bookings",
    ]}
  />
));

export const head: DocumentHead = { title: "Business Development · Sanco" };
