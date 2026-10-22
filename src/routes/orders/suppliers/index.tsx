import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type SupplierInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { CURRENCIES } from "~/components/items/cost-sheet";
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

const COUNTRIES = [
  { value: "TW", label: "Taiwan" },
  { value: "CN", label: "China" },
  { value: "OTHER", label: "Other" },
];
const countryLabel = (c: string) => COUNTRIES.find((x) => x.value === c)?.label ?? c;

interface Form {
  id: string;
  name: string;
  country: string;
  currency: string;
  contact_person: string;
  phone: string;
  email: string;
  address: string;
  notes: string;
  active: boolean;
}

const blank = (): Form => ({
  id: "",
  name: "",
  country: "TW",
  currency: "USD",
  contact_person: "",
  phone: "",
  email: "",
  address: "",
  notes: "",
  active: true,
});

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const suppliers = useSignal<SupplierInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const open = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);
  const deleting = useSignal<SupplierInfo | null>(null);
  const deleteOpen = useSignal(false);

  useLiveLoad(
    ["suppliers", "doc_numbers", "items"],
    $(async () => {
      try {
        suppliers.value = unwrap(await commands.listSuppliers());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "country", "status"]);
    const q = f.q.toLowerCase();
    return suppliers.value.filter(
      (s) =>
        (!q || [s.code, s.name, s.contact_person, s.email].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.country || s.country === f.country) &&
        (!f.status || (f.status === "active") === s.active),
    );
  });

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      unwrap(
        await commands.saveSupplier({
          id: form.id || null,
          name: form.name,
          country: form.country,
          currency: form.currency,
          contact_person: form.contact_person,
          phone: form.phone,
          email: form.email,
          address: form.address,
          notes: form.notes,
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

  const canEdit = can(session.user, "orders", "edit");
  const canDelete = can(session.user, "orders", "delete");

  return (
    <>
      <PageHeader title="Suppliers" subtitle="Manufacturers and traders Sanco imports from.">
        {can(session.user, "orders", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blank());
              formError.value = "";
              open.value = true;
            }}
          >
            New supplier
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, contact"
        selects={[
          { key: "country", label: "Countries", options: COUNTRIES },
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "active", label: "Active" },
              { value: "inactive", label: "Inactive" },
            ],
          },
        ]}
      />
      <section class={card}>
        {shown.value.length === 0 ? (
          <Empty>{suppliers.value.length ? "No suppliers match." : "No suppliers yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Code</th>
                <th class={th}>Name</th>
                <th class={th}>Country</th>
                <th class={th}>Currency</th>
                <th class={th}>Contact</th>
                <th class={[th, "text-right"]}>Items</th>
                <th class={th}>Status</th>
                <th class={th} />
              </tr>
            </thead>
            <tbody>
              {shown.value.map((s) => (
                <tr key={s.id} class={s.active ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    {s.code ?? "—"}
                    {!s.code_confirmed && (
                      <span class={["ml-1", hint]} title="Given offline: may change after sync">
                        *
                      </span>
                    )}
                  </td>
                  <td class={[td, "font-medium"]}>
                    {s.name}
                    {s.duplicate_name && (
                      <span class="ml-2">
                        <Badge tone="warning">duplicate name</Badge>
                      </span>
                    )}
                    {s.notes && <div class={hint}>{s.notes}</div>}
                  </td>
                  <td class={td}>{countryLabel(s.country)}</td>
                  <td class={td}>{s.currency}</td>
                  <td class={td}>
                    {[s.contact_person, s.phone, s.email].filter(Boolean).join(" · ") || "—"}
                  </td>
                  <td class={[td, "text-right"]}>
                    {s.item_count ? (
                      <Link
                        class="text-brand-700 hover:underline"
                        href={`/items/?supplier=${s.id}`}
                      >
                        {s.item_count}
                      </Link>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td class={td}>
                    <Badge tone={s.active ? "brand" : "muted"}>
                      {s.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                  <td class={[td, "whitespace-nowrap text-right"]}>
                    {canEdit && (
                      <button
                        class={btnGhost}
                        onClick$={() => {
                          Object.assign(form, {
                            id: s.id,
                            name: s.name,
                            country: s.country,
                            currency: s.currency,
                            contact_person: s.contact_person ?? "",
                            phone: s.phone ?? "",
                            email: s.email ?? "",
                            address: s.address ?? "",
                            notes: s.notes ?? "",
                            active: s.active,
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
                          deleting.value = s;
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
      </section>

      <Modal open={open} title={form.id ? "Edit supplier" : "New supplier"} wide>
        <form class="space-y-3" preventdefault:submit onSubmit$={save}>
          {formError.value && <Alert>{formError.value}</Alert>}
          {!form.id && (
            <p class={hint}>The supplier code is given automatically when you save.</p>
          )}
          <div class="grid gap-3 sm:grid-cols-2">
            <Field label="Name" class="sm:col-span-2">
              <input
                class={input}
                required
                value={form.name}
                onInput$={(_, el) => (form.name = el.value)}
              />
            </Field>
            <Field label="Country">
              <select class={input} onChange$={(_, el) => (form.country = el.value)}>
                {COUNTRIES.map((c) => (
                  <option key={c.value} value={c.value} selected={c.value === form.country}>
                    {c.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Default currency" hint="For its prices and POs.">
              <select class={input} onChange$={(_, el) => (form.currency = el.value)}>
                {CURRENCIES.map((c) => (
                  <option key={c} value={c} selected={c === form.currency}>
                    {c}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Contact person">
              <input
                class={input}
                value={form.contact_person}
                onInput$={(_, el) => (form.contact_person = el.value)}
              />
            </Field>
            <Field label="Phone">
              <input
                class={input}
                value={form.phone}
                onInput$={(_, el) => (form.phone = el.value)}
              />
            </Field>
            <Field label="Email" class="sm:col-span-2">
              <input
                class={input}
                type="email"
                value={form.email}
                onInput$={(_, el) => (form.email = el.value)}
              />
            </Field>
            <Field label="Address" class="sm:col-span-2">
              <textarea
                class={input}
                rows={2}
                value={form.address}
                onInput$={(_, el) => (form.address = el.value)}
              />
            </Field>
            <Field label="Notes" class="sm:col-span-2">
              <textarea
                class={input}
                rows={2}
                value={form.notes}
                onInput$={(_, el) => (form.notes = el.value)}
              />
            </Field>
          </div>
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
        title={`Delete ${deleting.value?.name ?? "supplier"}?`}
        onConfirm$={$(async () => {
          if (deleting.value) unwrap(await commands.deleteSupplier(deleting.value.id));
        })}
      >
        <p>
          This deletes the supplier on every PC and can't be undone. Suppliers that items still
          use can't be deleted; deactivate them instead.
        </p>
      </Confirm>
    </>
  );
});

export const head: DocumentHead = { title: "Suppliers · Sanco" };
