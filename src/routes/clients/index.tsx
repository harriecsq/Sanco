import {
  $,
  component$,
  useComputed$,
  useSignal,
  useStore,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import {
  Link,
  useLocation,
  useNavigate,
  type DocumentHead,
} from "@builder.io/qwik-city";
import { commands, type ClientInfo, type RateInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatPct,
  formatPesoOr,
  moneyText,
  parseCount,
  parseMoney,
  parsePct,
  pctText,
} from "~/lib/format";
import { can, sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import {
  blankRate,
  RateModal,
  RatesTable,
  rateForm,
  type RateForm,
} from "~/components/rates/rates";
import {
  Alert,
  Badge,
  Confirm,
  Empty,
  Field,
  InfoList,
  Modal,
  NoAccess,
  PageHeader,
} from "~/components/ui/ui";
import {
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
  contact_person: string;
  phone: string;
  email: string;
  tin: string;
  address: string;
  terms: string;
  tier: string;
  credit: string;
  notes: string;
  active: boolean;
}

const blank = (): Form => ({
  id: "",
  name: "",
  contact_person: "",
  phone: "",
  email: "",
  tin: "",
  address: "",
  terms: "30",
  tier: "",
  credit: "",
  notes: "",
  active: true,
});

const toForm = (c: ClientInfo): Form => ({
  id: c.id,
  name: c.name,
  contact_person: c.contact_person ?? "",
  phone: c.phone ?? "",
  email: c.email ?? "",
  tin: c.tin ?? "",
  address: c.address ?? "",
  terms: c.terms_days == null ? "" : String(c.terms_days),
  tier: pctText(c.margin_tier_bp),
  credit: moneyText(c.credit_limit_c),
  notes: c.notes ?? "",
  active: c.active,
});

const ClientFormModal = component$<{
  open: Signal<boolean>;
  form: Form;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const session = useSession();
  const error = useSignal("");
  const busy = useSignal(false);
  const margins = sees(session.user, "margins");

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const id = unwrap(
        await commands.saveClient({
          id: form.id || null,
          name: form.name,
          contact_person: form.contact_person,
          phone: form.phone,
          email: form.email,
          tin: form.tin,
          address: form.address,
          terms_days: parseCount(form.terms, "number of days"),
          margin_tier_bp: parsePct(form.tier, "margin"),
          credit_limit_c: parseMoney(form.credit, "credit limit"),
          notes: form.notes,
          active: form.active,
        }),
      );
      open.value = false;
      if (saved$) await saved$(id);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  });

  return (
    <Modal open={open} title={form.id ? "Edit client" : "New client"} wide>
      <form class="space-y-3" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        {!form.id && (
          <p class={hint}>The client code is given automatically when you save.</p>
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
          <Field label="Email">
            <input
              class={input}
              type="email"
              value={form.email}
              onInput$={(_, el) => (form.email = el.value)}
            />
          </Field>
          <Field label="TIN">
            <input
              class={input}
              value={form.tin}
              onInput$={(_, el) => (form.tin = el.value)}
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
          <Field label="Payment terms (days)">
            <input
              class={input}
              inputMode="numeric"
              value={form.terms}
              onInput$={(_, el) => (form.terms = el.value)}
            />
          </Field>
          <Field label="Credit limit (₱)" hint="Leave blank for no limit.">
            <input
              class={input}
              inputMode="decimal"
              value={form.credit}
              onInput$={(_, el) => (form.credit = el.value)}
            />
          </Field>
          {margins && (
            <Field
              label="Margin tier (%)"
              hint="Prices items at landed cost ÷ (1 − tier) unless the client has a rate. Blank uses each item's default margin."
            >
              <input
                class={input}
                inputMode="decimal"
                value={form.tier}
                onInput$={(_, el) => (form.tier = el.value)}
              />
            </Field>
          )}
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
            Active (shown in pickers for new bookings)
          </label>
        )}
        <div class="flex justify-end gap-2 pt-2">
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
      </form>
    </Modal>
  );
});

const ClientList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const clients = useSignal<ClientInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const open = useSignal(false);

  useLiveLoad(
    ["clients", "doc_numbers", "client_item_rates"],
    $(async () => {
      try {
        clients.value = unwrap(await commands.listClients());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "status"]);
    const q = f.q.toLowerCase();
    return clients.value.filter(
      (c) =>
        (!q ||
          [c.code, c.name, c.contact_person, c.tin, c.phone].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.status || (f.status === "active") === c.active),
    );
  });

  const margins = sees(session.user, "margins");

  return (
    <>
      <PageHeader title="Clients" subtitle="Retailers and other buyers.">
        {can(session.user, "clients", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blank());
              open.value = true;
            }}
          >
            New client
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, contact, TIN"
        selects={[
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
          <Empty>
            {clients.value.length ? "No clients match." : "No clients yet."}
          </Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Code</th>
                <th class={th}>Name</th>
                <th class={th}>Contact</th>
                <th class={[th, "text-right"]}>Terms</th>
                {margins && <th class={[th, "text-right"]}>Tier</th>}
                <th class={[th, "text-right"]}>Credit limit</th>
                <th class={[th, "text-right"]}>Rates</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((c) => (
                <tr key={c.id} class={c.active ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    {c.code ?? "—"}
                    {!c.code_confirmed && (
                      <span class={["ml-1", hint]} title="Given offline: may change after sync">
                        *
                      </span>
                    )}
                  </td>
                  <td class={td}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/clients/?id=${c.id}`}
                    >
                      {c.name}
                    </Link>
                    {c.duplicate_name && (
                      <span class="ml-2">
                        <Badge tone="warning">duplicate name</Badge>
                      </span>
                    )}
                  </td>
                  <td class={td}>
                    {[c.contact_person, c.phone].filter(Boolean).join(" · ") || "—"}
                  </td>
                  <td class={[td, "text-right"]}>
                    {c.terms_days == null ? "—" : `${c.terms_days} d`}
                  </td>
                  {margins && (
                    <td class={[td, "text-right"]}>{formatPct(c.margin_tier_bp)}</td>
                  )}
                  <td class={[td, "text-right tabular-nums"]}>
                    {formatPesoOr(c.credit_limit_c)}
                  </td>
                  <td class={[td, "text-right"]}>{c.rate_count || "—"}</td>
                  <td class={td}>
                    <Badge tone={c.active ? "brand" : "muted"}>
                      {c.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <ClientFormModal
        open={open}
        form={form}
        saved$={$((id: string) => nav(`/clients/?id=${id}`))}
      />
    </>
  );
});

const ClientDetail = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const nav = useNavigate();
  const client = useSignal<ClientInfo | null>(null);
  const missing = useSignal(false);
  const rates = useSignal<RateInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const editOpen = useSignal(false);
  const deleteOpen = useSignal(false);
  const rate = useStore<RateForm>(blankRate(id));
  const rateOpen = useSignal(false);

  useLiveLoad(
    ["clients", "doc_numbers", "client_item_rates", "items", "item_costs", "item_cost_charges", "settings"],
    $(async () => {
      try {
        const c = unwrap(await commands.getClient(id));
        client.value = c;
        missing.value = !c;
        rates.value = unwrap(await commands.listRates({ client_id: id, item_id: null }));
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const c = client.value;
  if (missing.value) {
    return (
      <>
        <PageHeader title="Client not found" />
        <p class="text-sm">
          It may have been deleted on another PC.{" "}
          <Link class="text-brand-700 hover:underline" href="/clients/">
            Back to clients
          </Link>
        </p>
      </>
    );
  }
  if (!c) return error.value ? <Alert>{error.value}</Alert> : null;

  const margins = sees(session.user, "margins");
  const canEditRates = can(session.user, "items", "edit");
  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/clients/">
          ← Clients
        </Link>
      </p>
      <PageHeader
        title={c.name}
        subtitle={`${c.code ?? "No code"}${c.code_confirmed ? "" : " (given offline, may change)"}`}
      >
        {can(session.user, "clients", "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (deleteOpen.value = true)}>
            Delete
          </button>
        )}
        {can(session.user, "clients", "edit") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, toForm(c));
              editOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      {!c.active && (
        <div class="mb-4">
          <Alert tone="warning">This client is inactive.</Alert>
        </div>
      )}
      <div class="grid gap-4 lg:grid-cols-[22rem_minmax(0,1fr)]">
        <section class={[card, "p-5"]}>
          <h2 class="mb-3 font-semibold">Details</h2>
          <InfoList
            rows={[
              ["Contact person", c.contact_person ?? ""],
              ["Phone", c.phone ?? ""],
              ["Email", c.email ?? ""],
              ["TIN", c.tin ?? ""],
              ["Address", c.address ?? ""],
              ["Payment terms", c.terms_days == null ? "" : `${c.terms_days} days`],
              ["Credit limit", c.credit_limit_c == null ? "No limit" : formatPesoOr(c.credit_limit_c)],
              ...(margins
                ? ([["Margin tier", c.margin_tier_bp == null ? "Item defaults" : formatPct(c.margin_tier_bp)]] as [string, string][])
                : []),
              ["Notes", c.notes ?? ""],
            ]}
          />
        </section>
        <div class="space-y-4">
          <section class={card}>
            <div class="flex items-center justify-between border-b border-line px-4 py-3">
              <div>
                <h2 class="font-semibold">Rates</h2>
                <p class={hint}>
                  Manual prices for this client. Other items use{" "}
                  {c.margin_tier_bp != null || !margins ? "the client's margin tier" : "each item's default margin"}.
                </p>
              </div>
              {canEditRates && (
                <button
                  class={btnSecondary}
                  onClick$={() => {
                    Object.assign(rate, blankRate(id));
                    rateOpen.value = true;
                  }}
                >
                  Add rate
                </button>
              )}
            </div>
            <RatesTable
              rates={rates.value}
              showItem
              edit$={
                canEditRates
                  ? $((r: RateInfo) => {
                      Object.assign(rate, rateForm(r));
                      rateOpen.value = true;
                    })
                  : undefined
              }
            />
          </section>
          <section class={[card, "p-5"]}>
            <h2 class="font-semibold">Bookings, deliveries and balance</h2>
            <p class={["mt-1", hint]}>
              Bookings and DRs arrive in M7; balance, payments and checks in M8.
            </p>
          </section>
        </div>
      </div>
      <ClientFormModal open={editOpen} form={form} />
      <RateModal open={rateOpen} form={rate} lock="client" />
      <Confirm
        open={deleteOpen}
        title={`Delete ${c.name}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteClient(id));
          await nav("/clients/");
        })}
      >
        <p>
          This deletes the client and its {rates.value.length} rate(s) on every PC. It
          can't be undone. To keep the history, mark the client inactive instead.
        </p>
      </Confirm>
    </>
  );
});

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  if (!can(session.user, "clients")) return <NoAccess />;
  const id = loc.url.searchParams.get("id");
  return id ? <ClientDetail key={id} id={id} /> : <ClientList />;
});

export const head: DocumentHead = { title: "Clients · Sanco" };

