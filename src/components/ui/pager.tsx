import { component$, useComputed$, type ReadonlySignal } from "@builder.io/qwik";
import { useLocation, useNavigate } from "@builder.io/qwik-city";

/** Rows a list screen shows per page. */
export const PAGE_SIZE = 50;

/** Query-string key of the page number (1-based). A screen with more than
 *  one paged table gives the others their own key, e.g. `lots_page`. */
export const PAGE_KEY = "page";

/** True for every page key, so a filter change can send all lists back to
 *  their first page. */
export const isPageKey = (k: string) => k === PAGE_KEY || k.endsWith(`_${PAGE_KEY}`);

export interface PageView<T> {
  /** The rows on this page. */
  rows: T[];
  page: number;
  pages: number;
  total: number;
  /** 1-based positions of the first and last row shown. */
  from: number;
  to: number;
  key: string;
}

/**
 * One page of `rows`, at the page in the query string. Out-of-range pages
 * show the last one, so a list that shrank never shows an empty page.
 */
export function usePage<T>(
  rows: ReadonlySignal<T[]>,
  size = PAGE_SIZE,
  key = PAGE_KEY,
): ReadonlySignal<PageView<T>> {
  const loc = useLocation();
  return useComputed$(() => {
    const total = rows.value.length;
    const pages = Math.max(1, Math.ceil(total / size));
    const asked = Math.floor(Number(loc.url.searchParams.get(key))) || 1;
    const page = Math.min(Math.max(asked, 1), pages);
    const start = (page - 1) * size;
    return {
      rows: rows.value.slice(start, start + size),
      page,
      pages,
      total,
      from: total ? start + 1 : 0,
      to: Math.min(start + size, total),
      key,
    };
  });
}

const LEFT = "M15 6l-6 6 6 6";
const RIGHT = "M9 6l6 6-6 6";

/** The row under a paged table: page and range on the left, arrows on the right. */
export const Pager = component$<{ of: ReadonlySignal<PageView<unknown>> }>(({ of }) => {
  const loc = useLocation();
  const nav = useNavigate();
  const v = of.value;
  const key = v.key;
  if (v.total === 0) return null;
  const arrow =
    "inline-flex size-8 items-center justify-center rounded-md border border-line bg-white text-ink hover:bg-brand-50 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-white";
  return (
    <div class="flex items-center justify-between gap-3 border-t border-line px-4 py-2 text-sm">
      <span class="text-ink-muted tabular-nums">
        Page {v.page} of {v.pages} · {v.from}–{v.to} of {v.total}
      </span>
      <div class="flex items-center gap-1">
        {(
          [
            ["Previous page", LEFT, v.page - 1],
            ["Next page", RIGHT, v.page + 1],
          ] as const
        ).map(([label, d, to]) => (
          <button
            key={label}
            type="button"
            class={arrow}
            aria-label={label}
            title={label}
            disabled={to < 1 || to > v.pages}
            onClick$={async (_, el) => {
              const next = new URLSearchParams(loc.url.search);
              if (to > 1) next.set(key, String(to));
              else next.delete(key);
              const qs = next.toString();
              await nav(`${loc.url.pathname}${qs ? `?${qs}` : ""}`, {
                replaceState: true,
                scroll: false,
              });
              // Back to the top of the table when its top has scrolled away.
              const box = el.closest("section") ?? el.parentElement;
              if (box && box.getBoundingClientRect().top < 0) box.scrollIntoView({ block: "start" });
            }}
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="size-4"
              aria-hidden="true"
            >
              <path d={d} />
            </svg>
          </button>
        ))}
      </div>
    </div>
  );
});
