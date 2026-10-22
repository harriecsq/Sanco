import { $, component$, useSignal } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type PcSettings } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, PageHeader } from "~/components/ui/ui";
import { card, hint } from "~/components/ui/styles";

export default component$(() => {
  const session = useSession();
  const pc = useSignal<PcSettings | null>(null);
  const error = useSignal("");
  const load = $(async () => {
    try {
      pc.value = unwrap(await commands.pcSettings());
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad([], load);

  const s = session.status;
  const editable = can(session.user, "settings", "edit");
  const rows: [string, string][] = s
    ? [
        ["Workspace", s.workspace ?? "—"],
        ["PC name", s.pc_name],
        ["Sync address", s.addr ?? "Unknown: is Tailscale running?"],
        ["Sync port", String(s.port)],
        ["Data folder", s.data_dir],
        ["App version", s.app_version],
        ["Site id", s.site_id ?? "—"],
      ]
    : [];

  return (
    <>
      <PageHeader
        title="This PC"
        subtitle="Settings that apply only to this computer."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <div class="grid gap-4 lg:grid-cols-2">
        <section class={[card, "p-5"]}>
          <h2 class="mb-3 font-semibold">About</h2>
          <dl class="grid grid-cols-[8rem_1fr] gap-x-3 gap-y-1.5 text-sm">
            {rows.map(([k, v]) => (
              <div key={k} class="contents">
                <dt class="text-ink-muted">{k}</dt>
                <dd class="break-all">{v}</dd>
              </div>
            ))}
          </dl>
        </section>
        <section class={[card, "space-y-3 p-5"]}>
          <h2 class="font-semibold">Staying online</h2>
          <p class={hint}>
            Other PCs can only sync with this one while Sanco runs. Closing the
            window keeps it running in the tray; use the tray icon's Quit to
            stop it.
          </p>
          {pc.value && (
            <label class="flex items-start gap-2 text-sm">
              <input
                type="checkbox"
                class="mt-0.5 accent-brand-500"
                checked={pc.value.autostart}
                disabled={!editable || !pc.value.autostart_available}
                onChange$={async (_, el) => {
                  error.value = "";
                  try {
                    unwrap(await commands.setAutostart(el.checked));
                  } catch (e) {
                    error.value = errorText(e);
                  }
                  await load();
                }}
              />
              <span>
                Start Sanco in the tray when someone signs in to Windows
                {!pc.value.autostart_available && (
                  <span class={["block", hint]}>
                    Not available in test instances (--data-dir).
                  </span>
                )}
              </span>
            </label>
          )}
        </section>
      </div>
    </>
  );
});

export const head: DocumentHead = { title: "Settings · Sanco" };
