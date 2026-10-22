import { $, component$, useSignal, useStore } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type Action,
  type Field,
  type Module,
  type PermCatalog,
  type RoleInfo,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { isAdmin } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar } from "~/components/filter-bar/filter-bar";
import { useLocation } from "@builder.io/qwik-city";
import {
  Alert,
  Badge,
  Empty,
  Modal,
  NoAccess,
  PageHeader,
} from "~/components/ui/ui";
import {
  btnDanger,
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  label,
} from "~/components/ui/styles";

interface Form {
  id: string;
  name: string;
  admin: boolean;
  modules: Partial<Record<Module, Action[]>>;
  fields: Field[];
}

const blank = (): Form => ({
  id: "",
  name: "",
  admin: false,
  modules: {},
  fields: [],
});

function summary(r: RoleInfo, catalog: PermCatalog | null): string {
  if (r.perms.admin) return "Everything";
  const mods = Object.keys(r.perms.modules ?? {}) as Module[];
  if (mods.length === 0) return "Nothing yet";
  const names = mods.map(
    (m) => catalog?.modules.find((x) => x.id === m)?.label ?? m,
  );
  return names.join(", ");
}

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const roles = useSignal<RoleInfo[]>([]);
  const catalog = useSignal<PermCatalog | null>(null);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const formError = useSignal("");
  const open = useSignal(false);
  const delOpen = useSignal(false);
  const busy = useSignal(false);

  const load = $(async () => {
    try {
      catalog.value ??= await commands.permCatalog();
      roles.value = unwrap(await commands.listRoles());
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad(["roles", "user_roles", "users"], load);

  const toggle = $((m: Module, a: Action, on: boolean) => {
    const cur = new Set(form.modules[m] ?? []);
    if (on) {
      cur.add(a);
      cur.add("view");
    } else {
      cur.delete(a);
      // Without view, nothing else makes sense.
      if (a === "view") cur.clear();
    }
    form.modules = { ...form.modules, [m]: [...cur] };
  });

  if (!isAdmin(session.user)) return <NoAccess />;
  const q = (loc.url.searchParams.get("q") ?? "").toLowerCase();
  const shown = roles.value.filter((r) => r.name.toLowerCase().includes(q));
  const cat = catalog.value;

  return (
    <>
      <PageHeader
        title="Roles"
        subtitle="What each role may do, per module. A user with several roles gets everything any of them grants."
      >
        <button
          q:slot="actions"
          class={btnPrimary}
          onClick$={() => {
            Object.assign(form, blank());
            formError.value = "";
            open.value = true;
          }}
        >
          New role
        </button>
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar search="Search roles" />
      {shown.length === 0 ? (
        <section class={card}>
          <Empty>No roles match.</Empty>
        </section>
      ) : (
        <div class="grid gap-3 md:grid-cols-2">
          {shown.map((r) => (
            <section key={r.id} class={[card, "space-y-2 p-4"]}>
              <div class="flex items-center justify-between gap-2">
                <h2 class="font-semibold">
                  {r.name} {r.perms.admin && <Badge tone="brand">admin</Badge>}
                </h2>
                <button
                  class={btnGhost}
                  onClick$={() => {
                    Object.assign(form, {
                      id: r.id,
                      name: r.name,
                      admin: !!r.perms.admin,
                      modules: { ...(r.perms.modules ?? {}) },
                      fields: [...(r.perms.fields ?? [])],
                    });
                    formError.value = "";
                    open.value = true;
                  }}
                >
                  Edit
                </button>
              </div>
              <p class="text-sm">{summary(r, cat)}</p>
              <p class={hint}>
                {r.user_count} active user{r.user_count === 1 ? "" : "s"}
                {!r.perms.admin &&
                  (r.perms.fields?.length
                    ? ` · sees ${r.perms.fields
                        .map(
                          (f) =>
                            cat?.fields.find((x) => x.id === f)?.label ?? f,
                        )
                        .join(", ")
                        .toLowerCase()}`
                    : " · no costs or margins")}
              </p>
            </section>
          ))}
        </div>
      )}

      <Modal
        open={open}
        title={form.id ? `Edit role: ${form.name}` : "New role"}
        wide
      >
        <form
          class="space-y-4"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            formError.value = "";
            try {
              unwrap(
                await commands.saveRole({
                  id: form.id || null,
                  name: form.name,
                  perms: {
                    admin: form.admin,
                    modules: form.admin ? {} : { ...form.modules },
                    fields: form.admin ? [] : [...form.fields],
                  },
                }),
              );
              open.value = false;
            } catch (e) {
              formError.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          {formError.value && <Alert>{formError.value}</Alert>}
          <label class="block space-y-1">
            <span class={label}>Name</span>
            <input
              class={input}
              required
              value={form.name}
              onInput$={(_, el) => (form.name = el.value)}
            />
          </label>
          <label class="flex items-start gap-2 text-sm">
            <input
              type="checkbox"
              class="mt-0.5 accent-brand-500"
              checked={form.admin}
              onChange$={(_, el) => (form.admin = el.checked)}
            />
            <span>
              <b>Admin</b>: everything, including users, roles, join codes and
              restoring backups.
            </span>
          </label>
          {!form.admin && cat && (
            <>
              <table class="w-full text-sm">
                <thead>
                  <tr class="text-xs text-ink-muted">
                    <th class="py-1 text-left font-medium">Module</th>
                    {cat.actions.map((a) => (
                      <th key={a.id} class="px-2 py-1 font-medium">
                        {a.label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {cat.modules.map((m) => (
                    <tr key={m.id} class="border-t border-line">
                      <td class="py-1.5">{m.label}</td>
                      {cat.actions.map((a) => (
                        <td key={a.id} class="px-2 text-center">
                          <input
                            type="checkbox"
                            class="accent-brand-500"
                            aria-label={`${a.label} ${m.label}`}
                            checked={!!form.modules[m.id]?.includes(a.id)}
                            onChange$={(_, el) =>
                              toggle(m.id, a.id, el.checked)
                            }
                          />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
              <p class={hint}>
                Settings access covers devices, conflicts, backups and the audit
                log. Users, roles and restores are for admins only.
              </p>
              <fieldset class="space-y-1">
                <legend class={label}>Sensitive values</legend>
                {cat.fields.map((f) => (
                  <label key={f.id} class="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      class="accent-brand-500"
                      checked={form.fields.includes(f.id)}
                      onChange$={(_, el) => {
                        form.fields = el.checked
                          ? [...form.fields, f.id]
                          : form.fields.filter((x) => x !== f.id);
                      }}
                    />
                    May see {f.label.toLowerCase()}
                  </label>
                ))}
              </fieldset>
            </>
          )}
          <div class="flex justify-between gap-2 pt-2">
            <div>
              {form.id && (
                <button
                  type="button"
                  class={btnGhost}
                  onClick$={() => {
                    open.value = false;
                    delOpen.value = true;
                  }}
                >
                  Delete role
                </button>
              )}
            </div>
            <div class="flex gap-2">
              <button
                type="button"
                class={btnSecondary}
                onClick$={() => (open.value = false)}
              >
                Cancel
              </button>
              <button class={btnPrimary} disabled={busy.value}>
                Save
              </button>
            </div>
          </div>
        </form>
      </Modal>

      <Modal open={delOpen} title={`Delete role: ${form.name}`}>
        <div class="space-y-3 text-sm">
          {formError.value && <Alert>{formError.value}</Alert>}
          <p>Users with this role lose what it grants. This can't be undone.</p>
          <div class="flex justify-end gap-2">
            <button
              class={btnSecondary}
              onClick$={() => (delOpen.value = false)}
            >
              Cancel
            </button>
            <button
              class={btnDanger}
              disabled={busy.value}
              onClick$={async () => {
                busy.value = true;
                formError.value = "";
                try {
                  unwrap(await commands.deleteRole(form.id));
                  delOpen.value = false;
                } catch (e) {
                  formError.value = errorText(e);
                } finally {
                  busy.value = false;
                }
              }}
            >
              Delete
            </button>
          </div>
        </div>
      </Modal>
    </>
  );
});

export const head: DocumentHead = { title: "Roles · Sanco" };
