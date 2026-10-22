import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type Choice, type RateInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { can } from "~/lib/perms";
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
import { Alert, PageHeader } from "~/components/ui/ui";
import { btnPrimary, card } from "~/components/ui/styles";

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const rates = useSignal<RateInfo[]>([]);
  const clients = useSignal<Choice[]>([]);
  const error = useSignal("");
  const form = useStore<RateForm>(blankRate());
  const open = useSignal(false);

  useLiveLoad(
    ["client_item_rates", "clients", "items", "item_costs", "item_cost_charges", "settings", "doc_numbers"],
    $(async () => {
      try {
        rates.value = unwrap(await commands.listRates({ client_id: null, item_id: null }));
        clients.value = unwrap(await commands.clientOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "client", "status"]);
    const q = f.q.toLowerCase();
    return rates.value.filter(
      (r) =>
        (!q ||
          [r.item_code, r.item_name, r.client_name, r.client_code, r.notes].some((v) =>
            v?.toLowerCase().includes(q),
          )) &&
        (!f.client || r.client_id === f.client) &&
        (!f.status || (f.status === "below" ? r.below_cost : f.status === "vat" ? r.vat : !r.vat)),
    );
  });

  const canEdit = can(session.user, "items", "edit");
  return (
    <>
      <PageHeader
        title="Client rates"
        subtitle="Manual prices per client and item. They win over margin tiers and default margins."
      >
        {canEdit && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blankRate());
              open.value = true;
            }}
          >
            New rate
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search item, client, notes"
        selects={[
          {
            key: "client",
            label: "Clients",
            options: clients.value.map((c) => ({ value: c.id, label: c.label })),
          },
          {
            key: "status",
            label: "Rates",
            allLabel: "All rates",
            options: [
              { value: "below", label: "Below cost" },
              { value: "vat", label: "With VAT" },
              { value: "novat", label: "Without VAT" },
            ],
          },
        ]}
      />
      <section class={card}>
        <RatesTable
          rates={shown.value}
          showClient
          showItem
          edit$={
            canEdit
              ? $((r: RateInfo) => {
                  Object.assign(form, rateForm(r));
                  open.value = true;
                })
              : undefined
          }
        />
      </section>
      <RateModal open={open} form={form} />
    </>
  );
});

export const head: DocumentHead = { title: "Client rates · Sanco" };
