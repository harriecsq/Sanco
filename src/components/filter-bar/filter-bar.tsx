import { $, component$, Slot, useSignal } from "@builder.io/qwik";
import { useLocation, useNavigate } from "@builder.io/qwik-city";
import { btnGhost, inputBase } from "~/components/ui/styles";

export interface FilterOption {
  value: string;
  label: string;
}

export interface SelectFilter {
  /** Query-string key, e.g. `client`, `status`, `location`. */
  key: string;
  label: string;
  /** Text of the empty choice; defaults to "All <label>". */
  allLabel?: string;
  options: FilterOption[];
}

/** Keys the bar writes, besides the select keys. */
export const SEARCH_KEY = "q";
export const FROM_KEY = "from";
export const TO_KEY = "to";

/** Reads the current filters from the URL (`?q=…&from=…`). */
export function filtersOf(url: URL, keys: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const k of keys) out[k] = url.searchParams.get(k) ?? "";
  return out;
}

/**
 * The filter bar every list screen uses: text search, date range and
 * selects. Values live in the query string, so a filtered list can be
 * reloaded or linked to. Screens read them with `filtersOf(loc.url, …)`.
 */
export const FilterBar = component$<{
  search?: string;
  dates?: boolean;
  selects?: SelectFilter[];
}>(({ search, dates, selects = [] }) => {
  const loc = useLocation();
  const nav = useNavigate();
  const timer = useSignal<number>();
  const params = loc.url.searchParams;

  const set = $((key: string, value: string) => {
    const next = new URLSearchParams(loc.url.search);
    if (value) next.set(key, value);
    else next.delete(key);
    const qs = next.toString();
    nav(`${loc.url.pathname}${qs ? `?${qs}` : ""}`, {
      replaceState: true,
      scroll: false,
    });
  });

  const keys = [SEARCH_KEY, FROM_KEY, TO_KEY, ...selects.map((s) => s.key)];
  const active = keys.some((k) => params.get(k));

  return (
    <div class="mb-4 flex flex-wrap items-end gap-2">
      {search !== undefined && (
        <input
          type="search"
          class={[inputBase, "w-64"]}
          placeholder={search || "Search"}
          aria-label="Search"
          value={params.get(SEARCH_KEY) ?? ""}
          onInput$={(_, el) => {
            clearTimeout(timer.value);
            const v = el.value.trim();
            timer.value = window.setTimeout(() => set(SEARCH_KEY, v), 300);
          }}
        />
      )}
      {dates && (
        <div class="flex items-center gap-1 text-sm text-ink-muted">
          <input
            type="date"
            class={[inputBase, "w-38"]}
            aria-label="From date"
            value={params.get(FROM_KEY) ?? ""}
            onChange$={(_, el) => set(FROM_KEY, el.value)}
          />
          <span>to</span>
          <input
            type="date"
            class={[inputBase, "w-38"]}
            aria-label="To date"
            value={params.get(TO_KEY) ?? ""}
            onChange$={(_, el) => set(TO_KEY, el.value)}
          />
        </div>
      )}
      {selects.map((s) => (
        <select
          key={s.key}
          class={[inputBase, "w-auto min-w-36"]}
          aria-label={s.label}
          value={params.get(s.key) ?? ""}
          onChange$={(_, el) => set(s.key, el.value)}
        >
          <option value="">
            {s.allLabel ?? `All ${s.label.toLowerCase()}`}
          </option>
          {s.options.map((o) => (
            <option
              key={o.value}
              value={o.value}
              selected={params.get(s.key) === o.value}
            >
              {o.label}
            </option>
          ))}
        </select>
      ))}
      {active && (
        <button
          type="button"
          class={btnGhost}
          onClick$={() => {
            // Keeps other params, e.g. a detail page's `?id=…&tab=…`.
            const next = new URLSearchParams(loc.url.search);
            for (const k of keys) next.delete(k);
            const qs = next.toString();
            nav(`${loc.url.pathname}${qs ? `?${qs}` : ""}`, {
              replaceState: true,
              scroll: false,
            });
          }}
        >
          Clear filters
        </button>
      )}
      <div class="ml-auto flex items-center gap-2">
        <Slot />
      </div>
    </div>
  );
});
