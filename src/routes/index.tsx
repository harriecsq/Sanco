import { $, component$, useSignal } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands } from "~/lib/bindings";

export default component$(() => {
  const name = useSignal("Sanco");
  const reply = useSignal("");

  const greet = $(async () => {
    reply.value = await commands.greet(name.value);
  });

  return (
    <main class="mx-auto max-w-xl p-8">
      <div class="mb-8 inline-flex items-center gap-3 rounded-full bg-brand-500 px-6 py-3 text-white">
        <span class="grid size-9 place-items-center border-2 border-white bg-white text-xl font-semibold text-brand-500">
          S
        </span>
        <span class="leading-tight">
          <span class="block text-2xl">Sanco</span>
          <span class="block text-sm font-semibold">Medical Supply</span>
        </span>
      </div>

      <p class="mb-4 text-ink-muted">M0 check: typed Tauri command round trip.</p>
      <div class="flex gap-2">
        <input
          class="flex-1 rounded-md border border-line px-3 py-2"
          bind:value={name}
        />
        <button
          class="rounded-md bg-brand-500 px-4 py-2 font-medium text-white hover:bg-brand-600"
          onClick$={greet}
        >
          Greet
        </button>
      </div>
      {reply.value && (
        <p class="mt-4 rounded-md bg-brand-50 px-3 py-2 text-brand-800">
          {reply.value}
        </p>
      )}
    </main>
  );
});

export const head: DocumentHead = {
  title: "Sanco",
};
