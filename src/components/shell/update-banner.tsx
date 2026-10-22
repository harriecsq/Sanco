import { component$, useSignal, useVisibleTask$ } from "@builder.io/qwik";
import { commands, type UpdateStatus } from "~/lib/bindings";
import { useSession } from "~/lib/session";
import { errorText } from "~/lib/result";
import { btnPrimary } from "~/components/ui/styles";

/**
 * Top-of-app banner: a newer version runs on another PC (install it from
 * there), or other PCs still run an older one (they can't sync until
 * updated, if the database changed).
 */
export const UpdateBanner = component$(() => {
  const session = useSession();
  const status = useSignal<UpdateStatus | null>(null);
  const busy = useSignal(false);
  const error = useSignal("");

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async ({ track }) => {
      track(() => session.syncTick);
      track(() => session.user?.id);
      if (!session.user) return;
      const res = await commands.updateStatus();
      status.value = res.status === "ok" ? res.data : null;
    },
    { strategy: "document-ready" },
  );

  const s = status.value;
  if (!s) return null;
  const offer = s.offer;
  const blocked = s.older.filter((p) => p.blocked);

  if (offer) {
    return (
      <div
        role="status"
        class="flex flex-wrap items-center gap-3 border-b border-brand-200 bg-brand-50 px-6 py-2.5 text-sm text-brand-900"
      >
        <span class="flex-1">
          <strong>Sanco {offer.version}</strong> is available
          {offer.from_name ? ` on ${offer.from_name}` : ""}. This PC runs{" "}
          {s.current}.
          {error.value && (
            <span class="mt-1 block text-danger">{error.value}</span>
          )}
        </span>
        <button
          type="button"
          class={btnPrimary}
          disabled={busy.value}
          onClick$={async () => {
            busy.value = true;
            error.value = "";
            const res = await commands.installUpdate(offer.addr);
            // On success the app quits and the installer takes over.
            if (res.status === "error") {
              error.value = errorText(res.error);
              busy.value = false;
            }
          }}
        >
          {busy.value ? "Downloading…" : "Update now"}
        </button>
      </div>
    );
  }

  if (blocked.length > 0) {
    const names = blocked
      .map((p) => `${p.name ?? "a PC"} (${p.version})`)
      .join(", ");
    return (
      <div
        role="status"
        class="border-b border-amber-200 bg-amber-50 px-6 py-2.5 text-sm text-amber-900"
      >
        {names} {blocked.length === 1 ? "runs" : "run"} an older version and
        can't sync with this PC until updated.{" "}
        {s.can_share
          ? "Open Sanco there: it offers this version (Update now)."
          : "Install this version there with the setup program."}
      </div>
    );
  }
  return null;
});
