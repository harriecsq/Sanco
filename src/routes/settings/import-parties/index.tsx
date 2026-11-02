import { component$, useSignal } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { commands, type PartiesSummary } from "~/lib/bindings";
import { isAdmin } from "~/lib/perms";
import { errorText, unwrap } from "~/lib/result";
import { useSession } from "~/lib/session";
import { Alert, NoAccess, PageHeader } from "~/components/ui/ui";
import {
  btnPrimary,
  btnSecondary,
  card,
  hint,
  input,
} from "~/components/ui/styles";

/** Reads the chosen file; module-level so the handlers can use it. */
const readFile = (file: File | undefined) => file?.text() ?? Promise.resolve("");

export default component$(() => {
  const session = useSession();
  const json = useSignal("");
  const fileName = useSignal("");
  const summary = useSignal<PartiesSummary | null>(null);
  const done = useSignal(false);
  const busy = useSignal(false);
  const error = useSignal("");

  if (!isAdmin(session.user)) return <NoAccess />;

  return (
    <>
      <PageHeader
        title="Import clients and suppliers"
        subtitle="One-time: creates the code groups, clients and suppliers from a prepared import file."
      />
      <div class="max-w-2xl space-y-3">
        {error.value && <Alert>{error.value}</Alert>}
        {done.value && (
          <Alert tone="brand">
            Imported. The records are under Clients and Order → Suppliers.
          </Alert>
        )}
        <section class={[card, "space-y-3 p-5"]}>
          <p class={hint}>
            Only works while there are no clients or suppliers. A backup is made
            first. Run it on one PC only: the others get it by sync.
          </p>
          <input
            type="file"
            accept=".json,application/json"
            class={input}
            onChange$={async (_, el) => {
              error.value = "";
              summary.value = null;
              done.value = false;
              json.value = await readFile(el.files?.[0]);
              fileName.value = el.files?.[0]?.name ?? "";
            }}
          />
          <div class="flex gap-2">
            <button
              class={btnSecondary}
              disabled={!json.value || busy.value}
              onClick$={async () => {
                busy.value = true;
                error.value = "";
                try {
                  summary.value = unwrap(await commands.checkPartiesImport(json.value));
                } catch (e) {
                  summary.value = null;
                  error.value = errorText(e);
                } finally {
                  busy.value = false;
                }
              }}
            >
              Check file
            </button>
            <button
              class={btnPrimary}
              disabled={!summary.value || busy.value || done.value}
              onClick$={async () => {
                busy.value = true;
                error.value = "";
                try {
                  summary.value = unwrap(await commands.runPartiesImport(json.value));
                  done.value = true;
                } catch (e) {
                  error.value = errorText(e);
                } finally {
                  busy.value = false;
                }
              }}
            >
              {busy.value ? "Working…" : "Import"}
            </button>
          </div>
        </section>
        {summary.value && (
          <section class={[card, "p-5 text-sm"]}>
            <h2 class="mb-2 font-semibold">
              {done.value ? "Imported" : "Will import"} from {fileName.value}
            </h2>
            <p>
              {summary.value.clients} clients and {summary.value.suppliers} suppliers.
            </p>
            <ul class="mt-2 list-disc pl-5">
              {summary.value.groups.map(([name, n]) => (
                <li key={name}>
                  {name}: {n}
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </>
  );
});

export const head: DocumentHead = { title: "Import clients and suppliers · Sanco" };
