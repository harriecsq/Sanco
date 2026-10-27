import { component$ } from "@builder.io/qwik";
import { Link, useLocation } from "@builder.io/qwik-city";

export interface Tab {
  href: string;
  label: string;
  /** Shown after the label, e.g. how many rows the tab holds. */
  count?: number;
  /** Overrides the path match, for tabs kept in the query string. */
  active?: boolean;
}

/** Underlined tabs between a module's screens. The first tab is the
 *  module's index and matches only exactly; others match by prefix. */
export const TabNav = component$<{ tabs: Tab[]; label: string }>(
  ({ tabs, label }) => {
    const loc = useLocation();
    const path = loc.url.pathname;
    return (
      <nav
        class="mb-5 flex flex-wrap gap-1 border-b border-line"
        aria-label={label}
      >
        {tabs.map((t, i) => {
          const active =
            t.active ?? (i === 0 ? path === t.href : path.startsWith(t.href));
          return (
            <Link
              key={t.href}
              href={t.href}
              aria-current={active ? "page" : undefined}
              class={[
                "-mb-px border-b-2 px-3 py-2 text-sm",
                active
                  ? "border-brand-500 font-semibold text-brand-700"
                  : "border-transparent text-ink-muted hover:text-ink",
              ]}
            >
              {t.label}
              {t.count != null && (
                <span
                  class={[
                    "ml-1.5 rounded-full px-1.5 py-0.5 text-xs font-medium tabular-nums",
                    active ? "bg-brand-50 text-brand-700" : "bg-line text-ink-muted",
                  ]}
                >
                  {t.count}
                </span>
              )}
            </Link>
          );
        })}
      </nav>
    );
  },
);
