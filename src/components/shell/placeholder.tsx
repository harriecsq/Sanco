import { component$ } from "@builder.io/qwik";
import type { Module } from "~/lib/bindings";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess, PageHeader } from "~/components/ui/ui";

/** A module that isn't built yet: says what it will hold and when. */
export const ModulePlaceholder = component$<{
  module: Module;
  title: string;
  milestone: string;
  items: string[];
}>(({ module, title, milestone, items }) => {
  const session = useSession();
  if (!can(session.user, module)) return <NoAccess />;
  return (
    <>
      <PageHeader title={title} subtitle={`Coming in ${milestone}`} />
      <div class="rounded-lg border border-dashed border-brand-200 bg-white p-6">
        <p class="text-sm text-ink-muted">This module will cover:</p>
        <ul class="mt-2 list-inside list-disc space-y-1 text-sm">
          {items.map((i) => (
            <li key={i}>{i}</li>
          ))}
        </ul>
      </div>
    </>
  );
});
