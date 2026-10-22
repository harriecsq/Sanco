import { $, component$, useSignal } from "@builder.io/qwik";
import { Link, type DocumentHead } from "@builder.io/qwik-city";
import { commands } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
import { useSession } from "~/lib/session";
import { NoAccess, PageHeader } from "~/components/ui/ui";
import { card } from "~/components/ui/styles";

// M4 dashboard: greeting and sync alerts. KPIs and to-dos come in M10.

export default component$(() => {
  const session = useSession();
  const conflicts = useSignal<number | null>(null);
  const settings = can(session.user, "settings");

  const load = $(async () => {
    if (!can(session.user, "settings")) return;
    const res = await commands.listConflicts(false);
    conflicts.value = res.status === "ok" ? res.data.length : null;
  });
  useLiveLoad(["conflicts"], load);

  if (!can(session.user, "dashboard")) return <NoAccess />;
  const s = session.status;
  const hour = Number(
    new Intl.DateTimeFormat("en-PH", {
      timeZone: "Asia/Manila",
      hour: "numeric",
      hourCycle: "h23",
    }).format(new Date()),
  );
  const greeting =
    hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";

  return (
    <>
      <PageHeader
        title={`${greeting}, ${session.user?.display_name ?? ""}`}
        subtitle={`${s?.workspace ?? ""} · ${s?.pc_name ?? ""}`}
      />
      <div class="grid gap-4 md:grid-cols-3">
        {settings && (
          <Link
            href="/settings/conflicts/"
            class={[card, "block p-4 hover:border-brand-300"]}
          >
            <p class="text-xs font-semibold uppercase tracking-wide text-ink-muted">
              Sync conflicts
            </p>
            <p
              class={[
                "mt-1 text-2xl font-semibold",
                conflicts.value ? "text-warning" : "text-ink",
              ]}
            >
              {conflicts.value ?? "—"}
            </p>
            <p class="text-xs text-ink-muted">
              {conflicts.value
                ? "Edits made on two PCs at once. Review them."
                : "Nothing to review."}
            </p>
          </Link>
        )}
        <div class={[card, "p-4 md:col-span-2"]}>
          <p class="text-xs font-semibold uppercase tracking-wide text-ink-muted">
            Coming next
          </p>
          <p class="mt-1 text-sm">
            KPIs, alerts and to-dos (open requests, DRs to deliver, receivables,
            PDCs, low and expiring stock, POs in transit) arrive with the
            modules that feed them.
          </p>
        </div>
      </div>
    </>
  );
});

export const head: DocumentHead = {
  title: "Dashboard · Sanco",
};
