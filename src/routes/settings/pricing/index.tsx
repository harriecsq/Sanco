import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type ChargeMode, type ChargeTypeInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { parsePct, pctText } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import {
  Alert,
  Badge,
  Confirm,
  Field,
  Modal,
  PageHeader,
} from "~/components/ui/ui";
import {
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  inputBase,
  table,
  td,
  th,
} from "~/components/ui/styles";

const MODES: { value: ChargeMode; label: string }[] = [
  { value: "per_unit", label: "₱ per unit" },
  { value: "pct", label: "% of base cost" },
];

interface Form {
  id: string;
  name: string;
  mode: ChargeMode;
  sort: number;
  active: boolean;
}

export default component$(() => {
  const session = useSession();
  const vatBp = useSignal<number | null>(null);
  const vatText = useSignal("");
  const vatMsg = useSignal("");
  const types = useSignal<ChargeTypeInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>({ id: "", name: "", mode: "per_unit", sort: 0, active: true });
  const open = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);
  const deleting = useSignal<ChargeTypeInfo | null>(null);
  const deleteOpen = useSignal(false);

  useLiveLoad(
    ["settings", "charge_types", "item_cost_charges"],
    $(async () => {
      try {
        const p = unwrap(await commands.pricingSettings());
        if (vatBp.value === null || vatText.value === pctText(vatBp.value)) {
          vatText.value = pctText(p.vat_bp);
        }
        vatBp.value = p.vat_bp;
        types.value = unwrap(await commands.listChargeTypes());
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
        await commands.saveChargeType({
          id: form.id || null,
          name: form.name,
          mode: form.mode,
          sort: form.sort,
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

  const editable = can(session.user, "settings", "edit");

  return (
    <>
      <PageHeader title="Pricing" subtitle="VAT and the charges that make up landed cost." />
      {error.value && <Alert>{error.value}</Alert>}
      <div class="grid gap-4 lg:grid-cols-[20rem_minmax(0,1fr)]">
        <form
          class={[card, "space-y-3 self-start p-5"]}
          preventdefault:submit
          onSubmit$={async () => {
            vatMsg.value = "";
            try {
              const bp = parsePct(vatText.value, "VAT rate");
              if (bp == null) throw new Error("enter a VAT rate");
              unwrap(await commands.setVatRate(bp));
              vatMsg.value = "Saved.";
            } catch (e) {
              vatMsg.value = errorText(e);
            }
          }}
        >
          <h2 class="font-semibold">VAT</h2>
          <Field
            label="VAT rate (%)"
            hint="Used wherever VAT is ticked: gross = net × (1 + VAT)."
          >
            <input
              class={input}
              inputMode="decimal"
              disabled={!editable}
              value={vatText.value}
              onInput$={(_, el) => (vatText.value = el.value)}
            />
          </Field>
          {editable && (
            <div class="flex items-center justify-end gap-2">
              {vatMsg.value && <span class={hint}>{vatMsg.value}</span>}
              <button
                class={btnPrimary}
                disabled={vatText.value === pctText(vatBp.value)}
              >
                Save
              </button>
            </div>
          )}
        </form>

        <section class={card}>
          <div class="flex items-center justify-between border-b border-line px-4 py-3">
            <div>
              <h2 class="font-semibold">Cost-sheet charges</h2>
              <p class={hint}>
                Lines on every item's cost sheet. The type is the default; each item can change
                it.
              </p>
            </div>
            {editable && (
              <button
                class={btnSecondary}
                onClick$={() => {
                  const next = Math.max(0, ...types.value.map((t) => t.sort)) + 1;
                  Object.assign(form, { id: "", name: "", mode: "per_unit", sort: next, active: true });
                  formError.value = "";
                  open.value = true;
                }}
              >
                Add charge
              </button>
            )}
          </div>
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Charge</th>
                <th class={th}>Default type</th>
                <th class={[th, "text-right"]}>Used on</th>
                <th class={th}>Status</th>
                {editable && <th class={th} />}
              </tr>
            </thead>
            <tbody>
              {types.value.map((t) => (
                <tr key={t.id} class={t.active ? "" : "text-ink-muted"}>
                  <td class={[td, "font-medium"]}>{t.name}</td>
                  <td class={td}>{MODES.find((m) => m.value === t.mode)?.label}</td>
                  <td class={[td, "text-right"]}>
                    {t.used_by ? `${t.used_by} item${t.used_by > 1 ? "s" : ""}` : "—"}
                  </td>
                  <td class={td}>
                    <Badge tone={t.active ? "brand" : "muted"}>
                      {t.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                  {editable && (
                    <td class={[td, "whitespace-nowrap text-right"]}>
                      <button
                        class={btnGhost}
                        onClick$={() => {
                          Object.assign(form, {
                            id: t.id,
                            name: t.name,
                            mode: t.mode,
                            sort: t.sort,
                            active: t.active,
                          });
                          formError.value = "";
                          open.value = true;
                        }}
                      >
                        Edit
                      </button>
                      <button
                        class={btnGhost}
                        disabled={t.used_by > 0}
                        title={t.used_by > 0 ? "In use: deactivate it instead" : undefined}
                        onClick$={() => {
                          deleting.value = t;
                          deleteOpen.value = true;
                        }}
                      >
                        Delete
                      </button>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>

      <Modal open={open} title={form.id ? "Edit charge" : "New charge"}>
        <form class="space-y-3" preventdefault:submit onSubmit$={save}>
          {formError.value && <Alert>{formError.value}</Alert>}
          <Field label="Name">
            <input
              class={input}
              required
              value={form.name}
              onInput$={(_, el) => (form.name = el.value)}
            />
          </Field>
          <Field label="Default type">
            <select
              class={[inputBase, "w-full"]}
              onChange$={(_, el) => (form.mode = el.value as ChargeMode)}
            >
              {MODES.map((m) => (
                <option key={m.value} value={m.value} selected={m.value === form.mode}>
                  {m.label}
                </option>
              ))}
            </select>
          </Field>
          {form.id && (
            <label class="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                class="accent-brand-500"
                checked={form.active}
                onChange$={(_, el) => (form.active = el.checked)}
              />
              Active (inactive charges stay on sheets that use them)
            </label>
          )}
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

      <Confirm
        open={deleteOpen}
        title={`Delete ${deleting.value?.name ?? "charge"}?`}
        onConfirm$={$(async () => {
          if (deleting.value) unwrap(await commands.deleteChargeType(deleting.value.id));
        })}
      >
        <p>This removes the charge from every PC. It can't be undone.</p>
      </Confirm>
    </>
  );
});

export const head: DocumentHead = { title: "Pricing · Sanco" };
