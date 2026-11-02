import { $, component$, useComputed$, useSignal, useStore } from "@builder.io/qwik";
import { Link, useLocation, type DocumentHead } from "@builder.io/qwik-city";
import { commands, type LocationInfo, type LotInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { formatDay, formatMonth, formatQty, serialSpan } from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { ExpiryBadge } from "~/components/inventory/expiry-badge";
import { Pager, usePage } from "~/components/ui/pager";
import { Alert, Empty, Field, Modal, PageHeader } from "~/components/ui/ui";
import { btnGhost, btnPrimary, btnSecondary, card, hint, input, table, td, th } from "~/components/ui/styles";

interface Form {
  id: string;
  title: string;
  tracks_expiry: boolean;
  expiry: string;
  serial_from: string;
  serial_to: string;
}

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  const lots = useSignal<LotInfo[]>([]);
  const locations = useSignal<LocationInfo[]>([]);
  const error = useSignal("");
  const open = useSignal(false);
  const formError = useSignal("");
  const busy = useSignal(false);
  const form = useStore<Form>({
    id: "",
    title: "",
    tracks_expiry: false,
    expiry: "",
    serial_from: "",
    serial_to: "",
  });

  useLiveLoad(
    ["items", "locations", "lots", "stock_docs", "stock_doc_lines", "po_receipts", "delivery_receipts"],
    $(async () => {
      try {
        lots.value = unwrap(await commands.listLots());
        locations.value = unwrap(await commands.listLocations());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const g = filtersOf(loc.url, ["q", "location"]);
    const q = g.q.toLowerCase();
    return lots.value.filter(
      (l) =>
        (!g.location || l.by_location.some((b) => b.location_id === g.location)) &&
        (!q ||
          [l.item_code, l.item_name, l.serial_from, l.serial_to, formatMonth(l.month)].some((v) =>
            v?.toLowerCase().includes(q),
          )),
    );
  });
  const pg = usePage(shown);

  const save = $(async () => {
    busy.value = true;
    formError.value = "";
    try {
      unwrap(
        await commands.saveLot({
          id: form.id,
          expiry: form.expiry || null,
          serial_from: form.serial_from,
          serial_to: form.serial_to,
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

  return (
    <>
      <PageHeader
        title="Lots"
        subtitle="Every shipment by item and arrival month, sold out or not. Oldest first."
      />
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, serial, month"
        selects={[
          {
            key: "location",
            label: "Locations",
            options: locations.value.map((l) => ({ value: l.id, label: l.name })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>No lots yet. Stock receipts and receiving reports make them.</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Item</th>
                <th class={th}>Lot</th>
                <th class={th}>Serial #</th>
                <th class={th}>Expiry</th>
                <th class={th}>Where</th>
                <th class={[th, "text-right"]}>On hand</th>
                {canEdit && <th class={th} />}
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((l) => (
                <tr key={l.id}>
                  <td class={td}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/inventory/?item=${l.item_id}`}>
                      {l.item_code}
                    </Link>{" "}
                    {l.item_name}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatMonth(l.month)}</td>
                  <td class={[td, "whitespace-nowrap"]}>{serialSpan(l.serial_from, l.serial_to) || "—"}</td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {l.expiry ? (
                      <>
                        {formatDay(l.expiry)} <ExpiryBadge days={l.days_left} />
                      </>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td class={td}>
                    {l.by_location.length
                      ? l.by_location.map((b) => `${b.location_name ?? "Deleted location"} ${formatQty(b.qty)}`).join(" · ")
                      : "—"}
                  </td>
                  <td class={[td, "text-right tabular-nums whitespace-nowrap", l.on_hand < 0 && "text-danger"]}>
                    {formatQty(l.on_hand, l.unit)}
                  </td>
                  {canEdit && (
                    <td class={[td, "text-right"]}>
                      <button
                        type="button"
                        class={btnGhost}
                        onClick$={() => {
                          form.id = l.id;
                          form.title = `${l.item_code} · ${formatMonth(l.month)}`;
                          form.tracks_expiry = l.tracks_expiry || !!l.expiry;
                          form.expiry = l.expiry ?? "";
                          form.serial_from = l.serial_from ?? "";
                          form.serial_to = l.serial_to ?? "";
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
        <Pager of={pg} />
      </section>
      <Modal open={open} title={`Lot ${form.title}`}>
        <form class="space-y-4" preventdefault:submit onSubmit$={save}>
          {formError.value && <Alert>{formError.value}</Alert>}
          <div class="grid gap-3 sm:grid-cols-2">
            <Field label="Serial # from">
              <input
                class={input}
                value={form.serial_from}
                onInput$={(_, el) => (form.serial_from = el.value)}
              />
            </Field>
            <Field label="Serial # to">
              <input
                class={input}
                value={form.serial_to}
                onInput$={(_, el) => (form.serial_to = el.value)}
              />
            </Field>
            {form.tracks_expiry && (
              <Field label="Expiry">
                <input
                  type="date"
                  class={input}
                  value={form.expiry}
                  onChange$={(_, el) => (form.expiry = el.value)}
                />
              </Field>
            )}
          </div>
          <p class={hint}>The serial range is for reference only. It doesn't change any stock.</p>
          <div class="flex justify-end gap-2">
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

export const head: DocumentHead = { title: "Lots · Sanco" };
