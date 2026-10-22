import {
  $,
  component$,
  useComputed$,
  useSignal,
  useStore,
  type QRL,
  type Signal,
} from "@builder.io/qwik";
import { Link, useLocation, useNavigate, type DocumentHead } from "@builder.io/qwik-city";
import {
  commands,
  type BdDetail,
  type BdInfo,
  type BdStatus,
  type Choice,
} from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import {
  formatDateTime,
  formatDay,
  formatPeso,
  formatPesoOr,
  formatQty,
  moneyText,
  parseCount,
  parseMoney,
  todayManila,
} from "~/lib/format";
import { can } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { FilterBar, filtersOf } from "~/components/filter-bar/filter-bar";
import { BD_STATUS, OfflineBadge, StatusBadge } from "~/components/sales/sales";
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

const TABLES = ["bd_requests", "bd_lines", "clients", "items", "doc_numbers", "users", "bookings"];

/** Item picker value for a line that isn't in the catalog. */
const FREE_TEXT = "+text";

interface FormLine {
  key: number;
  id: string | null;
  /** An item id, FREE_TEXT, or "" while not chosen. */
  item: string;
  description: string;
  qty: string;
  price: string;
}

interface Form {
  id: string;
  client_id: string;
  prospect_name: string;
  contact: string;
  date: string;
  status: BdStatus;
  follow_up: string;
  vat: boolean;
  notes: string;
  lost_reason: string;
  lines: FormLine[];
}

let nextKey = 1;
const blankLine = (): FormLine => ({
  key: nextKey++,
  id: null,
  item: "",
  description: "",
  qty: "",
  price: "",
});

const blank = (): Form => ({
  id: "",
  client_id: "",
  prospect_name: "",
  contact: "",
  date: todayManila(),
  status: "open",
  follow_up: "",
  vat: true,
  notes: "",
  lost_reason: "",
  lines: [blankLine()],
});

const toForm = (d: BdDetail): Form => ({
  id: d.request.id,
  client_id: d.request.client_id ?? "",
  prospect_name: d.request.prospect_name ?? "",
  contact: d.request.contact ?? "",
  date: d.request.date,
  status: d.request.status,
  follow_up: d.request.follow_up ?? "",
  vat: d.request.vat,
  notes: d.request.notes ?? "",
  lost_reason: d.request.lost_reason ?? "",
  lines: d.lines.map((l) => ({
    key: nextKey++,
    id: l.id,
    item: l.item_id ?? FREE_TEXT,
    description: l.description ?? "",
    qty: String(l.qty),
    price: moneyText(l.price_c),
  })),
});

/** Who asked: the client, or the prospect's name. */
const who = (r: BdInfo) => r.client_name ?? r.prospect_name ?? "—";

const followUpDue = (r: BdInfo) =>
  !!r.follow_up && (r.status === "open" || r.status === "quoted") && r.follow_up <= todayManila();

const BdFormModal = component$<{
  open: Signal<boolean>;
  form: Form;
  saved$?: QRL<(id: string) => void>;
}>(({ open, form, saved$ }) => {
  const clients = useSignal<Choice[]>([]);
  const items = useSignal<Choice[]>([]);
  const error = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    ["clients", "items", "doc_numbers"],
    $(async () => {
      try {
        clients.value = unwrap(await commands.clientOptions());
        items.value = unwrap(await commands.itemOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  /** Prefills a new line's quote with the client's price. */
  const suggest = $(async (line: FormLine) => {
    if (!form.client_id || !line.item || line.item === FREE_TEXT || line.price) return;
    const q = await commands.priceQuote(form.client_id, line.item);
    if (q.status === "ok" && q.data.net_c != null && !line.price) line.price = moneyText(q.data.net_c);
  });

  const save = $(async () => {
    busy.value = true;
    error.value = "";
    try {
      const lines = form.lines
        .filter((l) => l.item || l.qty.trim() || l.description.trim())
        .map((l) => {
          const what =
            l.item === FREE_TEXT
              ? l.description.trim() || "a line"
              : (items.value.find((i) => i.id === l.item)?.label ?? "a line");
          if (!l.item) throw new Error("choose an item on every line, or remove it");
          const qty = parseCount(l.qty, `quantity for ${what}`);
          if (!qty) throw new Error(`enter a quantity for ${what}`);
          return {
            id: l.id,
            item_id: l.item === FREE_TEXT ? null : l.item,
            description: l.item === FREE_TEXT ? l.description : "",
            qty,
            price_c: parseMoney(l.price, `price for ${what}`),
          };
        });
      const id = unwrap(
        await commands.saveBdRequest({
          id: form.id || null,
          client_id: form.client_id || null,
          prospect_name: form.client_id ? "" : form.prospect_name,
          contact: form.contact,
          date: form.date,
          status: form.status,
          follow_up: form.follow_up || null,
          vat: form.vat,
          notes: form.notes,
          lost_reason: form.lost_reason,
          lines,
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

  const pickable = (list: Choice[], current: string) =>
    list.filter((o) => o.active || o.id === current);

  return (
    <Modal open={open} title={form.id ? "Edit request" : "New request"} wide>
      <form class="space-y-4" preventdefault:submit onSubmit$={save}>
        {error.value && <Alert>{error.value}</Alert>}
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Client" hint="Leave blank for a prospect who isn't a client yet.">
            <select class={input} onChange$={(_, el) => (form.client_id = el.value)}>
              <option value="">Prospect (not a client)</option>
              {pickable(clients.value, form.client_id).map((c) => (
                <option key={c.id} value={c.id} selected={c.id === form.client_id}>
                  {c.label}
                </option>
              ))}
            </select>
          </Field>
          {!form.client_id && (
            <Field label="Prospect name">
              <input
                class={input}
                required
                value={form.prospect_name}
                onInput$={(_, el) => (form.prospect_name = el.value)}
              />
            </Field>
          )}
          <Field label="Contact" hint="Person, phone or email.">
            <input
              class={input}
              value={form.contact}
              onInput$={(_, el) => (form.contact = el.value)}
            />
          </Field>
          <Field label="Request date">
            <input
              type="date"
              class={input}
              required
              value={form.date}
              onChange$={(_, el) => (form.date = el.value)}
            />
          </Field>
          <Field label="Follow up on">
            <input
              type="date"
              class={input}
              value={form.follow_up}
              onChange$={(_, el) => (form.follow_up = el.value)}
            />
          </Field>
          <Field label="Status" hint="A request is won by converting it into a booking.">
            <select
              class={input}
              onChange$={(_, el) => (form.status = el.value as Form["status"])}
            >
              {form.status === "won" && (
                <option value="won" selected>
                  {BD_STATUS.won.label}
                </option>
              )}
              {(["open", "quoted", "lost"] as const).map((s) => (
                <option key={s} value={s} selected={form.status === s}>
                  {BD_STATUS[s].label}
                </option>
              ))}
            </select>
          </Field>
          {form.status === "lost" && (
            <Field label="Why it was lost">
              <input
                class={input}
                value={form.lost_reason}
                onInput$={(_, el) => (form.lost_reason = el.value)}
              />
            </Field>
          )}
        </div>

        <div>
          <div class="mb-1 flex items-center justify-between">
            <span class="text-sm font-medium">Requested items</span>
            <span class={hint}>Quoted prices are per unit, before VAT.</span>
          </div>
          <div class="space-y-2">
            {form.lines.map((line, idx) => (
              <div
                key={line.key}
                class="grid grid-cols-[minmax(0,1fr)_6rem_8rem_auto] items-start gap-2"
              >
                <div class="space-y-1">
                  <select
                    class={input}
                    aria-label="Item"
                    onChange$={(_, el) => {
                      line.item = el.value;
                      if (el.value !== FREE_TEXT) suggest(line);
                    }}
                  >
                    <option value="">Choose an item…</option>
                    {pickable(items.value, line.item).map((i) => (
                      <option key={i.id} value={i.id} selected={i.id === line.item}>
                        {i.label}
                      </option>
                    ))}
                    <option value={FREE_TEXT} selected={line.item === FREE_TEXT}>
                      Not in the catalog…
                    </option>
                  </select>
                  {line.item === FREE_TEXT && (
                    <input
                      class={input}
                      aria-label="Description"
                      placeholder="What they asked for"
                      required
                      value={line.description}
                      onInput$={(_, el) => (line.description = el.value)}
                    />
                  )}
                </div>
                <input
                  class={[inputBase, "w-full text-right"]}
                  aria-label="Quantity"
                  inputMode="numeric"
                  placeholder="Qty"
                  value={line.qty}
                  onInput$={(_, el) => (line.qty = el.value)}
                />
                <input
                  class={[inputBase, "w-full text-right"]}
                  aria-label="Quoted price"
                  inputMode="decimal"
                  placeholder="₱ quoted"
                  value={line.price}
                  onInput$={(_, el) => (line.price = el.value)}
                />
                <button
                  type="button"
                  class={btnGhost}
                  aria-label="Remove line"
                  onClick$={() => form.lines.splice(idx, 1)}
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
          <button type="button" class={[btnGhost, "mt-2"]} onClick$={() => form.lines.push(blankLine())}>
            + Add item
          </button>
        </div>

        <label class="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            class="accent-brand-500"
            checked={form.vat}
            onChange$={(_, el) => (form.vat = el.checked)}
          />
          Prices are subject to VAT
        </label>
        <Field label="Notes">
          <textarea
            class={input}
            rows={2}
            value={form.notes}
            onInput$={(_, el) => (form.notes = el.value)}
          />
        </Field>
        {!form.id && <p class={hint}>The request number is given when you save.</p>}
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
  );
});

const BdList = component$(() => {
  const session = useSession();
  const loc = useLocation();
  const nav = useNavigate();
  const requests = useSignal<BdInfo[]>([]);
  const clients = useSignal<Choice[]>([]);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const open = useSignal(false);

  useLiveLoad(
    TABLES,
    $(async () => {
      try {
        requests.value = unwrap(await commands.listBdRequests());
        clients.value = unwrap(await commands.clientOptions());
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  const shown = useComputed$(() => {
    const f = filtersOf(loc.url, ["q", "from", "to", "status", "client"]);
    const q = f.q.toLowerCase();
    return requests.value.filter(
      (r) =>
        (!q ||
          [r.no, who(r), r.contact, r.summary, r.notes].some((v) => v?.toLowerCase().includes(q))) &&
        (!f.from || r.date >= f.from) &&
        (!f.to || r.date <= f.to) &&
        (!f.client || r.client_id === f.client) &&
        (!f.status || (f.status === "due" ? followUpDue(r) : r.status === f.status)),
    );
  });

  return (
    <>
      <PageHeader
        title="Business Development"
        subtitle="Client and prospect inquiries. Won requests become bookings."
      >
        {can(session.user, "bd", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={() => {
              Object.assign(form, blank());
              open.value = true;
            }}
          >
            New request
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <FilterBar
        search="Search number, client, prospect, items"
        dates
        selects={[
          {
            key: "status",
            label: "Statuses",
            options: [
              ...(["open", "quoted", "won", "lost"] as const).map((s) => ({
                value: s,
                label: BD_STATUS[s].label,
              })),
              { value: "due", label: "Follow-up due" },
            ],
          },
          {
            key: "client",
            label: "Clients",
            options: clients.value.map((c) => ({ value: c.id, label: c.label })),
          },
        ]}
      />
      <section class={[card, "overflow-x-auto"]}>
        {shown.value.length === 0 ? (
          <Empty>{requests.value.length ? "No requests match." : "No requests yet."}</Empty>
        ) : (
          <table class={table}>
            <thead>
              <tr>
                <th class={th}>Number</th>
                <th class={th}>Date</th>
                <th class={th}>Client / prospect</th>
                <th class={th}>Items</th>
                <th class={[th, "text-right"]}>Quoted</th>
                <th class={th}>Follow up</th>
                <th class={th}>Status</th>
              </tr>
            </thead>
            <tbody>
              {shown.value.map((r) => (
                <tr key={r.id}>
                  <td class={[td, "whitespace-nowrap"]}>
                    <Link class="font-medium text-brand-700 hover:underline" href={`/bd/?id=${r.id}`}>
                      {r.no ?? "—"}
                    </Link>
                    <OfflineBadge show={!r.no_confirmed} />
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>{formatDay(r.date)}</td>
                  <td class={td}>
                    {who(r)}
                    {!r.client_id && <span class={[hint, "block"]}>prospect</span>}
                  </td>
                  <td class={td}>{r.summary || "—"}</td>
                  <td class={[td, "text-right tabular-nums"]}>
                    {r.quoted_net_c ? formatPeso(r.quoted_net_c) : "—"}
                  </td>
                  <td class={[td, "whitespace-nowrap"]}>
                    {r.follow_up ? formatDay(r.follow_up) : "—"}
                    {followUpDue(r) && (
                      <span class="ml-2">
                        <Badge tone="danger">due</Badge>
                      </span>
                    )}
                  </td>
                  <td class={td}>
                    <StatusBadge s={BD_STATUS[r.status]} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <BdFormModal open={open} form={form} saved$={$((id: string) => nav(`/bd/?id=${id}`))} />
    </>
  );
});

const BdView = component$<{ id: string }>(({ id }) => {
  const session = useSession();
  const nav = useNavigate();
  const detail = useSignal<BdDetail | null>(null);
  const missing = useSignal(false);
  const error = useSignal("");
  const form = useStore<Form>(blank());
  const editOpen = useSignal(false);
  const deleteOpen = useSignal(false);
  const convertOpen = useSignal(false);
  const clients = useSignal<Choice[]>([]);
  const convertClient = useSignal("");
  const convertError = useSignal("");
  const busy = useSignal(false);

  useLiveLoad(
    TABLES,
    $(async () => {
      try {
        const d = unwrap(await commands.getBdRequest(id));
        detail.value = d;
        missing.value = !d;
      } catch (e) {
        error.value = errorText(e);
      }
    }),
  );

  if (missing.value) {
    return (
      <>
        <PageHeader title="Request not found" />
        <Link class="text-sm text-brand-700 hover:underline" href="/bd/">
          Back to requests
        </Link>
      </>
    );
  }
  const det = detail.value;
  if (!det) return error.value ? <Alert>{error.value}</Alert> : null;
  const r = det.request;
  const live = r.status === "open" || r.status === "quoted";
  const skipped = det.lines.filter((l) => !l.item_id).length;

  const rows: [string, string][] = [
    ["Client", r.client_name ? `${r.client_name}${r.client_code ? ` (${r.client_code})` : ""}` : ""],
  ];
  if (!r.client_id) rows.push(["Prospect", r.prospect_name ?? ""]);
  rows.push(
    ["Contact", r.contact ?? ""],
    ["Request date", formatDay(r.date)],
    ["Follow up", r.follow_up ? formatDay(r.follow_up) : ""],
    ["VAT", r.vat ? "Prices subject to VAT" : "No VAT"],
  );
  if (r.status === "lost") rows.push(["Lost because", r.lost_reason ?? ""]);
  rows.push(
    ["Notes", r.notes ?? ""],
    ["Entered", `${formatDateTime(r.created_at)} by ${r.created_by_name ?? "—"}`],
  );

  return (
    <>
      <p class="mb-2 text-sm">
        <Link class="text-brand-700 hover:underline" href="/bd/">
          ← Requests
        </Link>
      </p>
      <PageHeader title={`Request ${r.no ?? ""}`} subtitle={who(r)}>
        {r.status !== "won" && can(session.user, "bd", "delete") && (
          <button q:slot="actions" class={btnSecondary} onClick$={() => (deleteOpen.value = true)}>
            Delete
          </button>
        )}
        {can(session.user, "bd", "edit") && (
          <button
            q:slot="actions"
            class={btnSecondary}
            onClick$={() => {
              Object.assign(form, toForm(det));
              editOpen.value = true;
            }}
          >
            Edit
          </button>
        )}
        {live && can(session.user, "bd", "edit") && can(session.user, "bookings", "create") && (
          <button
            q:slot="actions"
            class={btnPrimary}
            onClick$={async () => {
              convertClient.value = r.client_id ?? "";
              convertError.value = "";
              convertOpen.value = true;
              const c = await commands.clientOptions();
              if (c.status === "ok") clients.value = c.data;
            }}
          >
            Convert to booking
          </button>
        )}
      </PageHeader>
      {error.value && <Alert>{error.value}</Alert>}
      <div class="mb-4 flex flex-wrap items-center gap-2">
        <StatusBadge s={BD_STATUS[r.status]} />
        {followUpDue(r) && <Badge tone="danger">follow-up due</Badge>}
        {r.booking_id && (
          <span class="text-sm">
            Booking{" "}
            <Link class="font-medium text-brand-700 hover:underline" href={`/bookings/?id=${r.booking_id}`}>
              {r.booking_no ?? "—"}
            </Link>
          </span>
        )}
      </div>
      <div class="grid gap-4 xl:grid-cols-[24rem_minmax(0,1fr)]">
        <section class={[card, "self-start p-5"]}>
          <InfoList rows={rows} />
        </section>
        <section class={card}>
          {det.lines.length === 0 ? (
            <Empty>No items on this request.</Empty>
          ) : (
            <table class={table}>
              <thead>
                <tr>
                  <th class={th}>Item</th>
                  <th class={[th, "text-right"]}>Qty</th>
                  <th class={[th, "text-right"]}>Quoted</th>
                  <th class={[th, "text-right"]}>Amount</th>
                </tr>
              </thead>
              <tbody>
                {det.lines.map((l) => (
                  <tr key={l.id}>
                    <td class={td}>
                      {l.item_id ? (
                        <>
                          <Link class="text-brand-700 hover:underline" href={`/items/?id=${l.item_id}`}>
                            {l.item_code ?? "Deleted item"}
                          </Link>{" "}
                          {l.item_name}
                        </>
                      ) : (
                        <>
                          {l.description} <Badge>not in catalog</Badge>
                        </>
                      )}
                    </td>
                    <td class={[td, "text-right tabular-nums"]}>{formatQty(l.qty, l.unit)}</td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPesoOr(l.price_c)}</td>
                    <td class={[td, "text-right tabular-nums"]}>{formatPesoOr(l.amount_c)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {r.quoted_net_c > 0 && (
            <p class="border-t border-line px-3 py-2 text-right text-sm">
              Quoted total <span class="font-semibold tabular-nums">{formatPeso(r.quoted_net_c)}</span>{" "}
              <span class={hint}>{r.vat ? "before VAT" : "no VAT"}</span>
            </p>
          )}
        </section>
      </div>

      <BdFormModal open={editOpen} form={form} />
      <Confirm
        open={deleteOpen}
        title={`Delete request ${r.no ?? ""}?`}
        onConfirm$={$(async () => {
          unwrap(await commands.deleteBdRequest(id));
          await nav("/bd/");
        })}
      >
        <p>This deletes the request on every PC. Its number isn't given out again.</p>
      </Confirm>
      <Modal open={convertOpen} title="Convert to booking">
        <form
          class="space-y-3"
          preventdefault:submit
          onSubmit$={async () => {
            busy.value = true;
            convertError.value = "";
            try {
              const booking = unwrap(
                await commands.convertBdRequest(id, convertClient.value || null),
              );
              convertOpen.value = false;
              await nav(`/bookings/?id=${booking}`);
            } catch (e) {
              convertError.value = errorText(e);
            } finally {
              busy.value = false;
            }
          }}
        >
          {convertError.value && <Alert>{convertError.value}</Alert>}
          <p class="text-sm">
            Creates a draft booking with this request's catalog items and quoted prices, and marks
            the request won.
          </p>
          {!r.client_id && (
            <Field
              label="Client"
              hint={`${r.prospect_name ?? "The prospect"} must be a client first. Add them in Clients if needed.`}
            >
              <select class={input} required onChange$={(_, el) => (convertClient.value = el.value)}>
                <option value="">Choose a client…</option>
                {clients.value
                  .filter((c) => c.active)
                  .map((c) => (
                    <option key={c.id} value={c.id} selected={c.id === convertClient.value}>
                      {c.label}
                    </option>
                  ))}
              </select>
            </Field>
          )}
          {skipped > 0 && (
            <Alert tone="warning">
              {skipped} line(s) not in the catalog will be left out. Add the item to the catalog and
              edit the request first to include them.
            </Alert>
          )}
          <div class="flex justify-end gap-2">
            <button type="button" class={btnSecondary} onClick$={() => (convertOpen.value = false)}>
              Cancel
            </button>
            <button class={btnPrimary} disabled={busy.value}>
              Create booking
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
});

export default component$(() => {
  const session = useSession();
  const loc = useLocation();
  if (!can(session.user, "bd")) return <NoAccess />;
  const id = loc.url.searchParams.get("id");
  return id ? <BdView key={id} id={id} /> : <BdList />;
});

export const head: DocumentHead = { title: "Business Development · Sanco" };
