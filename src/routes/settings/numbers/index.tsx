import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type NumberFormat, type SeriesFormat } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, Field, Modal, PageHeader } from "~/components/ui/ui";
import {
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  table,
  td,
  th,
} from "~/components/ui/styles";

/** Mirrors the backend's rendering, for the live example. */
function render(f: NumberFormat) {
  const parts: string[] = [];
  if (f.prefix.trim()) parts.push(f.prefix.trim().toUpperCase());
  if (f.year) parts.push(String(new Date().getFullYear()));
  parts.push("1".padStart(Math.max(1, Math.min(f.pad, 8)), "0"));
  return parts.join("-");
}

export default component$(() => {
  const session = useSession();
  const formats = useSignal<SeriesFormat[]>([]);
  const error = useSignal("");
  const editing = useStore({ series: "", label: "", manual_allowed: false, manual: false, typed: false });
  const form = useStore<NumberFormat>({ prefix: "", year: true, reset_yearly: true, pad: 4 });
  const open = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["settings", "doc_numbers"],
    $(async () => {
      try {
        formats.value = unwrap(await commands.numberFormats());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      if (editing.manual_allowed && editing.typed !== editing.manual) {
        unwrap(await commands.setNumberMode(editing.series, editing.typed));
      }
      unwrap(await commands.setNumberFormat(editing.series, { ...form }));
      open.value = false;
    } catch (e) {
      formError.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  const editable = can(session.user, "settings", "edit");

  return (
    <>
      <PageHeader
        title="Number formats"
        subtitle="How new document numbers and codes look. Numbers already given keep their text."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <section class={[card, "max-w-4xl"]}>
        <table class={table}>
          <thead>
            <tr>
              <th class={th}>Series</th>
              <th class={th}>Numbers</th>
              <th class={th}>Next looks like</th>
              <th class={th}>Prefix</th>
              <th class={th}>Year</th>
              <th class={th}>Restarts yearly</th>
              <th class={th}>Digits</th>
              {editable && <th class={th} />}
            </tr>
          </thead>
          <tbody>
            {formats.value.map((f) => (
              <tr key={f.series}>
                <td class={[td, "font-medium"]}>{f.label}</td>
                <td class={td}>{f.manual ? "Typed from the booklet" : "Automatic"}</td>
                <td class={[td, "font-mono"]}>{f.manual ? "—" : f.example}</td>
                <td class={td}>{f.format.prefix || "—"}</td>
                <td class={td}>{f.format.year ? "Shown" : "—"}</td>
                <td class={td}>{f.format.reset_yearly ? "Yes" : "No"}</td>
                <td class={td}>{f.format.pad}</td>
                {editable && (
                  <td class={[td, "text-right"]}>
                    <button
                      class={btnGhost}
                      onClick$={() => {
                        editing.series = f.series;
                        editing.label = f.label;
                        editing.manual_allowed = f.manual_allowed;
                        editing.manual = f.manual;
                        editing.typed = f.manual;
                        Object.assign(form, f.format);
                        formError.value = "";
                        open.value = true;
                      }}
                    >
                      Edit
                    </button>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <p class={["mt-3 max-w-4xl", hint]}>
        "Next looks like" shows the format with sequence 1; the real next number continues from
        the last one given. The yearly restart can't change once a series has numbers.
      </p>

      <Modal open={open} title={`Format: ${editing.label}`}>
        <form class="space-y-3" preventdefault:submit onSubmit$={save}>
          {formError.value && <Alert>{formError.value}</Alert>}
          {editing.manual_allowed && (
            <fieldset class="space-y-1 text-sm">
              <legend class="mb-1 font-medium">Numbers</legend>
              <label class="flex items-center gap-2">
                <input
                  type="radio"
                  name="number-mode"
                  class="accent-brand-500"
                  checked={editing.typed}
                  onChange$={() => (editing.typed = true)}
                />
                Typed by staff from the BIR booklet
              </label>
              <label class="flex items-center gap-2">
                <input
                  type="radio"
                  name="number-mode"
                  class="accent-brand-500"
                  checked={!editing.typed}
                  onChange$={() => (editing.typed = false)}
                />
                Given automatically in the format below
              </label>
              <p class={hint}>Numbers already given keep their text when this changes.</p>
            </fieldset>
          )}
          <Field label="Prefix" hint="Letters and digits only, e.g. DR.">
            <input
              class={input}
              value={form.prefix}
              onInput$={(_, el) => (form.prefix = el.value)}
            />
          </Field>
          <label class="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              class="accent-brand-500"
              checked={form.year}
              onChange$={(_, el) => {
                form.year = el.checked;
                if (!el.checked) form.reset_yearly = false;
              }}
            />
            Show the year
          </label>
          <label class="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              class="accent-brand-500"
              checked={form.reset_yearly}
              disabled={!form.year}
              onChange$={(_, el) => (form.reset_yearly = el.checked)}
            />
            Restart at 1 each year
          </label>
          <Field label="Digits">
            <input
              class={input}
              type="number"
              min={1}
              max={8}
              value={form.pad}
              onInput$={(_, el) => (form.pad = Number(el.value) || 1)}
            />
          </Field>
          <p class="text-sm">
            Example: <span class="font-mono font-medium">{render(form)}</span>
          </p>
          <div class="flex justify-end gap-2 pt-2">
            <button type="button" class={btnSecondary} onClick$={() => (open.value = false)}>
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Save
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

export const head: DocumentHead = { title: "Number formats · Sanco" };
