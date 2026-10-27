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
import {
  commands,
  type ItemInfo,
  type ItemSuggestions,
  type RateInfo,
  type SupplierChoice,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatPct,
  formatPesoOr,
  parseCount,
  formatQty,
  parsePct,
  pctText,
  todayManila,
} from "~/lib/format";
import { can, sees } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { CostSheet } from "~/components/items/cost-sheet";
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
import { Pager, usePage } from "~/components/ui/pager";

/** Qwik's input typings lack `list`; this links an input to a datalist. */
const datalist = (id: string) => ({ list: id }) as Record<string, string>;

/** The first serial of a code this year: "MT26-0001". */
const serialExample = (code: string, digits: string | number) =>
  `${code.trim().toUpperCase() || "MT"}${todayManila().slice(2, 4)}-${"1".padStart(Number(digits) || 4, "0")}`;

interface Form {
  id: string;
  code: string;
  name: string;
  unit: string;
  category: string;
  supplier_id: string;
  tracks_expiry: boolean;
  reorder: string;
  margin: string;
  notes: string;
  active: boolean;
  /** Box serial code; blank = the item doesn't come in boxes. */
  serial_code: string;
  serial_digits: string;
  per_box: string;
}

const blank = (): Form => ({
  id: "",
  code: "",
  name: "",
  unit: "pc",
  category: "",
  supplier_id: "",
  tracks_expiry: true,
  reorder: "",
  margin: "",
  notes: "",
  active: true,
  serial_code: "",
  serial_digits: "4",
  per_box: "",
});

const toForm = (i: ItemInfo): Form => ({
  id: i.id,
  code: i.code,
  name: i.name,
  unit: i.unit ?? "",
  category: i.category ?? "",
  supplier_id: i.supplier_id ?? "",
  tracks_expiry: i.tracks_expiry,
  reorder: i.reorder_level == null ? "" : String(i.reorder_level),
  margin: pctText(i.default_margin_bp),
  notes: i.notes ?? "",
  active: i.active,
  serial_code: i.serial_code ?? "",
  serial_digits: String(i.serial_digits),
  per_box: i.pcs_per_box == null ? "" : String(i.pcs_per_box),
});

const ItemFormModal = component$<{
  open: Signal<boolean>;
  form: Form;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const session = useSession();
  const suppliers = useSignal<SupplierChoice[]>([]);
  const suggest = useSignal<ItemSuggestions>({ categories: [], units: [] });
  const error = useSignal("");
  const busy = useSignal(false);
  const margins = sees(session.user, "margins");
  const example = useComputed$(() => serialExample(form.serial_code, form.serial_digits));

  useLiveLoad(
    ["suppliers", "items"],
    $(async () => {
      try {
        suppliers.value = unwrap(await commands.supplierOptions());
        suggest.value = unwrap(await commands.itemSuggestions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const id = unwrap(
        await commands.saveItem({
          id: form.id || null,
          code: form.code,
          name: form.name,
          unit: form.unit,
          category: form.category,
          supplier_id: form.supplier_id || null,
          tracks_expiry: form.tracks_expiry,
          reorder_level: parseCount(form.reorder, "reorder level"),
          default_margin_bp: parsePct(form.margin, "margin"),
          notes: form.notes,
          active: form.active,
          serial_code: form.serial_code.trim(),
          serial_digits: form.serial_code.trim() ? Number(form.serial_digits) : null,
          pcs_per_box: parseCount(form.per_box, "pcs per box"),
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
    <Modal open={open} title={form.id ? "Edit item" : "New item"} wide>
      <form class="space-y-3" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Code" hint="Your own item code, e.g. GLV-100.">
            <input
              class={input}
              required
              value={form.code}
              onInput$={(_, el) => (form.code = el.value)}
            />
          </Field>
          <Field label="Name">
            <input
              class={input}
              required
              value={form.name}
              onInput$={(_, el) => (form.name = el.value)}
            />
          </Field>
          <Field label="Unit" hint="The base unit stock is counted in.">
            <input
              class={input}
              {...datalist("item-units")}
              value={form.unit}
              onInput$={(_, el) => (form.unit = el.value)}
            />
            <datalist id="item-units">
              {[...new Set(["pc", "box", "pack", "set", "bottle", ...suggest.value.units])].map(
                (u) => (
                  <option key={u} value={u} />
                ),
              )}
            </datalist>
          </Field>
          <Field label="Category">
            <input
              class={input}
              {...datalist("item-categories")}
              value={form.category}
              onInput$={(_, el) => (form.category = el.value)}
            />
            <datalist id="item-categories">
              {suggest.value.categories.map((c) => (
                <option key={c} value={c} />
              ))}
            </datalist>
          </Field>
          <Field label="Supplier">
            <select class={input} onChange$={(_, el) => (form.supplier_id = el.value)}>
              <option value="">None</option>
              {suppliers.value
                .filter((s) => s.active || s.id === form.supplier_id)
                .map((s) => (
                  <option key={s.id} value={s.id} selected={s.id === form.supplier_id}>
                    {s.label}
                  </option>
                ))}
            </select>
          </Field>
          <Field label="Reorder level" hint="Low-stock alert below this quantity.">
            <input
              class={input}
              inputMode="numeric"
              value={form.reorder}
              onInput$={(_, el) => (form.reorder = el.value)}
            />
          </Field>
          {margins && (
            <Field
              label="Default margin (%)"
              hint="Margin on price: ₱100 cost at 20% sells at ₱125."
            >
              <input
                class={input}
                inputMode="decimal"
                value={form.margin}
                onInput$={(_, el) => (form.margin = el.value)}
              />
            </Field>
          )}
          <div class="grid gap-3 sm:col-span-2 sm:grid-cols-3">
            <Field label="Serial code" hint="Letters, e.g. MT. Blank = not in boxes.">
              <input
                class={input}
                maxLength={6}
                value={form.serial_code}
                onInput$={(_, el) => (form.serial_code = el.value.toUpperCase())}
              />
            </Field>
            <Field label="Serial digits" hint={`e.g. ${example.value}`}>
              <select
                class={input}
                disabled={!form.serial_code.trim()}
                onChange$={(_, el) => (form.serial_digits = el.value)}
              >
                {["3", "4", "5"].map((n) => (
                  <option key={n} value={n} selected={n === form.serial_digits}>
                    {n}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Pcs per box" hint="Default box size when receiving.">
              <input
                class={input}
                inputMode="numeric"
                value={form.per_box}
                onInput$={(_, el) => (form.per_box = el.value)}
              />
            </Field>
          </div>
          <Field label="Notes" class="sm:col-span-2">
            <textarea
              class={input}
              rows={2}
              value={form.notes}
              onInput$={(_, el) => (form.notes = el.value)}
            />
          </Field>
        </div>
        <label class="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            class="accent-brand-500"
            checked={form.tracks_expiry}
            onChange$={(_, el) => (form.tracks_expiry = el.checked)}
          />
          Track expiry dates
        </label>
        {form.id && (
          <label class="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              class="accent-brand-500"
              checked={form.active}
              onChange$={(_, el) => (form.active = el.checked)}
            />
            Active (shown in pickers)
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
  );
});

const ItemList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const items = useSignal<ItemInfo[]>([]);
  const suppliers = useSignal<SupplierChoice[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const open = useSignal(false);

  useLiveLoad(
    ["items", "item_costs", "item_cost_charges", "charge_types", "suppliers", "doc_numbers"],
    $(async () => {
      try {
        items.value = unwrap(await commands.listItems());
        suppliers.value = unwrap(await commands.supplierOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "category", "supplier", "status"]);
    const q = f.q.toLowerCase();
    return items.value.filter(
      (i) =>
        (!q || [i.code, i.name, i.category].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.category || i.category === f.category) &&
        (!f.supplier || i.supplier_id === f.supplier) &&
        (!f.status ||
          (f.status === "duplicate" ? i.duplicate_code : (f.status === "active") === i.active)),
    );
  });
  const pg = usePage(shown);

  const categories = [...new Set(items.value.map((i) => i.category).filter(Boolean))] as string[];
  const costs = sees(session.user, "costs");
  const margins = sees(session.user, "margins");

  return (
    <>
      <PageHeader title="Item catalog" subtitle="What Sanco buys and sells.">
        {can(session.user, "items", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blank());
              open.value = true;
            }}
          >
            New item
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search code, name, category"
        selects={[
          {
            key: "category",
            label: "Categories",
            options: categories.sort().map((c) => ({ value: c, label: c })),
          },
          {
            key: "supplier",
            label: "Suppliers",
            options: suppliers.value.map((s) => ({ value: s.id, label: s.label })),
          },
          {
            key: "status",
            label: "Statuses",
            options: [
              { value: "active", label: "Active" },
              { value: "inactive", label: "Inactive" },
              { value: "duplicate", label: "Duplicate codes" },
            ],
          },
        ]}
      />
      <section class={card}>
        {shown.value.length === 0 ? (
          <Empty>{items.value.length ? "No items match." : "No items yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Code</th>
                <th class={th}>Name</th>
                <th class={th}>Category</th>
                <th class={th}>Supplier</th>
                <th class={th}>Unit</th>
                {costs && <th class={[th, "text-right"]}>Landed cost</th>}
                {margins && <th class={[th, "text-right"]}>Margin</th>}
                <th class={[th, "text-right"]}>Base price</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {pg.value.rows.map((i) => (
                <tr key={i.id} class={i.active ? "" : "text-ink-muted"}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link
                      class="font-medium text-brand-700 hover:underline"
                      href={`/items/?id=${i.id}`}
                    >
                      {i.code}
                    </Link>
                    {i.duplicate_code && (
                      <span class="ml-2">
                        <Badge tone="warning">duplicate</Badge>
                      </span>
                    )}
                  </td>
                  <td class={td}>{i.name}</td>
                  <td class={td}>{i.category ?? "—"}</td>
                  <td class={td}>{i.supplier_name ?? "—"}</td>
                  <td class={td}>{i.unit ?? "—"}</td>
                  {costs && (
                    <td class={[td, "text-right tabular-nums"]}>
                      {formatPesoOr(i.landed_cost_c)}
                    </td>
                  )}
                  {margins && (
                    <td class={[td, "text-right"]}>{formatPct(i.default_margin_bp)}</td>
                  )}
                  <td class={[td, "text-right tabular-nums"]}>
                    {formatPesoOr(i.base_price_c)}
                  </td>
                  <td class={td}>
                    <Badge tone={i.active ? "brand" : "muted"}>
                      {i.active ? "Active" : "Inactive"}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pager of={pg} />
      </section>
      <ItemFormModal
        open={open}
        form={form}
        saved$={$((id: string) => nav(`/items/?id=${id}`))}
      />
    </>
  );
});

const ItemDetail = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const nav = useNavigate();
  const item = useSignal<ItemInfo | null>(null);
  const missing = useSignal(false);
  const rates = useSignal<RateInfo[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const editOpen = useSignal(false);
  const deleteOpen = useSignal(false);
  const rate = useStore<RateForm>(blankRate("", id));
  const rateOpen = useSignal(false);

  useLiveLoad(
    ["items", "item_costs", "item_cost_charges", "charge_types", "suppliers", "clients", "client_item_rates", "settings", "doc_numbers"],
    $(async () => {
      try {
        const i = unwrap(await commands.getItem(id));
        item.value = i;
        missing.value = !i;
        rates.value = unwrap(await commands.listRates({ client_id: null, item_id: id }));
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  if (missing.value) {
    return (
      <>
        <PageHeader title="Item not found" />
        <p class="text-sm">
          It may have been deleted on another PC.{" "}
          <Link class="text-brand-700 hover:underline" href="/items/">
            Back to the catalog
          </Link>
        </p>
      </>
    );
  }
  const i = item.value;
  if (!i) return error.value ? <Alert>{error.value}</Alert> : null;

  const costs = sees(session.user, "costs");
  const margins = sees(session.user, "margins");
  const canEdit = can(session.user, "items", "edit");
  const rows: [string, string][] = [
    ["Unit", i.unit ?? ""],
    ["Category", i.category ?? ""],
    ["Supplier", i.supplier_name ?? ""],
    ["Expiry dates", i.tracks_expiry ? "Tracked" : "Not tracked"],
    ["Reorder level", i.reorder_level == null ? "" : `${i.reorder_level} ${i.unit ?? ""}`],
    [
      "Boxes",
      i.serial_code
        ? `Serials ${serialExample(i.serial_code, i.serial_digits)}…${i.pcs_per_box ? ` · ${formatQty(i.pcs_per_box, i.unit)} a box` : ""}`
        : "Not in boxes",
    ],
  ];
  if (costs) rows.push(["Landed cost", formatPesoOr(i.landed_cost_c)]);
  if (margins) rows.push(["Default margin", formatPct(i.default_margin_bp)]);
  rows.push(["Base price", i.base_price_c == null ? "Needs a cost sheet and margin" : `${formatPesoOr(i.base_price_c)} before VAT`]);
  rows.push(["Notes", i.notes ?? ""]);

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/items/">
          ← Catalog
        </Link>
      </p>
      <PageHeader title={`${i.code} · ${i.name}`}>
        {can(session.user, "items", "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (deleteOpen.value = true)}>
            Delete
          </button>
        )}
        {canEdit && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, toForm(i));
              editOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 space-y-2">
        {i.duplicate_code && (
          <Alert tone="warning">
            Another item has the code {i.code} (probably added on two PCs while offline). Give one
            of them a new code, or delete the extra one.
          </Alert>
        )}
        {!i.active && <Alert tone="warning">This item is inactive.</Alert>}
      </div>
      <div class="grid gap-4 2xl:grid-cols-[22rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <h2 class="mb-3 font-semibold">Details</h2>
          <InfoList rows={rows} />
        </section>
        <div class="space-y-4">
          {costs && <CostSheet itemId={id} editable={canEdit} />}
          <section class={card}>
            <div class="flex items-center justify-between border-b border-line px-4 py-3">
              <div>
                <h2 class="font-semibold">Client rates</h2>
                <p class={hint}>Clients without a rate pay their margin tier or the base price.</p>
              </div>
              {canEdit && (
                <button
                  class={btnSecondary}
                  onClick$={() => {
                    Object.assign(rate, blankRate("", id));
                    rateOpen.value = true;
                  }}
                >
                  Add rate
                </button>
              )}
            </div>
            <RatesTable
              rates={rates.value}
              showClient
              edit$={
                canEdit
                  ? $((r: RateInfo) => {
                      Object.assign(rate, rateForm(r));
                      rateOpen.value = true;
                    })
                  : undefined
              }
            />
          </section>
        </div>
      </div>
      <ItemFormModal open={editOpen} form={form} />
      <RateModal open={rateOpen} form={rate} lock="item" />
      <Confirm
        open={deleteOpen}
        title={`Delete ${i.code}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteItem(id));
          await nav("/items/");
        })}
      >
        <p>
          This deletes the item, its cost sheet and {rates.value.length} client rate(s) on every
          PC. It can't be undone. To keep the history, mark it inactive instead.
        </p>
      </Confirm>
    </>
  );
});

export default component$(() => {
  const loc = useLocation();
  const id = loc.url.searchParams.get("id");
  return id ? <ItemDetail key={id} id={id} /> : <ItemList />;
});

export const head: DocumentHead = { title: "Items · Sanco" };
