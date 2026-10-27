import {
  $,
  component$,
  useComputed$,
  useSignal,
  useStore,
} from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type RoleInfo, type UserInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDate } from "~/lib/format";
import { isAdmin } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { Pager, usePage } from "~/components/ui/pager";
import {
  Alert,
  Badge,
  Empty,
  Modal,
  NoAccess,
  PageHeader,
} from "~/components/ui/ui";
import {
  btnGhost,
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
  label,
  table,
  td,
  th,
} from "~/components/ui/styles";

interface Form {
  /** Empty when creating. */
  id: string;
  username: string;
  display_name: string;
  password: string;
  active: boolean;
  role_ids: string[];
}

const blank = (): Form => ({
  id: "",
  username: "",
  display_name: "",
  password: "",
  active: true,
  role_ids: [],
});

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const users = useSignal<UserInfo[]>([]);
  const roles = useSignal<RoleInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const formError = useSignal("");
  const editOpen = useSignal(false);
  const pwOpen = useSignal(false);
  const pw = useSignal("");
  const busy = useSignal(false);

  const load = $(async () => {
    try {
      users.value = unwrap(await commands.listUsers());
      roles.value = unwrap(await commands.listRoles());
    } catch (e) {
      error.value = errorText(e);
    }
  });
  useLiveLoad(["users", "user_roles", "roles"], load);

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "status", "role"]);
    const q = f.q.toLowerCase();
    return users.value.filter(
      (u) =>
        (!q ||
          u.username.includes(q) ||
          u.display_name.toLowerCase().includes(q)) &&
        (!f.status || (f.status === "active") === u.active) &&
        (!f.role || u.role_ids.includes(f.role)),
    );
  });
  const pg = usePage(shown);

  const roleName = (id: string) =>
    roles.value.find((r) => r.id === id)?.name ?? "?";

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      if (form.id) {
        unwrap(
          await commands.updateUser({
            id: form.id,
            display_name: form.display_name,
            active: form.active,
            role_ids: [...form.role_ids],
          }),
        );
      } else {
        unwrap(
          await commands.createUser({
            username: form.username,
            display_name: form.display_name,
            password: form.password,
            role_ids: [...form.role_ids],
          }),
        );
      }
      editOpen.value = false;
    } catch (e) {
      formError.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  if (!isAdmin(session.user)) return <NoAccess />;

  return (
    <>
      <PageHeader
        title="Users"
        subtitle="One account per person. Users are deactivated, never deleted, so their history stays."
      >
        <button
          q:slot="actions"
          class={btnPrimary}
          onClick$={() => {
            Object.assign(form, blank());
            formError.value = "";
            editOpen.value = true;
          }}
        >
          New user
        </button>
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search name or username"
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "active", label: "Active" },
              { value: "inactive", label: "Inactive" },
            ],
          },
          {
            key: "role",
            label: "Roles",
            options: roles.value.map((r) => ({ value: r.id, label: r.name })),
          },
        ]}
      />
      <section class={card}>
        {shown.value.length === 0 ? (
          <Empty>No users match.</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Name</th>
                <th class={th}>Username</th>
                <th class={th}>Roles</th>
                <th class={th}>Status</th>
                <th class={th}>Created</th>
                <th class={th} />
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((u) => (
                <tr key={u.id} class={u.active ? "" : "text-ink-muted"}>
                  <td class={[td, "font-medium"]}>
                    {u.display_name}
                    {u.id === session.user?.id && (
                      <span class={["ml-2", hint]}>(you)</span>
                    )}
                  </td>
                  <td class={td}>{u.username}</td>
                  <td class={td}>
                    {u.role_ids.length
                      ? u.role_ids.map(roleName).join(", ")
                      : "—"}
                  </td>
                  <td class={td}>
                    <Badge tone={u.active ? "brand" : "muted"}>
                      {u.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                  <td class={td}>{formatDate(u.created_at)}</td>
                  <td class={[td, "whitespace-nowrap text-right"]}>
                    <button
                      class={btnGhost}
                      onClick$={() => {
                        Object.assign(form, {
                          ...blank(),
                          id: u.id,
                          username: u.username,
                          display_name: u.display_name,
                          active: u.active,
                          role_ids: [...u.role_ids],
                        });
                        formError.value = "";
                        editOpen.value = true;
                      }}
                    >
                      Edit
                    </button>
                    <button
                      class={btnGhost}
                      onClick$={() => {
                        form.id = u.id;
                        form.display_name = u.display_name;
                        pw.value = "";
                        formError.value = "";
                        pwOpen.value = true;
                      }}
                    >
                      Reset password
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>

      <Modal open={editOpen} title={form.id ? "Edit user" : "New user"}>
        <form class="space-y-3" preventdefault:submit onSubmit$={save}>
          {formError.value && <Alert>{formError.value}</Alert>}
          <label class="block space-y-1">
            <span class={label}>Name</span>
            <input
              class={input}
              value={form.display_name}
              onInput$={(_, el) => (form.display_name = el.value)}
            />
          </label>
          <label class="block space-y-1">
            <span class={label}>Username</span>
            <input
              class={input}
              required
              disabled={!!form.id}
              value={form.username}
              onInput$={(_, el) => (form.username = el.value)}
            />
            {form.id && <span class={hint}>Usernames can't be changed.</span>}
          </label>
          {!form.id && (
            <label class="block space-y-1">
              <span class={label}>Password</span>
              <input
                class={input}
                type="password"
                autoComplete="new-password"
                required
                value={form.password}
                onInput$={(_, el) => (form.password = el.value)}
              />
              <span class={hint}>
                At least 6 characters. They can change it under My account.
              </span>
            </label>
          )}
          <fieldset class="space-y-1">
            <legend class={label}>Roles</legend>
            {roles.value.map((r) => (
              <label key={r.id} class="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  class="accent-brand-500"
                  checked={form.role_ids.includes(r.id)}
                  onChange$={(_, el) => {
                    form.role_ids = el.checked
                      ? [...form.role_ids, r.id]
                      : form.role_ids.filter((x) => x !== r.id);
                  }}
                />
                {r.name}
                {r.perms.admin && <Badge tone="brand">admin</Badge>}
              </label>
            ))}
          </fieldset>
          {form.id && (
            <label class="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                class="accent-brand-500"
                checked={form.active}
                onChange$={(_, el) => (form.active = el.checked)}
              />
              Active (can sign in)
            </label>
          )}
          <div class="flex justify-end gap-2 pt-2">
            <button
              type="button"
              class={btnSecondary}
              onClick$={() => (editOpen.value = false)}
            >
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Save
            </button>
          </div>
        </form>
      </Modal>

      <Modal open={pwOpen} title={`Reset password: ${form.display_name}`}>
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            formError.value = "";
            try {
              unwrap(await commands.resetPassword(form.id, pw.value));
              pwOpen.value = false;
            } catch (e) {
              formError.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          {formError.value && <Alert>{formError.value}</Alert>}
          <label class="block space-y-1">
            <span class={label}>New password</span>
            <input
              class={input}
              type="password"
              autoComplete="new-password"
              required
              bind:value={pw}
            />
          </label>
          <p class={hint}>Tell them the new password in person.</p>
          <div class="flex justify-end gap-2">
            <button
              type="button"
              class={btnSecondary}
              onClick$={() => (pwOpen.value = false)}
            >
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Set password
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

export const head: DocumentHead = { title: "Users · Sanco" };
