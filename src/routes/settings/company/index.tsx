import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type Company } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, Field, PageHeader } from "~/components/ui/ui";
import { btnPrimary, card, hint, input } from "~/components/ui/styles";

const FIELDS: { key: keyof Company; label: string; multiline?: boolean }[] = [
  { key: "name", label: "Company name" },
  { key: "address", label: "Address", multiline: true },
  { key: "tin", label: "TIN" },
  { key: "phone", label: "Phone" },
  { key: "email", label: "Email" },
];

export default component$(() => {
  const session = useSession();
  const form = useStore<Company>({ name: "", address: "", tin: "", phone: "", email: "" });
  const dirty = useSignal(false);
  const error = useSignal("");
  const saved = useSignal(false);
  const busy = useSignal(false);

  useLiveLoad(
    ["settings"],
    $(async () => {
      try {
        const c = unwrap(await commands.companyInfo());
        if (!dirty.value) Object.assign(form, c);
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const editable = can(session.user, "settings", "edit");
  return (
    <>
      <PageHeader title="Company" subtitle="Sanco's details, shared by every PC." />
      <form
        class={[card, "max-w-xl space-y-3 p-5"]}
        preventdefault:submit
        onSubmit$={async () => {
          busy.value = true;
          error.value = "";
          try {
            unwrap(await commands.setCompanyInfo({ ...form }));
            dirty.value = false;
            saved.value = true;
          } catch (e) {
            error.value = errorText(e);
          } finally {
            busy.value = false;
          }
        }}
      >
        {error.value && <Alert>{error.value}</Alert>}
        <fieldset disabled={!editable} class="space-y-3">
          {FIELDS.map((f) =>
            f.multiline ? (
              <Field key={f.key} label={f.label}>
                <textarea
                  class={input}
                  rows={3}
                  value={form[f.key]}
                  onInput$={(_, el) => {
                    form[f.key] = el.value;
                    dirty.value = true;
                    saved.value = false;
                  }}
                />
              </Field>
            ) : (
              <Field key={f.key} label={f.label}>
                <input
                  class={input}
                  value={form[f.key]}
                  onInput$={(_, el) => {
                    form[f.key] = el.value;
                    dirty.value = true;
                    saved.value = false;
                  }}
                />
              </Field>
            ),
          )}
        </fieldset>
        {editable && (
          <div class="flex items-center justify-end gap-2">
            {saved.value && <span class={hint}>Saved.</span>}
            <button class={btnPrimary} disabled={busy.value || !dirty.value}>
              Save
            </button>
          </div>
        )}
      </form>
    </>
  );
});

export const head: DocumentHead = { title: "Company · Sanco" };
