import { component$, useSignal } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands } from "~/lib/bindings";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, PageHeader } from "~/components/ui/ui";
import { btnPrimary, card, hint, input, label } from "~/components/ui/styles";

export default component$(() => {
  const session = useSession();
  const current = useSignal("");
  const next = useSignal("");
  const repeat = useSignal("");
  const error = useSignal("");
  const done = useSignal(false);
  const busy = useSignal(false);
  const u = session.user;

  return (
    <>
      <PageHeader
        title="My account"
        subtitle={u ? `${u.display_name} · ${u.username}` : ""}
      />
      <div class="grid gap-4 lg:grid-cols-2">
        <form
          class={[card, "space-y-3 p-5"]}
          preventdefault:submit
          onSubmit$={async () => {
            error.value = "";
            done.value = false;
            if (next.value !== repeat.value) {
              error.value = "The new passwords don't match.";
              return;
            }
            busy.value = true;
            try {
              unwrap(await commands.changePassword(current.value, next.value));
              current.value = next.value = repeat.value = "";
              done.value = true;
            } catch (e) {
              error.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          <h2 class="font-semibold">Change password</h2>
          {error.value && <Alert>{error.value}</Alert>}
          {done.value && (
            <Alert tone="brand">
              Password changed. It works on every PC after the next sync.
            </Alert>
          )}
          <label class="block space-y-1">
            <span class={label}>Current password</span>
            <input
              class={input}
              type="password"
              autoComplete="current-password"
              required
              bind:value={current}
            />
          </label>
          <label class="block space-y-1">
            <span class={label}>New password</span>
            <input
              class={input}
              type="password"
              autoComplete="new-password"
              required
              bind:value={next}
            />
            <span class={hint}>At least 6 characters.</span>
          </label>
          <label class="block space-y-1">
            <span class={label}>Repeat new password</span>
            <input
              class={input}
              type="password"
              autoComplete="new-password"
              required
              bind:value={repeat}
            />
          </label>
          <button class={btnPrimary} disabled={busy.value}>
            Change password
          </button>
        </form>
        <section class={[card, "space-y-2 p-5 text-sm"]}>
          <h2 class="font-semibold">Roles</h2>
          <p>{u?.roles.join(", ") || "No role yet. Ask an admin."}</p>
          <p class={hint}>
            Roles decide which modules you can open and change. Admins edit them
            under Settings → Roles.
          </p>
        </section>
      </div>
    </>
  );
});

export const head: DocumentHead = { title: "My account · Sanco" };
