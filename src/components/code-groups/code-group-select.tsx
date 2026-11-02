import { component$, useSignal, useVisibleTask$, type QRL } from "@builder.io/qwik";
import { commands, type CodeGroupInfo, type Kind } from "~/lib/bindings";
import { unwrap } from "~/lib/result";
import { Field } from "~/components/ui/ui";
import { input } from "~/components/ui/styles";

/**
 * Picks the code group a client or supplier is numbered in. Retired groups
 * are only listed while they are the current value. A new record (empty
 * `value`) gets the first active group.
 */
export const CodeGroupSelect = component$<{
  kind: Kind;
  value: string;
  onChange$: QRL<(key: string) => void>;
}>(({ kind, value, onChange$ }) => {
  const groups = useSignal<CodeGroupInfo[]>([]);

  // eslint-disable-next-line qwik/no-use-visible-task
  useVisibleTask$(
    async () => {
      try {
        groups.value = unwrap(await commands.listCodeGroups(kind));
      } catch {
        groups.value = [];
      }
      const first = groups.value.find((g) => g.group.active);
      if (!value && first) await onChange$(first.group.key);
    },
    { strategy: "document-ready" },
  );

  const shown = groups.value.filter((g) => g.group.active || g.group.key === value);
  return (
    <Field
      label="Code group"
      hint={
        groups.value.length === 0
          ? "No code groups yet: an admin adds them under Settings → Number formats."
          : value
            ? "Changing the group gives it a new code."
            : "The code is given automatically when you save."
      }
    >
      <select class={input} onChange$={(_, el) => onChange$(el.value)}>
        {!value && <option value="">Choose…</option>}
        {shown.map((g) => (
          <option key={g.group.key} value={g.group.key} selected={g.group.key === value}>
            {g.group.active ? g.group.prefix : `${g.group.prefix} (retired)`}
          </option>
        ))}
      </select>
    </Field>
  );
});
