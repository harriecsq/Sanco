import { $, component$, useSignal, type QRL } from "@builder.io/qwik";
import { commands, type BankAccountInfo } from "~/lib/bindings";
import { useLiveLoad } from "~/lib/events";
import { unwrap } from "~/lib/result";
import { input } from "~/components/ui/styles";

/** "BDO Savings · 0012-3456-78" */
export const bankAccountLabel = (a: Pick<BankAccountInfo, "name" | "account_no" | "currency">) =>
  [a.name, a.account_no, a.currency !== "PHP" ? a.currency : null].filter(Boolean).join(" · ");

/**
 * Picks one of the company's bank accounts: active ones, plus `value` if it
 * was made inactive since.
 */
export const BankAccountSelect = component$<{
  value: string;
  onPick$: QRL<(id: string) => void>;
  /** Shown for "no account". */
  none?: string;
  ariaLabel?: string;
}>((props) => {
  const accounts = useSignal<BankAccountInfo[]>([]);
  const loaded = useSignal(false);

  useLiveLoad(
    ["bank_accounts"],
    $(async () => {
      try {
        accounts.value = unwrap(await commands.listBankAccounts());
      } catch {
        accounts.value = [];
      }
      loaded.value = true;
    }),
  );

  const shown = accounts.value.filter((a) => a.active || a.id === props.value);
  return (
    <>
      <select
        class={input}
        aria-label={props.ariaLabel}
        onChange$={(_, el) => props.onPick$(el.value)}
      >
        <option value="" selected={!props.value}>
          {props.none ?? "Not recorded"}
        </option>
        {shown.map((a) => (
          <option key={a.id} value={a.id} selected={a.id === props.value}>
            {`${bankAccountLabel(a)}${a.active ? "" : " (inactive)"}`}
          </option>
        ))}
      </select>
      {loaded.value && accounts.value.length === 0 && (
        <p class="mt-1 text-xs text-ink-muted">
          No bank accounts yet. Add them under Accounting →{" "}
          <a class="text-brand-700 hover:underline" href="/accounting/bank-accounts/">
            Bank accounts
          </a>
          .
        </p>
      )}
    </>
  );
});
