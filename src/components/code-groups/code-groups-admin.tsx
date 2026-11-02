import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import { commands, type CodeGroupInfo, type Kind } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { errorText, unwrap } from "~/lib/result";
import { Alert, Badge, Field, Modal } from "~/components/ui/ui";
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

interface Form {
  isNew: boolean;
  key: string;
  prefix: string;
  pad: number;
  active: boolean;
}

/** Same rendering as the backend, for the live example. */
const example = (f: Form) =>
  `${f.prefix.trim().toUpperCase()}-${"1".padStart(Math.max(1, Math.min(f.pad, 8)), "0")}`;

const Groups = component$<{ kind: Kind; title: string; sample: string; editable: boolean }>(
  ({ kind, title, sample, editable }) => {
    const groups = useSignal<CodeGroupInfo[]>([]);
    const error = useSignal("");
    const form = useStore<Form>({ isNew: true, key: "", prefix: "", pad: 4, active: true });
    const open = useSignal(false);
    const formError = useSignal("");
    const busy = useSignal(false);

    useLiveLoad(
      ["settings", "doc_numbers"],
      $(async () => {
        try {
          groups.value = unwrap(await commands.listCodeGroups(kind));
        } catch (e) {
          error.value = errorText(e);
        }
      }),
    );

    const save = $(async () => {
      busy.value = true;
      formError.value = "";
      try {
        unwrap(
          await commands.saveCodeGroup(kind, {
            key: form.key,
            prefix: form.prefix,
            pad: form.pad,
            active: form.active,
          }),
        );
        open.value = false;
      } catch (e) {
        formError.value = errorText(e);
      } finally {
        busy.value = false;
      }
    });

    return (
      <section class={[card, "mt-6 max-w-4xl"]}>
        <div class="mb-2 flex items-center justify-between">
          <h2 class="font-semibold">{title}</h2>
          {editable && (
            <button
              class={btnSecondary}
              onClick$={() => {
                Object.assign(form, { isNew: true, key: "", prefix: "", pad: 4, active: true });
                formError.value = "";
                open.value = true;
              }}
            >
              Add group
            </button>
          )}
        </div>
        {error.value && <Alert>{error.value}</Alert>}
        {groups.value.length === 0 ? (
          <p class={hint}>No groups yet. Add one before adding records, e.g. {sample}.</p>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Prefix</th>
                <th class={th}>Key</th>
                <th class={th}>Next looks like</th>
                <th class={th}>Digits</th>
                <th class={[th, "text-right"]}>In use</th>
                <th class={th}>Status</th>
                {editable && <th class={th} />}
              </tr>
            </thead>
            <tbody>
              {groups.value.map((g) => (
                <tr key={g.group.key} class={g.group.active ? "" : "text-ink-muted"}>
                  <td class={[td, "font-medium"]}>{g.group.prefix}</td>
                  <td class={td}>{g.group.key}</td>
                  <td class={[td, "font-mono"]}>{g.example}</td>
                  <td class={td}>{g.group.pad}</td>
                  <td class={[td, "text-right"]}>{g.used}</td>
                  <td class={td}>
                    <Badge tone={g.group.active ? "brand" : "muted"}>
                      {g.group.active ? "Active" : "Retired"}
                    </Badge>
                  </td>
                  {editable && (
                    <td class={[td, "text-right"]}>
                      <button
                        class={btnGhost}
                        onClick$={() => {
                          Object.assign(form, { isNew: false, ...g.group });
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
        )}

        <Modal open={open} title={form.isNew ? "New code group" : `Code group ${form.key}`}>
          <form class="space-y-3" preventdefault:submit onSubmit$={save}>
            {formError.value && <Alert>{formError.value}</Alert>}
            {form.isNew && (
              <Field label="Key" hint="1–8 letters or digits. Can't change later.">
                <input
                  class={input}
                  required
                  value={form.key}
                  onInput$={(_, el) => (form.key = el.value)}
                />
              </Field>
            )}
            <Field label="Prefix" hint="Letters, digits and dashes, e.g. NCR or SUP-TWN.">
              <input
                class={input}
                required
                value={form.prefix}
                onInput$={(_, el) => (form.prefix = el.value)}
              />
            </Field>
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
            {!form.isNew && (
              <label class="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  class="accent-brand-500"
                  checked={form.active}
                  onChange$={(_, el) => (form.active = el.checked)}
                />
                Active (retired groups give out no new codes)
              </label>
            )}
            <p class="text-sm">
              Example: <span class="font-mono font-medium">{example(form)}</span>
            </p>
            <p class={hint}>Codes already given keep their text.</p>
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
      </section>
    );
  },
);

export const CodeGroupsAdmin = component$<{ editable: boolean }>(({ editable }) => (
  <>
    <Groups kind="client" title="Client code groups" sample="NCR, REG" editable={editable} />
    <Groups kind="supplier" title="Supplier code groups" sample="SUP-TWN, SUP-PHP" editable={editable} />
  </>
));
