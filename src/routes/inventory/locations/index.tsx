import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type LocationInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Pager, usePage } from "~/components/ui/pager";
import {
  Alert,
  Badge,
  Confirm,
  Empty,
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
  table,
  td,
  th,
} from "~/components/ui/styles";

interface Form {
  id: string;
  name: string;
  notes: string;
  sort: string;
  active: boolean;
}

export default component$(() => {
  const session = useSession();
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>({ id: "", name: "", notes: "", sort: "", active: true });
  const open = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);
  const deleting = useSignal<LocationInfo | null>(null);
  const deleteOpen = useSignal(false);

  useLiveLoad(
    ["locations"],
    $(async () => {
      try {
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );
  const pg = usePage(locations);

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      const sort = Number(form.sort || 0);
      if (!Number.isInteger(sort)) throw new Error("the order must be a whole number");
      unwrap(
        await commands.saveLocation({
          id: form.id || null,
          name: form.name,
          notes: form.notes,
          sort,
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

  const canEdit = can(session.user, "inventory", "edit");
  const canDelete = can(session.user, "inventory", "delete");

  return (
    <>
      <PageHeader title="Locations" subtitle="Where stock is kept.">
        {can(session.user, "inventory", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              const next = Math.max(0, ...locations.value.map((l) => l.sort)) + 1;
              Object.assign(form, { id: "", name: "", notes: "", sort: String(next), active: true });
              formError.value = "";
              open.value = true;
            }}
          >
            New location
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <section class={[card, "max-w-3xl"]}>
        {locations.value.length === 0 ? (
          <Empty>No locations.</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Name</th>
                <th class={th}>Notes</th>
                <th class={[th, "text-right"]}>Order</th>
                <th class={th}>Status</th>
                <th class={th} />
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((l) => (
                <tr key={l.id} class={l.active ? "" : "text-ink-muted"}>
                  <td class={[td, "font-medium"]}>{l.name}</td>
                  <td class={td}>{l.notes ?? "—"}</td>
                  <td class={[td, "text-right"]}>{l.sort}</td>
                  <td class={td}>
                    <Badge tone={l.active ? "brand" : "muted"}>
                      {l.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                  <td class={[td, "whitespace-nowrap text-right"]}>
                    {canEdit && (
                      <button
                        class={btnGhost}
                        onClick$={() => {
                          Object.assign(form, {
                            id: l.id,
                            name: l.name,
                            notes: l.notes ?? "",
                            sort: String(l.sort),
                            active: l.active,
                          });
                          formError.value = "";
                          open.value = true;
                        }}
                      >
                        Edit
                      </button>
                    )}
                    {canDelete && (
                      <button
                        class={btnGhost}
                        onClick$={() => {
                          deleting.value = l;
                          deleteOpen.value = true;
                        }}
                      >
                        Delete
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>

      <Modal open={open} title={form.id ? "Edit location" : "New location"}>
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
          <Field label="Notes">
            <input
              class={input}
              value={form.notes}
              onInput$={(_, el) => (form.notes = el.value)}
            />
          </Field>
          <Field label="Order" hint="Lower numbers come first in lists.">
            <input
              class={input}
              inputMode="numeric"
              value={form.sort}
              onInput$={(_, el) => (form.sort = el.value)}
            />
          </Field>
          {form.id && (
            <label class="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                class="accent-brand-500"
                checked={form.active}
                onChange$={(_, el) => (form.active = el.checked)}
              />
              Active
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
        title={`Delete ${deleting.value?.name ?? "location"}?`}
        onConfirm$={$(async () => {
          if (deleting.value) unwrap(await commands.deleteLocation(deleting.value.id));
        })}
      >
        <p>
          This deletes the location on every PC and can't be undone.{" "}
          <span class={hint}>
            A location with stock movements can't be deleted; mark it inactive instead.
          </span>
        </p>
      </Confirm>
    </>
  );
});

export const head: DocumentHead = { title: "Locations · Sanco" };
