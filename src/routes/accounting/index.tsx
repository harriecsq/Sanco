import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ReceiptsPage } from "~/components/sales/receipts";

export default component$(() => <ReceiptsPage kind="dr" />);

export const head: DocumentHead = { title: "Delivery receipts · Sanco" };
